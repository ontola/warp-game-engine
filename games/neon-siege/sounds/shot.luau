-- A pilot's shot: a quick falling square-ish blip. 0.07 s, quiet (it repeats).
length(0.07)
local f = expr("lerp(1500, 700, t / 0.07)")
return expr("tanh(sin(tau * f * t) * 3) * exp(-t * 45) * 0.12 * (t < 0.07)", { f = f })
