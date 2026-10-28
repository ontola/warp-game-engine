-- Kill confirm bell, after sfx.gd _synth_kill_confirm (1572-1584).
-- G4 + D5 (392 + 587 Hz) sines with an e^-9t decay. Deterministic in Godot. Fixed 0.28 s.
--
-- Params: none.
-- Game (kill_confirm 1144-1146, game.gd:4575/4587 for kind "kill"): 2D, -6 dB, SPL 130.
length(0.28)

return expr("(sin(tau * 392 * t) * 0.45 + sin(tau * 587 * t) * 0.3) * exp(-t * 9) * 0.5 * (t < 0.28)")
