"""Growing Guns strike, movement and UI sounds, transliterated 1:1 from sfx.gd (GDScript) at 44.1 kHz.

Covers spec sections 6 (air strike inbound, ion cannon charge, rocket descent,
mine plant), 7 (dash, lava sizzle) and 8 (hit_received, kill_confirm,
hitmarker, pick countdown beep, pling, card_flip, pickup_drop, enemy_incoming).

Each function mirrors one Godot synth: the same jitter draws in the same order,
the same per-sample loop, the same clamps. `rng` stands in for the seeded
RandomNumberGenerator (lava sizzle, card flip) or Godot's global randf (the
rest); only the noise realisation differs from Godot. Line numbers refer to
growing-guns/scripts/sfx.gd. Output is mono (Godot writes L == R, except the
ion charge's R = 0.98 L). Callers' cache buckets (snappedf) are applied where
the caller applies them.
"""
import math

import numpy as np

from weapons_ref import Rng, clamp, lerp  # noqa: F401  (Rng re-exported for the runner)

SR = 44100
TAU = math.tau
PI = math.pi


def smoothstep(a, b, x):
    """Godot 4 smoothstep(from, to, x)."""
    if a == b:
        return a
    s = clamp((x - a) / (b - a), 0.0, 1.0)
    return s * s * (3.0 - 2.0 * s)


def snapped(x, step):
    return math.floor(x / step + 0.5) * step


# 6.1  attach_air_strike_inbound 1068-1084, _synth_air_strike_inbound 1907-1932 (global randf)
def air_strike_inbound(rng, travel_seconds=2.3, distant=False):
    travel_seconds = snapped(clamp(travel_seconds, 1.75, 6.5), 0.25)  # the caller's bucket
    dur = clamp(travel_seconds, 1.75, 6.5)
    n = int(dur * SR)
    out = np.zeros(n)
    sample_dt = 1.0 / SR
    lp = lp2 = brown = 0.0
    start_env = 0.52 if distant else 0.46
    start_cutoff = 45.0 if distant else 60.0
    for i in range(n):
        t = i / SR
        u = t / dur
        env = lerp(start_env, 1.0, u * u)
        cutoff = lerp(start_cutoff, 320.0, u)
        rc = 1.0 / (TAU * cutoff)
        alpha = sample_dt / (rc + sample_dt)
        brown = brown * 0.998 + rng.randf_range(-1.0, 1.0) * 0.04
        lp = lp + alpha * (brown - lp)
        lp2 = lp2 + alpha * (lp - lp2)
        out[i] = math.tanh(lp2 * 6.5) * 0.62 * env
    return out


# 6.2  attach_ion_cannon_charge 1092-1112, _synth_ion_cannon_charge 1935-1969 (global randf)
# The six partials are independent of the noise, so they are computed as
# arrays; phases accumulate sample by sample (np.cumsum) exactly as the loop does.
def ion_cannon_charge(rng, charge_seconds=3.8):
    charge_seconds = snapped(clamp(charge_seconds, 2.5, 6.5), 0.1)  # the caller's bucket
    dur = clamp(charge_seconds, 2.5, 6.5)
    n = int(dur * SR)
    freqs = [220.0, 261.63, 329.63, 392.0, 440.0, 554.37]
    gains = [0.34, 0.28, 0.24, 0.20, 0.18, 0.12]
    detune = [1.0, 1.002, 0.998, 1.004, 0.996, 1.001]
    phases0 = [rng.randf() * TAU for _ in freqs]
    t = np.arange(n) / SR
    u = t / dur
    swell = u ** 3.4
    s_ = np.clip((u - 0.42) / (1.0 - 0.42), 0.0, 1.0)
    bloom = s_ * s_ * (3.0 - 2.0 * s_)
    pulse = 1.0 + np.sin(t * 8.6) * 0.035 * bloom
    chord = np.zeros(n)
    for f, g, d, p0 in zip(freqs, gains, detune, phases0):
        hz = f * d
        ph = p0 + np.cumsum(np.full(n, TAU * hz / SR))  # phases[j] += ... before reading
        chord += np.sin(ph) * g
        chord += np.sin(ph * 2.0) * g * 0.07 * bloom
        chord += np.sin(ph * 3.0) * g * 0.03 * bloom * bloom
    noise_lp = np.zeros(n)
    y = 0.0
    for i in range(n):
        y = lerp(y, rng.randf_range(-1.0, 1.0), 0.014)
        noise_lp[i] = y
    bed = noise_lp * (0.05 + (0.18 - 0.05) * swell)
    env = 0.10 + (1.0 - 0.10) * swell
    return np.tanh((chord * 0.22 + bed) * env * pulse) * 0.68


