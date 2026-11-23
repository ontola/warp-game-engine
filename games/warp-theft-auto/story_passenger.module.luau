-- Edit the cast, dialogue and street coordinates to write another passenger run.
local v = vector.create
return {
	id="passenger", title="Last Ferry",
	cast={{kind="person",key="mara",name="Mara Vale",look=19,pos=v(8,0,31),yaw=math.pi}},
	steps={
		{op="walk",person="mara",points={v(8,0,24),v(8,0,17)},speed=1.5,
			text="Meet Mara at the Exchange curb."},
		{op="pickup",person="mara",radius=8,text="Stop a car beside Mara and let her board."},
		{op="dropoff",person="mara",pos=v(109,0,60),radius=7,
			text="Drive Mara to the Harbor curb and stop."},
		{op="walk",person="mara",points={v(120,0,62),v(120,0,73)},speed=1.7,
			text="See Mara to the ferry entrance."},
	},
}
