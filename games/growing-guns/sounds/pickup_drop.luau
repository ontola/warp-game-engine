-- Pickup drop telegraph, after sfx.gd _synth_pickup_drop (2080-2100).
-- A falling whoosh tone (920 -> 180 Hz over u^2, (1 - u) e^-4.5t) and, from 58% of the
-- length, a 1180 Hz chime (e^-18t), through tanh. As in Godot the whoosh is
-- sin(2 pi f(u) t), not an integrated phase, so its real pitch falls faster than the
-- nominal range and sweeps through zero near the end. Deterministic. Fixed 0.62 s.
--
-- Params: none.
-- Game (pickup_drop 1158-1159, game.gd:3001): raytraced 3D at the drop (room reverb),
--   unit 12 m, -11 dB, SPL 75, big-tail send -24 dB.
length(0.62)

local whoosh = expr("sin(tau * lerp(920, 180, (t / 0.62) ^ 2) * t) * (1 - t / 0.62) * exp(-t * 4.5) * 0.22")
local cs = 0.62 * 0.58
local chime = expr("sin(tau * 1180 * (t - " .. cs .. ")) * exp(-(t - " .. cs .. ") * 18) * 0.42 * step(" .. cs .. ", t)")
return expr("tanh((a + b) * 1.35) * 0.55 * (t < 0.62)", { a = whoosh, b = chime })
