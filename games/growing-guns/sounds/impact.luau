-- Bullet impact on world geometry, after sfx.gd _synth_impact (1738-1775).
-- A high-passed noise click plus, above base damage, a small swept-sine thump.
-- Godot has one impact recipe: no per-surface set, no ricochet.
--
-- Params:
--   dmg_ratio  bullet damage / 22, clamped by the game to 0.5..6 (bullet.gd: clamp(dmg/22, 0.5, 5))
-- Game: non-explosive rounds hitting the world, at the hit point, dry, unit 8 m, no big tail.
--   gain -16 + dmg_db, SPL 90 + dmg_db, dmg_db = clamp(4 * log2(max(1, dmg_ratio)), 0, 9)
length(0.09)

-- Godot draws one uniform white sample per 44.1 kHz tick; at a higher rate the same
-- variance spreads over more bandwidth, so scale it to keep Godot's noise density.
local function white() return noise() * expr("sqrt(sr / 44100)") end

local d = expr("clamp(x, 0.5, 6)", { x = param("dmg_ratio", 1) })
local dn = expr("clamp((d - 1) / 4, 0, 1)", { d = d })

-- Click: noise minus its one-pole average (Godot k = clamp(cf * 2π * 0.4 / 44100, 0.05, 0.6)).
local hp_hz = expr("-log(1 - clamp(c * tau * 0.4 / 44100, 0.05, 0.6)) * 44100 / tau", { c = rand(2200, 4500) })
local click = highpass1(white(), hp_hz) * expr("exp(-t * k)", { k = rand(180, 320) })

-- Thump: 160-260 Hz falling to 45-80 Hz (lower for heavy rounds), silent at base damage.
local f1 = expr("j * lerp(1, 0.7, n)", { j = rand(45, 80), n = dn })
local thump = sine(sweep(rand(160, 260), f1, rand(60, 100))) * expr("exp(-t * k) * 0.65 * n", { k = rand(28, 42), n = dn })

local dur = expr("lerp(0.030, 0.080, n) * j", { n = dn, j = rand(0.92, 1.10) })
return expr("tanh(x * 1.4) * (t < d)", { x = click + thump, d = dur })
