-- Soles meet stone, followed by two short rifle handling sounds.
length(0.32)
local grit = godot_lp(white44(), 0.18)
local boot = expr("(1 - exp(-t * 250)) * exp(-t * 22) * (0.64 * sin(tau * (64 * t + 0.6 * (1 - exp(-t * 37)))) + 0.85 * n)", { n = grit })
local metal = godot_hp(godot_lp(white44(), 0.52), 0.16)
local function clack(at, hz)
    local age = expr("max(0, t - a)", { a = at })
    return expr("(1 - exp(-u * 1200)) * (n * exp(-u * 100) + 0.23 * sin(tau * h * u) * exp(-u * 74))", { u = age, n = metal, h = hz })
end
return boot + clack(0.095, 890) + 0.59 * clack(0.15, 1380)
