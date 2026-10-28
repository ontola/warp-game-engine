-- Gunshot, after sfx.gd _synth_shot (1202-1299). The weapon shapes it:
--   damage    max(1, weapon.get_damage() / 22) (no last-in-mag bonus: Godot's synth ignores it).
--             Bigger: longer, lower kick, more sub, lower output high-pass.
--   accuracy  clamp(1 - spread / 0.06, 0, 1); base spread 0.008 gives 0.87.
--             Higher: brighter tail and the 2.4 kHz click.
--   velocity  clamp(bullet_speed_mult, 0.5, 3). Higher: shorter, harder.
-- Every play draws its own jitter from the voice seed (Godot: 5 seeded variants).
-- Game (sfx.gd shot 771-846; the mixer applies these, not the patch):
--   dmg_db = clamp(6 log2(max(0.5, d)), -4, 14), d = get_damage_for_shot(last) / 22
--   pitch  = clamp(1 - 0.2 log2(max(1, d)), 0.35, 1.15) * U(0.97, 1.03) (x1.08 silenced), playback rate
--   dry: self 2D centre at -2 +-1.5 + dmg_db (+15.6 dB centre boost, max 18), others 3D dry unit 3 at
--        +5 +-1.5 + dmg_db; wet copy at dry - 10 dB, room reverb, unit 3; silenced -10 / -12 dB.
--   SPL 105 + 7 log2(max(1, d)).
--   Rumble: when d > 1.4, also play explosion_rumble.luau with radius = round(clamp(1.5 d, 2, 12))
--   at -12 dB (2D for the shooter, else at the muzzle), room reverb, unit clamp(0.8 radius, 4, 12),
--   the shot's SPL. Godot reuses the explosion rumble synth for it, so there is no separate patch.
local D = param("damage", 1)
local A = param("accuracy", 0.87)
local V = param("velocity", 1)
length(0.6)

-- Godot draws one uniform white sample per 44.1 kHz tick; at a higher rate the same
-- variance spreads over more bandwidth, so scale it to keep Godot's noise density.
local function white() return noise() * expr("sqrt(sr / 44100)") end

-- One-pole "lerp(y, x, k)" coefficients from the Godot code, as cutoffs.
local function k_hz(k) return expr("-log(1 - k) * 44100 / tau", { k = k }) end

local d = expr("max(d, 1)", { d = D })
local v = expr("clamp(v, 0.5, 3)", { v = V })
local a = expr("clamp(a, 0, 1)", { a = A })

local dur = expr("clamp((0.08 + 0.08 * log(d) / log(2)) * j / sqrt(v), 0.04, 0.35)", { d = d, j = rand(0.90, 1.12), v = v })
local env = decay(dur, 1.6)

-- Thwack: high-passed, saturated noise burst. The tanh flattens the noise into
-- nearly full-scale broadband noise, so keep Godot's per-sample level going in
-- and scale the result to Godot's noise density instead.
local raw = noise()
local thwack_drive = expr("j * lerp(0.9, 1.3, clamp((v - 1) / 2, 0, 1))", { j = rand(4, 8), v = v })
local thwack = expr("tanh(x * g) * 1.5 * sqrt(sr / 44100) * exp(-t * k)", {
  x = raw - lowpass1(raw, 434), g = thwack_drive, k = expr("j * sqrt(v)", { j = rand(180, 280), v = v }) })

-- Kick: exponential pitch drop 220 Hz to about 55 Hz (lower for big guns).
local f0 = 220 * rand(0.88, 1.12)
local f1 = expr("clamp(55 / sqrt(d), 28, 80) * j", { d = d, j = rand(0.92, 1.10) })
local kick = lowpass1(sine(sweep(f0, f1, rand(70, 95))), 2503)
  * expr("exp(-t * clamp(35 / sqrt(d), 10, 40)) * lerp(0.55, 1.10, clamp((d - 1) / 3, 0, 1))", { d = d })

-- Noise tail, brighter for accurate guns.
local bright = expr("clamp(lerp(0.18, 0.55, a) * j, 0.05, 0.95)", { a = a, j = rand(0.85, 1.15) })
local tail = lowpass1(white(), k_hz(bright)) * 0.5 * expr("exp(-t * j * sqrt(v))", { j = rand(8, 16), v = v })

-- Click: the crack of an accurate round.
local click = expr("sin(tau * 2400 * j * t) * exp(-t * 280) * 0.3 * a", { j = rand(0.80, 1.25), a = a })

local s = expr("tanh(x * e * 1.4)", { x = mix(thwack, tail, kick, click), e = env })
-- Damage-scaled output HP. Godot computes k = clamp(2π fc / 44100, 0.005, 0.5),
-- so the real cutoff is 223 Hz at base damage and never below 35 Hz.
local out = highpass1(s, k_hz(expr("clamp(tau * clamp(220 / d ^ 1.4, 25, 220) / 44100, 0.005, 0.5)", { d = d })))
-- Godot's buffer ends at dur and its WAV conversion hard-clamps to +-1 (the HP can overshoot).
return expr("clamp(x, -1, 1) * (t < d)", { x = out, d = dur })
