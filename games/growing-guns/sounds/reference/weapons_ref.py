"""Growing Guns weapon sounds, transliterated 1:1 from sfx.gd (GDScript) at 44.1 kHz.

Each function mirrors one Godot synth: the same jitter draws (in the same
order), the same per-sample loop, the same clamps. `rng` stands in for the
seeded RandomNumberGenerator (or the global randf for the unseeded synths);
only the noise realisation differs from Godot. Line numbers refer to
growing-guns/scripts/sfx.gd. Output is mono (Godot writes L == R, except the
ion burst's R = 0.99 L).
"""
import math

import numpy as np

SR = 44100
TAU = math.tau
PI = math.pi


def lerp(a, b, x):
    return a + (b - a) * x


def clamp(x, lo, hi):
    return min(max(x, lo), hi)


class Rng:
    """randf / randf_range / randi_range on a seeded numpy generator."""

    def __init__(self, seed):
        self.g = np.random.default_rng(seed)

    def randf(self):
        return float(self.g.random())

    def randf_range(self, a, b):
        return a + (b - a) * self.randf()

    def randi_range(self, a, b):
        return int(self.g.integers(a, b + 1))


# 2.2  _synth_shot 1202-1299
def shot(rng, dmg_ratio=1.0, accuracy=0.87, velocity=1.0):
    dmg_ratio = max(1.0, dmg_ratio)
    accuracy = clamp(accuracy, 0.0, 1.0)
    velocity = clamp(velocity, 0.5, 3.0)
    bright_jit = lerp(0.85, 1.15, rng.randf())
    click_freq_jit = lerp(0.80, 1.25, rng.randf())
    thwack_decay = lerp(180.0, 280.0, rng.randf()) * math.sqrt(velocity)
    thwack_drive = lerp(4.0, 8.0, rng.randf()) * lerp(0.9, 1.3, clamp((velocity - 1.0) / 2.0, 0.0, 1.0))
    dur_jit = lerp(0.90, 1.12, rng.randf())
    kick_f0 = 220.0 * lerp(0.88, 1.12, rng.randf())
    kick_f1 = clamp(55.0 / math.sqrt(dmg_ratio), 28.0, 80.0) * lerp(0.92, 1.10, rng.randf())
    kick_sweep_rate = lerp(70.0, 95.0, rng.randf())
    kick_decay = clamp(35.0 / math.sqrt(dmg_ratio), 10.0, 40.0)
    kick_gain = lerp(0.55, 1.10, clamp((dmg_ratio - 1.0) / 3.0, 0.0, 1.0))
    dur = clamp((0.08 + 0.08 * math.log(dmg_ratio) / math.log(2.0)) * dur_jit / math.sqrt(velocity), 0.04, 0.35)
    n = int(dur * SR)
    out = np.zeros(n)
    noise_brightness = clamp(lerp(0.18, 0.55, accuracy) * bright_jit, 0.05, 0.95)
    noise_decay = lerp(8.0, 16.0, rng.randf()) * math.sqrt(velocity)
    click_gain = accuracy * 0.3
    click_freq = 2400.0 * click_freq_jit
    lp = thwack_low_acc = kick_phase = kick_lp = 0.0
    hp_cutoff = clamp(220.0 / pow(dmg_ratio, 1.4), 25.0, 220.0)
    hp_k = clamp(TAU * hp_cutoff / SR, 0.005, 0.5)
    bass_acc = 0.0
    for i in range(n):
        t = i / SR
        env = pow(clamp(1.0 - t / dur, 0.0, 1.0), 1.6)
        noise = rng.randf_range(-1.0, 1.0)
        raw_thwack = rng.randf_range(-1.0, 1.0)
        thwack_low_acc = lerp(thwack_low_acc, raw_thwack, 0.06)
        hp = raw_thwack - thwack_low_acc
        thwack = math.tanh(hp * thwack_drive) * 1.5 * math.exp(-t * thwack_decay)
        kick_freq = kick_f1 + (kick_f0 - kick_f1) * math.exp(-t * kick_sweep_rate)
        kick_phase += 2.0 * PI * kick_freq / SR
        kick_lp = lerp(kick_lp, math.sin(kick_phase), 0.30)
        kick = kick_lp * math.exp(-t * kick_decay)
        lp = lerp(lp, noise, noise_brightness)
        lp_env = math.exp(-t * noise_decay)
        click = math.sin(2.0 * PI * click_freq * t) * math.exp(-t * 280.0) * click_gain
        s = math.tanh((thwack + lp * 0.5 * lp_env + kick * kick_gain + click) * env * 1.4)
        bass_acc = lerp(bass_acc, s, hp_k)
        s -= bass_acc
        out[i] = s
    return out


