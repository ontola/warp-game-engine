-- A short boot-and-seat kick with a burst of compressed air.
length(0.33)
local puff = godot_lp(white44(), expr("0.05 + 0.20 * exp(-t * 17)"))
local kick = expr("sin(tau * (73 * t + 1.15 * (1 - exp(-t * 32)))) * exp(-t * 21)")
return expr("(1 - exp(-t * 230)) * (0.72 * k + 1.85 * p * exp(-t * 11))", { k = kick, p = puff })