# 6.3  rocket_descent 1061-1065, _synth_rocket_descent 2011-2041 (global randf)
def rocket_descent(rng):
    dur = 0.85
    n = int(dur * SR)
    out = np.zeros(n)
    sample_dt = 1.0 / SR
    lp = lp2 = 0.0
    for i in range(n):
        t = i / SR
        u = t / dur
        env = lerp(0.45, 1.0, u * u)
        cutoff = lerp(70.0, 280.0, u)
        rc = 1.0 / (TAU * cutoff)
        alpha = sample_dt / (rc + sample_dt)
        noise = rng.randf_range(-1.0, 1.0)
        lp = lp + alpha * (noise - lp)
        lp2 = lp2 + alpha * (lp - lp2)
        out[i] = math.tanh(lp2 * 5.5) * 0.55 * env
    return out


# 6.4  mine_plant 1130-1133, _synth_mine_plant 2044-2063 (deterministic)
def mine_plant(rng):
    dur = 0.35
    n = int(dur * SR)
    out = np.zeros(n)
    for i in range(n):
        t = i / SR
        thud = math.sin(2.0 * PI * 80.0 * t) * math.exp(-t * 35.0) * 0.6
        beep_env = 0.0
        if t > 0.10:
            bt = t - 0.10
            beep_env = math.exp(-bt * 16.0) * (1.0 - math.exp(-bt * 50.0))
        beep = math.sin(2.0 * PI * 1200.0 * t) * beep_env * 0.35
        out[i] = math.tanh((thud + beep) * 1.2) * 0.55
    return out


# 7  dash 1039-1040, _synth_dash 1808-1827 (global randf)
def dash(rng):
    dur = 0.16
    n = int(dur * SR)
    out = np.zeros(n)
    lp = 0.0
    for i in range(n):
        t = i / SR
        env_phase = clamp(t / dur, 0.0, 1.0)
        env = math.sin(PI * env_phase) ** 1.6
        noise = rng.randf_range(-1.0, 1.0)
        cutoff_k = lerp(0.08, 0.22, t / dur)
        lp = lerp(lp, noise, cutoff_k)
        out[i] = lp * env * 0.18
    return out


# 7  lava_sizzle 1053-1060, _synth_lava_sizzle 1830-1861 (seeded per variant and mode)
def lava_sizzle(rng, fall_death=False):
    dur = lerp(1.55, 1.95, rng.randf()) if fall_death else lerp(0.42, 0.58, rng.randf())
    n = int(dur * SR)
    out = np.zeros(n)
    hiss_lp = low_lp = 0.0
    bubble_phase = rng.randf() * TAU
    for i in range(n):
        t = i / SR
        u = t / dur
        attack_time = 0.42 if fall_death else 0.075
        attack = smoothstep(0.0, attack_time, t)
        fade_start = 0.38
        fade_u = clamp((u - fade_start) / (1.0 - fade_start), 0.0, 1.0)
        fall_tail = math.cos(fade_u * PI * 0.5) ** 1.65 if math.cos(fade_u * PI * 0.5) > 0 else 0.0
        fall_body = smoothstep(0.0, 0.18, u) * fall_tail
        body = fall_body if fall_death else math.exp(-u * 4.2)
        env = attack * body
        noise = rng.randf_range(-1.0, 1.0)
        hiss_lp = lerp(hiss_lp, noise, 0.08)
        hiss = (noise - hiss_lp) * env * (0.11 if fall_death else 0.26)
        low_noise = rng.randf_range(-1.0, 1.0)
        low_lp = lerp(low_lp, low_noise, 0.018)
        bubble_phase += TAU * lerp(22.0, 38.0, rng.randf()) / SR
        bubble_env = body * (0.55 + 0.45 * math.sin(bubble_phase))
        bubble = math.tanh(low_lp * 4.0) * bubble_env * 0.18
        out[i] = math.tanh((hiss + bubble) * (1.05 if fall_death else 1.35))
    return out