# 3.1  _synth_grenade_launch 1301-1359
def grenade_launch(rng, variant=None):
    if variant is None:
        variant = rng.randi_range(0, 4)  # the game's randi() % 5
    dur = 0.18
    n = int(dur * SR)
    out = np.zeros(n)
    pitch_offsets = [-5.0, -2.0, 0.0, 2.0, 4.0]
    body_hz = 160.0 + pitch_offsets[clamp(variant, 0, 4)]
    noise_lp_hi = noise_lp_lo = tick_lp_lo = body_phase = 0.0
    for i in range(n):
        t = i / SR
        sweep_phase = clamp(t / 0.065, 0.0, 1.0)
        sweep_t = sweep_phase * sweep_phase * (3.0 - 2.0 * sweep_phase)
        center_hz = lerp(650.0, 4300.0, sweep_t)
        lp_hi_k = clamp(center_hz * TAU * 1.8 / SR, 0.04, 0.92)
        lp_lo_k = clamp(center_hz * TAU * 0.35 / SR, 0.004, 0.45)
        gas_env = math.sin(sweep_phase * PI) * math.exp(-max(0.0, t - 0.06) * 55.0)
        noise = rng.randf_range(-1.0, 1.0)
        noise_lp_hi = lerp(noise_lp_hi, noise, lp_hi_k)
        noise_lp_lo = lerp(noise_lp_lo, noise_lp_hi, lp_lo_k)
        gas = (noise_lp_hi - noise_lp_lo) * gas_env * 0.55
        tick_t = t - 0.038
        tick_env = math.exp(-tick_t * 420.0) if tick_t >= 0.0 else 0.0
        tick_noise = rng.randf_range(-1.0, 1.0) * tick_env
        tick_lp_lo = lerp(tick_lp_lo, tick_noise, 0.035)
        tick = (tick_noise - tick_lp_lo) * 0.75
        body_t = max(0.0, t - 0.026)
        body_phase_norm = clamp(body_t / max(0.001, dur - 0.026), 0.0, 1.0)
        body_attack = clamp(body_t / 0.012, 0.0, 1.0)
        body_attack = body_attack * body_attack * (3.0 - 2.0 * body_attack)
        body_release = 1.0 - body_phase_norm
        body_release = body_release * body_release * (3.0 - 2.0 * body_release)
        body_env = body_attack * body_release
        body_phase += TAU * body_hz / SR
        body = (math.sin(body_phase) * 0.86 + math.sin(body_phase * 2.0) * 0.018
                + math.sin(body_phase * 3.0) * 0.006) * body_env
        sub = math.sin(body_phase * 0.5) * body_env * 0.06
        out[i] = math.tanh((gas + tick + body + sub) * 1.35) * 0.72
    return out


# 3.2  _synth_next_round 1361-1388 (global randf)
def next_round(rng):
    dur = 0.07
    n = int(dur * SR)
    out = np.zeros(n)
    lp_low = lp_high = 0.0
    t_back, t_fwd = 0.0, 0.022
    for i in range(n):
        t = i / SR
        env_back = math.exp(-(t - t_back) * 260.0) if t >= t_back else 0.0
        env_fwd = math.exp(-(t - t_fwd) * 200.0) * 0.75 if t >= t_fwd else 0.0
        noise = rng.randf_range(-1.0, 1.0)
        burst = noise * (env_back + env_fwd)
        lp_low = lerp(lp_low, burst, 0.04)
        hp = burst - lp_low
        lp_high = lerp(lp_high, hp, 0.48)
        out[i] = lp_high * 0.85
    return out


# 3.3  _synth_empty_chamber 1390-1408 (global randf)
def empty_chamber(rng):
    dur = 0.025
    n = int(dur * SR)
    out = np.zeros(n)
    lp_low = lp_high = 0.0
    for i in range(n):
        t = i / SR
        env = math.exp(-t * 280.0)
        burst = rng.randf_range(-1.0, 1.0) * env
        lp_low = lerp(lp_low, burst, 0.04)
        hp = burst - lp_low
        lp_high = lerp(lp_high, hp, 0.42)
        out[i] = lp_high * 0.9
    return out


