-- Brass above a muted iron strike when the double load closes the bridge.
length(0.44)
return expr("(sin(tau*233*t)*exp(-19*t)+0.54*sin(tau*349*t)*exp(-11*t)+0.27*sin(tau*699*t)*exp(-24*t))*(1-exp(-120*t))*0.32")
