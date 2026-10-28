-- Hit received thud, after sfx.gd _synth_hit_received (1540-1554).
-- A tone falling 160 -> 70 Hz over 80 ms plus a white-noise burst (e^-80t), under
-- (1 - t/dur)^2. As in Godot the tone is sin(2 pi f(t) t), not an integrated phase, so the
-- sweep chirps a little harder than its nominal range. Fixed 0.18 s.
--
-- Params: none. The game's intensity (player.gd:3830, clamp(dmg / 25, 0.04, 2)) only sets
--   the playback gain.
-- Game (hit_received 1136-1143): 2D, -12 + 20 log10(clamp(intensity, 0.05, 2)) dB, SPL 130.
length(0.18)

-- Godot draws one uniform white sample per 44.1 kHz tick; at a higher rate the same
-- variance spreads over more bandwidth, so scale it to keep Godot's noise density.
local function white() return noise() * expr("sqrt(sr / 44100)") end

local tone = expr("sin(tau * lerp(160, 70, clamp(t / 0.08, 0, 1)) * t)")
return expr("(a * 0.8 + n * exp(-t * 80) * 0.4) * max(1 - t / 0.18, 0) ^ 2 * 0.75 * (t < 0.18)", { a = tone, n = white() })
