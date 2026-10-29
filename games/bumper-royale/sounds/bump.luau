-- Two pucks collide: a rubbery thump with a click on top. 0.18 s.
length(0.18)
local thump = expr("sin(tau * lerp(220, 70, t / 0.18) * t) * exp(-t * 22) * 0.6")
local click = lowpass(noise(), 4000) * decay(0.03, 4) * 0.35
return expr("tanh((a + b) * 1.6) * 0.7 * (t < 0.18)", { a = thump, b = click })
