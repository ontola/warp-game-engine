-- Card-pick countdown beep, after sfx.gd _synth_pick_beep (1556-1569).
-- A sine blip with a 200/s attack and e^-14t tail, one semitone higher per second
-- closer to the timeout. Deterministic in Godot. Fixed 0.16 s.
--
-- Params:
--   seconds_left  seconds until the pick times out (game.gd:3489): 3 -> 440 Hz, 2 -> +1
--                 semitone, 1 -> +2. Godot plays step clampi(3 - seconds_left, 0, 2).
-- Game (pick_countdown_beep 1148-1153): 2D, -8 dB, SPL 140.
length(0.16)

local step = expr("clamp(3 - floor(s + 0.5), 0, 2)", { s = param("seconds_left", 3) })
local f = expr("440 * 2 ^ (k / 12)", { k = step })
return expr("sin(tau * f * t) * exp(-t * 14) * (1 - exp(-t * 200)) * 0.4 * (t < 0.16)", { f = f })
