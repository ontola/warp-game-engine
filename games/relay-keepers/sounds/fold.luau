-- Two glassy tones cross in pitch as the keeper changes shores.
length(0.46)
local crossing=expr("sin(tau*(440*t+560*t*t))+0.7*sin(tau*(700*t-460*t*t))")
return expr("a*(1-exp(-55*t))*exp(-9*t)*0.31",{a=crossing})
