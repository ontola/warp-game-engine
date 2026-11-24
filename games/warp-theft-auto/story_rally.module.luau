-- The story editor runs in an isolated Luau interpreter. These named positions
-- are resolved by the game against whichever cape the players are driving.
local car="kestrel"
return {
    id="mariner_rally", title="Mariner Ridge Rally",
    cast={{kind="car",key=car,name="Red Kestrel",paint="vermilion",pos={place="rallyStart"}}},
    steps={
        {op="enter",car=car,text="Head west out of town. Take the red Kestrel at the Mariner Wood trailhead."},
        {op="race_route",route="rally",car=car,radius=6,text="Follow the gravel road.",
            notes={"Climb through the timber line.","Follow the ridge to the first hairpin.",
                "Hairpin right. Drop to the hollow.","Turn uphill at the foot of the bank.",
                "Stay on gravel over the saddle.","Follow the long bend into the west wood.",
                "Round the western hairpin.","Crest. Brake for the descent.",
                "Turn back across the slope.","Last bend. Follow the runout to the coast road."},
            finish="Finish at the coast road."},
    },
}
