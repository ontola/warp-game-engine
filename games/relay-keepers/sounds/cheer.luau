-- Room cleared: a bright four-note arpeggio (C E G C) that rings out,
-- with a soft shimmer on top. 0.9 s.
-- Render it without speakers: warp audio render sounds/cheer.luau
length(0.9)

local function note(hz, at)
    return expr("(sin(tau * f * (t - a)) + 0.25 * sin(tau * 2 * f * (t - a))) * exp(-(t - a) * 6) * 0.3 * step(a, t)", { f = hz, a = at })
end
local shimmer = expr("sin(tau * 3136 * t) * sin(tau * 9 * t) * exp(-t * 4) * 0.05")
return expr("tanh((a + b + c + d + e) * 1.1) * 0.6 * (t < 0.9)",
    { a = note(523, 0), b = note(659, 0.08), c = note(784, 0.16), d = note(1047, 0.26), e = shimmer })
