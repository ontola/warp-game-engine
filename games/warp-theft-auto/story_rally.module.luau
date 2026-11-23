-- Surveyed gates along the gravel centreline. Pick up the rally car at the
-- eastern trailhead; the clock starts when its driver takes the wheel.
local v=vector.create
local car="kestrel"
local steps={{op="enter",car=car,text="Head west out of town. Take the red Kestrel at the Mariner Wood trailhead."}}
-- Mission ground positions snap to the same terrain the tyres follow.
local gates={
    {-615.34,-68.82,"Climb through the timber line."},
    {-656.93,-40.98,"Follow the ridge to the first hairpin."},
    {-733.09,-55.35,"Hairpin right. Drop to the hollow."},
    {-692.40,-111.38,"Turn uphill at the foot of the bank."},
    {-786.52,-66.43,"Stay on gravel over the saddle."},
    {-857.51,-6.15,"Follow the long bend into the west wood."},
    {-932.17,50.06,"Round the western hairpin."},
    {-865.75,40.59,"Crest. Brake for the descent."},
    {-808.04,68.06,"Turn back across the slope."},
    {-878.28,109.42,"Last bend. Follow the runout to the coast road."},
    {-821.00,142.55,"Finish at the northern trailhead."},
}
for _,g in gates do
    table.insert(steps,{op="checkpoint",car=car,pos=v(g[1],0,g[2]),radius=6,text=g[3]})
end
return {id="mariner_rally",title="Mariner Ridge Rally",
    cast={{kind="car",key=car,name="Red Kestrel",paint="vermilion",pos=v(-561,0,-88),yaw=2.49}},
    steps=steps}
