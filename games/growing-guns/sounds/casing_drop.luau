-- Brass casing ping, after sfx.gd _synth_casing_drop (1777-1806).
-- Two damped sines from phase 0: a ~2 kHz body (fast decay) and a ~10 kHz ring.
--
-- Params: none (Godot's 8 seeded variants are this voice's own rand draws).
-- Game: on the casing's first contacts (60 ms debounce), dry, unit 6 m, SPL 75, no big tail.
--   gain  -18 + lerp(-22, 2, clamp(speed / ref_speed, 0.05, 1.2)) dB
--   pitch clamp((0.022 / barrel_radius) ^ 0.7, 0.55, 1.4) * U(0.96, 1.04) (playback rate)
length(0.18)

return expr([[tanh((0.45 * sin(tau * a * (t + 1 / sr)) * exp(-t * c)
    + 0.55 * sin(tau * b * (t + 1 / sr)) * exp(-t * d)) * 1.1)]], {
  a = rand(1900, 2150), b = rand(9600, 10500), c = rand(95, 115), d = rand(24, 32) })