# 3.4  _synth_reload / _reload_cluster / _reload_tap 1603-1671 (global randf)
def reload(rng, duration=1.2):
    duration = round(clamp(duration, 0.2, 6.0) / 0.05) * 0.05  # the caller's snappedf bucket
    n = int(duration * SR)
    out = np.zeros(n)

    def tap(start_time, pitch_ratio, amp):
        tap_dur = 0.025
        start_idx = max(0, int(start_time * SR))
        if start_idx >= n:
            return
        tap_n = int(tap_dur * SR)
        center = 1700.0 * pitch_ratio
        fc_hi = center * 1.5
        fc_lo = max(40.0, center * 0.6)
        k_hi = 1.0 - math.exp(-TAU * fc_hi / SR)
        k_lo = 1.0 - math.exp(-TAU * fc_lo / SR)
        lp_hi = lp_lo = 0.0
        for i in range(tap_n):
            idx = start_idx + i
            if idx >= n:
                break
            t = i / SR
            env = math.exp(-t * 280.0)
            noise = rng.randf_range(-1.0, 1.0)
            lp_hi += k_hi * (noise - lp_hi)
            lp_lo += k_lo * (noise - lp_lo)
            out[idx] += (lp_hi - lp_lo) * env * amp

    def cluster(t_start, t_end, pitch_begin, pitch_end):
        span = max(t_end - t_start, 0.02)
        num_taps = rng.randi_range(6, 10)
        for tap_i in range(num_taps):
            frac = (float(tap_i) + rng.randf_range(-0.15, 0.15)) / float(num_taps)
            frac = clamp(frac, 0.0, 1.0)
            t_tap = t_start + span * frac
            pitch_ratio = lerp(pitch_begin, pitch_end, frac) * rng.randf_range(0.95, 1.05)
            amp = rng.randf_range(0.3, 0.5)
            tap(t_tap, pitch_ratio, amp)

    cluster(duration * 0.00, duration * 0.12, 2.2, 0.4)
    cluster(duration * 0.88, duration * 0.98, 0.4, 2.0)
    return np.tanh(out)


# 3.5  _synth_melee 1586-1601 (global randf)
def melee(rng, pitch_ratio=0.71):
    dur = 0.20
    n = int(dur * SR)
    out = np.zeros(n)
    lp = 0.0
    for i in range(n):
        t = i / SR
        env = math.sin(PI * clamp(t / dur, 0.0, 1.0))
        noise = rng.randf_range(-1.0, 1.0)
        cutoff_k = lerp(0.7, 0.18, t / dur) * pitch_ratio
        lp = lerp(lp, noise, clamp(cutoff_k, 0.01, 0.99))
        out[i] = lp * env * 0.45
    return out


# 3.6  _synth_casing_drop 1777-1806
def casing_drop(rng):
    dur = 0.18
    n = int(dur * SR)
    out = np.zeros(n)
    f_low = lerp(1900.0, 2150.0, rng.randf())
    f_high = lerp(9600.0, 10500.0, rng.randf())
    d_low = lerp(95.0, 115.0, rng.randf())
    d_high = lerp(24.0, 32.0, rng.randf())
    phase_low = phase_high = 0.0
    for i in range(n):
        t = i / SR
        phase_low += TAU * f_low / SR
        phase_high += TAU * f_high / SR
        p_low = math.sin(phase_low) * math.exp(-t * d_low) * 0.45
        p_high = math.sin(phase_high) * math.exp(-t * d_high) * 0.55
        out[i] = math.tanh((p_low + p_high) * 1.1)
    return out


# 3.7  _synth_bullet_zip 1673-1733 (global randf)
def bullet_zip(rng, speed_factor=1.0, scale_factor=1.0):
    speed_factor = clamp(speed_factor, 0.2, 5.0)
    scale_factor = clamp(scale_factor, 0.5, 4.0)
    pitch_jit = rng.randf_range(0.92, 1.08)
    dur_jit = rng.randf_range(0.85, 1.18)
    bright_jit = rng.randf_range(0.85, 1.15)
    nominal = clamp(2400.0 + 1300.0 * speed_factor, 300.0, 6000.0) * pitch_jit
    v_bullet = clamp(speed_factor * 165.0, 10.0, 280.0)
    c_sound = 343.0
    base_high = min(nominal * c_sound / (c_sound - v_bullet), 7000.0)
    base_low = nominal * c_sound / (c_sound + v_bullet)
    dur = clamp(0.06 + 0.06 / speed_factor, 0.07, 0.40) * dur_jit
    n = int(dur * SR)
    out = np.zeros(n)
    attack_frac = clamp(0.45 / speed_factor, 0.04, 0.5)
    drive = lerp(1.0, 3.5, clamp((speed_factor - 3.0) / 2.0, 0.0, 1.0))
    lp_hi = lp_lo = 0.0
    for i in range(n):
        t = i / SR
        phase = clamp(t / dur, 0.0, 1.0)
        sweep_t = phase * phase * phase * (phase * (phase * 6.0 - 15.0) + 10.0)
        center = lerp(base_high, base_low, sweep_t) * bright_jit
        lp_k = clamp(center * (TAU * 2.0) / SR, 0.04, 0.98)
        hp_k = clamp(center * (TAU * 0.5) / SR, 0.003, 0.6)
        if phase < attack_frac:
            env = phase / attack_frac
        else:
            d = (phase - attack_frac) / max(1.0 - attack_frac, 0.0001)
            env = math.exp(-d * 4.0)
        noise = rng.randf_range(-1.0, 1.0)
        lp_hi = lerp(lp_hi, noise, lp_k)
        lp_lo = lerp(lp_lo, lp_hi, hp_k)
        bp = lp_hi - lp_lo
        out[i] = math.tanh(bp * env * 0.55 * scale_factor * drive)
    return out


