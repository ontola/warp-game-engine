-- An air bubble and a softer answer from the water.
length(0.38)
local bubble=expr("sin(tau*(270*t+1500*t*t))*(1-exp(-90*t))*exp(-26*t)")
local drop=expr("sin(tau*(490*(t-0.075)+780*(t-0.075)*(t-0.075)))*step(0.075,t)*exp(-22*(t-0.075))")
return expr("(a+b*0.48)*0.39",{a=bubble,b=drop})
