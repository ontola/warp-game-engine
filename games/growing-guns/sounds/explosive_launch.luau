-- Subtle explosive-round gas/thump layer, after sfx.gd _synth_grenade_launch (1301-1359).
-- Rising band of gas noise, a breech tick at 38 ms, then a 160 Hz tube body
-- that eases out by 0.18 s.
--
-- Params: none. Godot picks one of 5 variants (body pitch 155/158/160/162/164
-- Hz); here each voice draws its variant from its seed.
-- Game: -22/-24 dB under the report, bounded with report voices, tiny room send.
length(0.18)

-- white44(): Godot's noise density (one uniform sample per 44.1 kHz tick).
-- godot_lp(x, k) / godot_hp(x, k): Godot's one-pole lerp(y, x, k) at 44.1 kHz, matched at
-- any rate, including its response near Nyquist (built in; see the Luau prelude).

-- Gas: white noise through a band that sweeps 650 Hz -> 4.3 kHz over 65 ms
-- (cascaded one-poles, hi - LP(hi)).
local fc = "lerp(650, 4300, smoothstep(0, 0.065, t))"
local hi = godot_lp(white44(), expr("clamp(" .. fc .. " * tau * 1.8 / 44100, 0.04, 0.92)"))
local lo = godot_lp(hi, expr("clamp(" .. fc .. " * tau * 0.35 / 44100, 0.004, 0.45)"))
local gas = (hi - lo) * expr("sin(pi * clamp(t / 0.065, 0, 1)) * exp(-max(0, t - 0.06) * 55) * 0.35")

-- Tick: high-passed noise snap at 38 ms.
local tick_noise = white44() * expr("step(0.038, t) * exp(-(t - 0.038) * 420)")
local tick = godot_hp(tick_noise, 0.035) * 0.10

-- Body: 160 Hz (+ variant offset) with faint 2nd/3rd harmonics and a sub-octave,
-- smoothstep attack from 26 ms and smoothstep release to 0.18 s.
local body_hz = expr("155 + 3 * (v > 1) + 2 * (v > 2) + 2 * (v > 3) + 2 * (v > 4)", { v = rand(0, 5) })
local body = expr([[(0.86 * sin(tau * f * (t + 1 / sr)) + 0.018 * sin(2 * tau * f * (t + 1 / sr))
    + 0.006 * sin(3 * tau * f * (t + 1 / sr)) + 0.06 * sin(0.5 * tau * f * (t + 1 / sr)))
    * smoothstep(0, 0.012, t - 0.026) * smoothstep(0, 1, 1 - clamp((t - 0.026) / 0.154, 0, 1))]], { f = body_hz })

return expr("tanh(x * 1.35) * 0.72", { x = mix(gas, tick, body) })
