-- Gunshot, after sfx.gd _synth_shot. The weapon shapes it:
--   damage    damage / 22 (bigger: longer, lower kick, more sub)
--   accuracy  1 - spread / 0.06 (higher: brighter tail, 2.4 kHz click)
--   velocity  bullet speed multiplier (higher: shorter, harder)
-- Every play draws its own jitter from the voice seed.
local D = param("damage", 1)
local A = param("accuracy", 0.87)
local V = param("velocity", 1)
length(0.6)

-- One-pole "lerp(y, x, k)" coefficients from the Godot code, as cutoffs.
local function k_hz(k) return expr("-log(1 - k) * 44100 / tau", { k = k }) end

local d = expr("max(d, 1)", { d = D })
local v = expr("clamp(v, 0.5, 3)", { v = V })
local a = expr("clamp(a, 0, 1)", { a = A })

local dur = expr("clamp((0.08 + 0.08 * log(d) / log(2)) * j / sqrt(v), 0.04, 0.35)", { d = d, j = rand(0.90, 1.12), v = v })
local env = decay(dur, 1.6)

-- Thwack: high-passed, saturated noise burst.
local raw = noise()
local thwack_drive = expr("j * lerp(0.9, 1.3, clamp((v - 1) / 2, 0, 1))", { j = rand(4, 8), v = v })
local thwack = expr("tanh(x * g) * 1.5 * exp(-t * k)", {
  x = raw - lowpass1(raw, 434), g = thwack_drive, k = expr("j * sqrt(v)", { j = rand(180, 280), v = v }) })

-- Kick: exponential pitch drop 220 Hz to about 55 Hz (lower for big guns).
local f0 = 220 * rand(0.88, 1.12)
local f1 = expr("clamp(55 / sqrt(d), 28, 80) * j", { d = d, j = rand(0.92, 1.10) })
local kick = lowpass1(sine(sweep(f0, f1, rand(70, 95))), 2503)
  * expr("exp(-t * clamp(35 / sqrt(d), 10, 40)) * lerp(0.55, 1.10, clamp((d - 1) / 3, 0, 1))", { d = d })

-- Noise tail, brighter for accurate guns.
local bright = expr("clamp(lerp(0.18, 0.55, a) * j, 0.05, 0.95)", { a = a, j = rand(0.85, 1.15) })
local tail = lowpass1(noise(), k_hz(bright)) * 0.5 * expr("exp(-t * j * sqrt(v))", { j = rand(8, 16), v = v })

-- Click: the crack of an accurate round.
local click = expr("sin(tau * 2400 * j * t) * exp(-t * 280) * 0.3 * a", { j = rand(0.80, 1.25), a = a })

local s = expr("tanh(x * e * 1.4)", { x = mix(thwack, tail, kick, click), e = env })
return highpass1(s, expr("clamp(220 / d ^ 1.4, 25, 220)", { d = d }))