# 4  _synth_impact 1738-1775
def impact(rng, dmg_ratio=1.0):
    dmg_ratio = clamp(dmg_ratio, 0.5, 6.0)  # impact() clamps before the synth
    click_freq = lerp(2200.0, 4500.0, rng.randf())
    click_decay = lerp(180.0, 320.0, rng.randf())
    dmg_norm = clamp((dmg_ratio - 1.0) / 4.0, 0.0, 1.0)
    thump_f0 = lerp(160.0, 260.0, rng.randf())
    thump_f1 = lerp(45.0, 80.0, rng.randf()) * lerp(1.0, 0.7, dmg_norm)
    thump_sweep = lerp(60.0, 100.0, rng.randf())
    thump_decay = lerp(28.0, 42.0, rng.randf())
    thump_gain = lerp(0.0, 0.65, dmg_norm)
    dur = lerp(0.030, 0.080, dmg_norm) * lerp(0.92, 1.10, rng.randf())
    n = int(dur * SR)
    out = np.zeros(n)
    lp_acc = thump_phase = 0.0
    for i in range(n):
        t = i / SR
        noise = rng.randf_range(-1.0, 1.0)
        hp_k = clamp(click_freq * (TAU * 0.4) / SR, 0.05, 0.6)
        lp_acc = lerp(lp_acc, noise, hp_k)
        click = (noise - lp_acc) * math.exp(-t * click_decay)
        thump_freq = thump_f1 + (thump_f0 - thump_f1) * math.exp(-t * thump_sweep)
        thump_phase += TAU * thump_freq / SR
        thump = math.sin(thump_phase) * math.exp(-t * thump_decay) * thump_gain
        out[i] = math.tanh((click + thump) * 1.4)
    return out


# 5.1  _synth_explosion 1410-1486
def explosion_bang(rng, radius=6.0):
    bright_jit = lerp(0.85, 1.15, rng.randf())
    thwack_decay = lerp(180.0, 280.0, rng.randf())
    thwack_drive = lerp(4.0, 8.0, rng.randf())
    dur_jit = lerp(0.92, 1.10, rng.randf())
    r_norm = clamp(radius / 6.0, 0.3, 4.0)
    kick_f0 = 220.0 * lerp(0.88, 1.12, rng.randf())
    kick_f1 = clamp(55.0 / math.sqrt(r_norm), 28.0, 80.0) * lerp(0.92, 1.10, rng.randf())
    kick_sweep_rate = lerp(70.0, 95.0, rng.randf())
    kick_decay = clamp(35.0 / math.sqrt(r_norm), 10.0, 40.0)
    kick_gain = lerp(0.55, 0.95, clamp((r_norm - 1.0) / 3.0, 0.0, 1.0))
    sub_cutoff_k = lerp(0.005, 0.010, rng.randf())
    sub_decay = clamp(3.5 / math.sqrt(r_norm), 1.2, 4.0)
    dur = clamp((0.4 + radius * 0.08) * dur_jit, 0.4, 1.6)
    n = int(dur * SR)
    out = np.zeros(n)
    thwack_cutoff_k = lerp(0.10, 0.16, rng.randf())
    noise_brightness = clamp(0.45 * bright_jit, 0.05, 0.95)
    noise_decay = lerp(8.0, 15.0, rng.randf())
    lp = thwack_lp = sub_lp = kick_phase = kick_lp = 0.0
    for i in range(n):
        t = i / SR
        env = math.exp(-t * 1.4) * pow(1.0 - t / dur, 0.6)
        noise = rng.randf_range(-1.0, 1.0)
        raw_thwack = rng.randf_range(-1.0, 1.0)
        thwack_lp = lerp(thwack_lp, raw_thwack, thwack_cutoff_k)
        thwack = math.tanh(thwack_lp * thwack_drive) * 1.5 * math.exp(-t * thwack_decay)
        lp = lerp(lp, noise, noise_brightness)
        lp_env = math.exp(-t * noise_decay)
        kick_freq = kick_f1 + (kick_f0 - kick_f1) * math.exp(-t * kick_sweep_rate)
        kick_phase += 2.0 * PI * kick_freq / SR
        kick_lp = lerp(kick_lp, math.sin(kick_phase), 0.30)
        kick = kick_lp * math.exp(-t * kick_decay)
        sub_lp = lerp(sub_lp, rng.randf_range(-1.0, 1.0), sub_cutoff_k)
        sub = sub_lp * 3.5 * math.exp(-t * sub_decay)
        out[i] = math.tanh((thwack + lp * 0.5 * lp_env + kick * kick_gain + sub * 0.7) * env * 1.4)
    return out