# 8  hit_received 1136-1143, _synth_hit_received 1540-1554 (global randf)
def hit_received(rng):
    dur = 0.18
    n = int(dur * SR)
    out = np.zeros(n)
    for i in range(n):
        t = i / SR
        env = (1.0 - t / dur) ** 2.0
        pitch = lerp(160.0, 70.0, clamp(t / 0.08, 0.0, 1.0))
        tone = math.sin(2.0 * PI * pitch * t)
        noise = rng.randf_range(-1.0, 1.0) * math.exp(-t * 80.0)
        out[i] = (tone * 0.8 + noise * 0.4) * env * 0.75
    return out


# 8  kill_confirm 1144-1146, _synth_kill_confirm 1572-1584 (deterministic)
def kill_confirm(rng):
    dur = 0.28
    n = int(dur * SR)
    out = np.zeros(n)
    for i in range(n):
        t = i / SR
        env = math.exp(-t * 9.0)
        tone = math.sin(2.0 * PI * 392.0 * t) * 0.45 + math.sin(2.0 * PI * 587.0 * t) * 0.3
        out[i] = tone * env * 0.5
    return out


# 8  hitmarker 1189-1198, _synth_hitmarker 2127-2157 (global randf)
def hitmarker(rng, kind="body", dmg_ratio=1.0):
    pitch_factor = 1.0 / pow(dmg_ratio, 0.45)
    decay_factor = lerp(1.0, 0.45, clamp((dmg_ratio - 1.0) / 4.0, 0.0, 1.0))
    dur = clamp(0.05 / decay_factor, 0.05, 0.14)
    n = int(dur * SR)
    out = np.zeros(n)
    f1, f2 = 2500.0, 1700.0
    if kind == "head":
        f1, f2 = 3400.0, 2250.0
    elif kind == "kill":
        f1, f2 = 1800.0, 1200.0
    f1 *= pitch_factor
    f2 *= pitch_factor
    for i in range(n):
        t = i / SR
        env = math.exp(-t * 100.0 * decay_factor)
        attack_env = math.exp(-t * 600.0 * decay_factor)
        noise = rng.randf_range(-1.0, 1.0) * attack_env * 0.5
        tone = math.sin(2.0 * PI * f1 * t) * 0.55 + math.sin(2.0 * PI * f2 * t) * 0.35
        out[i] = math.tanh((noise + tone * env) * 0.9)
    return out


# 8  pick_countdown_beep 1148-1153, _synth_pick_beep 1556-1569 (deterministic)
def pick_beep(rng, seconds_left=3):
    semitones_up = clamp(3 - int(seconds_left), 0, 2)
    dur = 0.16
    n = int(dur * SR)
    out = np.zeros(n)
    freq = 440.0 * pow(2.0, semitones_up / 12.0)
    for i in range(n):
        t = i / SR
        env = math.exp(-t * 14.0) * (1.0 - math.exp(-t * 200.0))
        out[i] = math.sin(2.0 * PI * freq * t) * env * 0.4
    return out


# 8  pling 1154-1156, _synth_pling 2066-2078 (deterministic; cached per round(pr * 100))
def pling(rng, pitch_ratio=1.0):
    dur = 0.25
    n = int(dur * SR)
    out = np.zeros(n)
    freq = 880.0 * pitch_ratio
    for i in range(n):
        t = i / SR
        out[i] = math.sin(2.0 * PI * freq * t) * math.exp(-t * 22.0) * 0.5
    return out


