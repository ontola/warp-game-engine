-- A fictional arcade contract on the Port Meridian backlot.
local v = vector.create
return {
	id="contract", title="The Silver Jack Contract",
	cast={{kind="person",key="jack",name="Silver Jack",look=27,pos=v(8,0,40),yaw=0}},
	steps={
		{op="walk",person="jack",points={v(8,0,48),v(36,0,48),v(47,0,48)},speed=1.2,wait=false,
			text="Find Silver Jack on the Exchange backlot."},
		{op="contract",person="jack",range=32,text="Take down Silver Jack with the arcade pistol."},
	},
}
