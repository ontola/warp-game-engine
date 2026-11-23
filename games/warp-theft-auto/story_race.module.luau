-- Checkpoints must be crossed in order in the same race car.
local v = vector.create
return {
	id="race", title="Exchange Street Sprint",
	cast={{kind="car",key="comet",name="Pearl Comet",paint="pearl",pos=v(3.5,0,-17),yaw=0}},
	steps={
		{op="enter",car="comet",text="Get into the pearl Comet at the starting grid."},
		{op="checkpoint",car="comet",pos=v(0,0,-44),radius=7,text="North gate · head up Exchange Avenue."},
		{op="checkpoint",car="comet",pos=v(-45,0,-56),radius=7,text="Market turn · follow the westbound street."},
		{op="checkpoint",car="comet",pos=v(-56,0,-8),radius=7,text="Canal gate · turn south toward the square."},
		{op="checkpoint",car="comet",pos=v(-9,0,0),radius=7,text="Clock gate · cross the Exchange."},
		{op="checkpoint",car="comet",pos=v(0,0,-27),radius=7,text="Finish · cross the line on Exchange Avenue."},
	},
}