# 8  card_flip 1164-1181, _synth_card_flip 1864-1905 (seeded per variant)
def card_flip(rng):
    dur = lerp(0.16, 0.22, rng.randf())
    noise_dur = dur * lerp(0.65, 0.80, rng.randf())
    tick_start = dur * lerp(0.78, 0.90, rng.randf())
    tick_decay = lerp(180.0, 320.0, rng.randf())
    wash_lp_k = lerp(0.20, 0.45, rng.randf())
    tick_hp_k = lerp(0.30, 0.55, rng.randf())
    n = int(dur * SR)
    out = np.zeros(n)
    wash_lp = tick_lp = 0.0
    for i in range(n):
        t = i / SR
        s = 0.0
        if t < noise_dur:
            env = math.sin(PI * (t / noise_dur)) * 0.20
            noise = rng.randf_range(-1.0, 1.0)
            wash_lp = lerp(wash_lp, noise, wash_lp_k)
            s += wash_lp * env
        if t >= tick_start:
            dt = t - tick_start
            n_t = rng.randf_range(-1.0, 1.0)
            tick_lp = lerp(tick_lp, n_t, tick_hp_k)
            s += (n_t - tick_lp) * math.exp(-dt * tick_decay) * 0.5
        out[i] = s
    return out


# 8  pickup_drop 1158-1159, _synth_pickup_drop 2080-2100 (deterministic)
def pickup_drop(rng):
    dur = 0.62
    n = int(dur * SR)
    out = np.zeros(n)
    for i in range(n):
        t = i / SR
        u = t / dur
        whoosh_env = (1.0 - u) * math.exp(-t * 4.5)
        whoosh_freq = lerp(920.0, 180.0, u * u)
        whoosh = math.sin(2.0 * PI * whoosh_freq * t) * whoosh_env * 0.22
        chime_start = dur * 0.58
        chime = 0.0
        if t >= chime_start:
            ct = t - chime_start
            chime = math.sin(2.0 * PI * 1180.0 * ct) * math.exp(-ct * 18.0) * 0.42
        out[i] = math.tanh((whoosh + chime) * 1.35) * 0.55
    return out


# 8  enemy_incoming 1161-1162, _synth_enemy_incoming 2102-2125 (global randf)
def enemy_incoming(rng):
    dur = 0.42
    n = int(dur * SR)
    out = np.zeros(n)
    sample_dt = 1.0 / SR
    lp = 0.0
    for i in range(n):
        t = i / SR
        u = t / dur
        env = math.exp(-t * 5.8) * smoothstep(0.0, 0.06, u)
        burst = 0.0
        if math.fmod(t, 0.07) < 0.022:
            burst = rng.randf_range(-1.0, 1.0) * 0.55
        tone = math.sin(2.0 * PI * lerp(210.0, 95.0, u) * t) * 0.28
        noise = rng.randf_range(-1.0, 1.0)
        rc = 1.0 / (TAU * lerp(180.0, 70.0, u))
        alpha = sample_dt / (rc + sample_dt)
        lp = lp + alpha * (noise - lp)
        out[i] = math.tanh((burst + tone + lp * 0.35) * env * 1.45) * 0.62
    return out


SOUNDS = {
    "air_strike_inbound": air_strike_inbound,
    "ion_cannon_charge": ion_cannon_charge,
    "rocket_descent": rocket_descent,
    "mine_plant": mine_plant,
    "dash": dash,
    "lava_sizzle": lava_sizzle,
    "hit_received": hit_received,
    "kill_confirm": kill_confirm,
    "hitmarker": hitmarker,
    "pick_beep": pick_beep,
    "pling": pling,
    "card_flip": card_flip,
    "pickup_drop": pickup_drop,
    "enemy_incoming": enemy_incoming,
}