# 5.2  _synth_explosion_rumble 1491-1538 (also the heavy-gun shot rumble)
def explosion_rumble(rng, radius=6.0):
    r_norm = clamp(radius, 2.0, 24.0) / 24.0
    dur = clamp(0.3 + r_norm * r_norm * 4.7, 0.3, 5.0)
    n = int(dur * SR)
    out = np.zeros(n)
    cutoff_base = clamp(220.0 - radius * 7.0, 55.0, 200.0)
    cutoff = cutoff_base + rng.randf_range(-15.0, 15.0)
    lp_k = clamp(TAU * cutoff / SR, 0.005, 0.4)
    brown = lp = 0.0
    attack_dur = lerp(0.05, 0.12, rng.randf())
    ease_power = lerp(2.4, 3.0, rng.randf())
    gain = 1.5 * math.sqrt(6.0 / max(6.0, radius))
    body_dur = max(dur - attack_dur, 0.0001)
    for i in range(n):
        t = i / SR
        attack = clamp(t / attack_dur, 0.0, 1.0)
        decay_phase = clamp((t - attack_dur) / body_dur, 0.0, 1.0)
        decay = pow(1.0 - decay_phase, ease_power)
        env = attack * decay
        brown = brown * 0.998 + rng.randf_range(-1.0, 1.0) * 0.03
        lp = lerp(lp, brown, lp_k)
        out[i] = math.tanh(lp * env * gain)
    return out


# 5.3  _synth_ion_cannon_burst 1972-2008 (left channel)
def ion_burst(rng, radius=38.0):
    r_norm = clamp(radius / 38.0, 0.6, 1.4)
    dur = clamp(lerp(0.55, 0.85, r_norm) * lerp(0.95, 1.05, rng.randf()), 0.5, 0.95)
    n = int(dur * SR)
    out = np.zeros(n)
    f0 = 40.0 * lerp(0.97, 1.03, rng.randf()) * lerp(1.0, 0.9, r_norm - 0.6)
    f1 = f0 * 2.0 * lerp(0.992, 1.008, rng.randf())
    ph0 = rng.randf() * TAU
    ph1 = rng.randf() * TAU
    lp = crack_lp = 0.0
    for i in range(n):
        t = i / SR
        u = t / dur
        ph0 = math.fmod(ph0 + TAU * f0 / SR, TAU)
        ph1 = math.fmod(ph1 + TAU * f1 / SR, TAU)
        saw = (ph0 / PI - 1.0) * 0.85 + (ph1 / PI - 1.0) * 0.3
        lp = lerp(lp, saw, 0.45)
        crack = rng.randf_range(-1.0, 1.0)
        crack_lp = lerp(crack_lp, crack, 0.5)
        crack_env = math.exp(-t * 85.0)
        env = math.exp(-t * lerp(4.6, 3.0, r_norm)) * pow(1.0 - u, 0.5)
        out[i] = math.tanh((lp * 2.4 + crack_lp * crack_env * 1.1) * env) * 0.7
    return out


SOUNDS = {
    "shot": shot,
    "grenade_launch": grenade_launch,
    "next_round": next_round,
    "empty_chamber": empty_chamber,
    "reload": reload,
    "melee": melee,
    "casing_drop": casing_drop,
    "bullet_zip": bullet_zip,
    "impact": impact,
    "explosion_bang": explosion_bang,
    "explosion_rumble": explosion_rumble,
    "ion_burst": ion_burst,
}
