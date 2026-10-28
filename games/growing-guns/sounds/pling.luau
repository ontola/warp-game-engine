-- Pling, after sfx.gd _synth_pling (2066-2078): a 880 Hz x pitch_ratio sine with an
-- e^-22t decay. Deterministic in Godot. Fixed 0.25 s.
--
-- Params:
--   pitch_ratio  the game passes 1.15 (player.gd:4676). Godot caches per round(pitch_ratio * 100).
-- Game (pling 1154-1156): 2D, -10 dB, SPL 70.
length(0.25)

return expr("sin(tau * 880 * p * t) * exp(-t * 22) * 0.5 * (t < 0.25)", { p = param("pitch_ratio", 1) })
