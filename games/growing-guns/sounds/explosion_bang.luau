-- Explosion bang, after sfx.gd _synth_explosion (1410-1486).
-- The gunshot's layers at explosion scale: low-passed saturated thwack, noise
-- tail, swept kick and a low-passed sub-noise body, all through one tanh.
--
-- Params:
--   radius  blast radius in m; Godot passes round(clamp(radius, 2, 24)).
-- Warp mix: one +10 dB report with a 0.08 room send and the quiet hall,
-- no amplified close-range gain. Shared bass plays separately at 0 dB.
-- Overlap budgets preserve existing tails instead of stealing live voices.
-- The ion cannon still adds its separate ion_burst layer.
length(1.6)

-- white44(): Godot's noise density (one uniform sample per 44.1 kHz tick).
-- godot_lp(x, k) / godot_hp(x, k): Godot's one-pole lerp(y, x, k) at 44.1 kHz, matched at
-- any rate, including its response near Nyquist (built in; see the Luau prelude).

local r = param("radius", 6)

local R = expr("clamp(r / 6, 0.3, 4)", { r = r })
local dur = expr("clamp((0.4 + 0.08 * r) * j, 0.4, 1.6)", { r = r, j = rand(0.92, 1.10) })
local env = expr("exp(-1.4 * t) * max(1 - t / d, 0) ^ 0.6", { d = dur })

-- Thwack: low-passed (k 0.10-0.16, 0.74-1.2 kHz), saturated noise burst.
local thwack = expr("tanh(x * g) * 1.5 * exp(-t * k)", {
  x = godot_lp(white44(), rand(0.10, 0.16)), g = rand(4, 8), k = rand(180, 280) })

-- Noise tail (k 0.45 * jitter, 3.4-5.1 kHz) with its own fast decay.
local bright = expr("clamp(0.45 * j, 0.05, 0.95)", { j = rand(0.85, 1.15) })
local tail = godot_lp(white44(), bright) * expr("0.5 * exp(-t * k)", { k = rand(8, 15) })

-- Kick: 220 Hz falling to 28-80 Hz, low-passed at k 0.30 (2.5 kHz).
local f1 = expr("clamp(55 / sqrt(R), 28, 80) * j", { R = R, j = rand(0.92, 1.10) })
local kick = godot_lp(sine(sweep(220 * rand(0.88, 1.12), f1, rand(70, 95))), 0.30)
  * expr("exp(-t * clamp(35 / sqrt(R), 10, 40)) * lerp(0.55, 0.95, clamp((R - 1) / 3, 0, 1))", { R = R })

-- Sub: noise low-passed at k 0.005-0.010 (35-70 Hz), slow decay.
local sub = godot_lp(white44(), rand(0.005, 0.010))
  * expr("3.5 * 0.7 * exp(-t * clamp(3.5 / sqrt(R), 1.2, 4))", { R = R })

return expr("tanh(x * e * 1.4)", { x = mix(thwack, tail, kick, sub), e = env })
