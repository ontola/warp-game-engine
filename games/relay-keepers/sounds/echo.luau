-- The hourglass turns: a falling, reversed-sounding whoosh with a bell.
-- Render it without speakers: warp audio render sounds/echo.luau
length(0.7)
local sweep = expr("sin(tau * (1400 * t - 900 * t * t)) * (t / 0.7) ^ 2 * 0.35")
local bell = expr("sin(tau * 660 * t) * exp(-t * 5) * 0.25 + sin(tau * 990 * t) * exp(-t * 8) * 0.12")
return expr("tanh((a + b) * 1.2) * 0.6 * (t < 0.7)", { a = sweep, b = bell })
