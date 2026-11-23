-- Port Meridian's third-person silhouette of the Growing Guns stat-built model.
-- These dimensions and attachment thresholds follow Look::of and build in
-- games/growing-guns/app/src/gun.rs. The wrist frame points along local +Z.
local prop = {}
local v = vector.create
local upright = v(math.pi/2, 0, 0)
local grip = {color="#4d402e", roughness=.70}
local receiver = {color="#737e85", metal=.85, roughness=.42}
local dark = {color="#505b63", metal=.75, roughness=.42}
local tube = {color="#88949a", metal=.82, roughness=.41}
local lens = {color="#69b8ed", emissive=.7, roughness=.32}

function prop.warm()
	for _,material in {grip, receiver, dark, tube} do world.warm("cube",material) end
	for _,material in {dark, tube, lens} do world.warm("capsule",material) end
end

local function proportions(spec)
	local accuracy = math.clamp(1-spec.spread/.05,0,1)
	local damage = math.clamp((spec.damage-1)/4,0,1)
	return .20+1.20*accuracy^10, .24+.31*damage, .072+.088*damage, damage
end

-- The rear wrist socket is shared by the visible mesh and its muzzle flash.
function prop.tip(hand,dir,up,spec)
	local length,body,high = proportions(spec)
	return hand+dir*(.143 + body*.5 + length + .05)+up*(high*.5+.055)
end

function prop.make(hand,rotation,spec)
	local length,body,high,damage = proportions(spec)
	local radius = .022*(1+.7*damage)
	local rate = .22/spec.interval
	local rotary = rate>=5
	-- Small adjacent barrels stop resolving at city camera distance. Use up to
	-- four for a planar bundle, or the original fixed six for a rotary one.
	local count = if rotary then 6 else math.clamp(spec.pellets,1,4)
	local pitch = radius*2.4
	local width = if rotary then (radius*2.8)*2.2 else math.max(.055+.020*damage,pitch*(count-1)+radius*2+.010)
	local front = body*.5
	local top = high*.5
	local center = v(0,top+.055,.143)
	local root = world.spawn("cube",{tag="growing-gun",pos=hand,rot=rotation,visible=false,solid=false})
	local function piece(shape,material,x,y,z,size,rot)
		local opts = table.clone(material)
		opts.tag,opts.solid,opts.pos,opts.size = "growing-gun-part",false,center+v(x,y,z),size
		opts.rot = rot
		world.spawn(shape,opts,root)
	end
	local function box(mat,x,y,z,w,h,d)
		piece("cube",mat,x,y,z,v(w,h,d))
	end
	local function barrel(mat,x,y,z,diameter,depth)
		piece("capsule",mat,x,y,z,v(diameter,depth,diameter),upright)
	end

	box(receiver,0,0,0,width,high,body)
	box(dark,0,-high*.34,-body*.02,width*1.04,high*.34,body*.92)
	piece("cube",grip,0,-top-.056,-.105,v(.042,.13,.065),v(.20,0,0))

	for i=1,count do
		local x,y = (i-(count+1)*.5)*pitch,0
		if rotary then
			local angle = (i-1)*2*math.pi/count
			x,y = math.cos(angle)*radius*1.8,math.sin(angle)*radius*1.8
		end
		barrel(tube,x,y,front+length*.5,radius*2,length)
		if not rotary then
			if spec.damage>=3 then box(dark,x,y,front+length+.025,radius*4.5,radius*1.6,.05)
			else barrel(dark,x,y,front+length+.025,radius*3.1,.05) end
		end
	end

	local bps = math.min(240,spec.pellets/spec.interval)
	if rotary then
		for _,z in {front+.01,front+length+.025} do barrel(dark,0,0,z,radius*6.3,.05) end
	elseif bps>=6 then
		local extent = radius+.012
		local cover = length*(.34+.58*math.clamp((bps-6)/36,0,1))
		-- Keep the dark rails separated so the lighter tube shows through the vents.
		for _,sign in {-1,1} do box(dark,0,sign*(extent-.003),front+cover*.5,width,.006,cover) end
		for _,z in {front+.012,front+cover-.012} do box(dark,0,0,z,width,extent*2,.024) end
	end

	if spec.magazine>30 then
		local diameter = math.clamp(.18+.006*(spec.magazine-30),.18,.36)
		barrel(dark,0,-top-diameter*.5,.08,diameter,.07)
	else
		local drop = math.clamp(.04+.020*spec.magazine,.06,.6)
		local depth = .045+.045*damage
		box(grip,0,-top-drop*.5+.003,.045+depth*.5,.044+.026*damage,drop,depth)
	end
	if length>.55 then
		box(grip,0,-top+.045,-front-.10,.05,.09,.20)
		box(grip,0,-radius-.068,front+.16,.043,.12,.05)
	end
	local rail = math.clamp((spec.damage-1.15)/.35,0,1)*.02
	if rail>.00002 then box(dark,0,top+rail*.5,0,math.min(width*.8,.04),rail,body*.85) end
	if spec.spread<.003 then
		local y = top+rail+.025
		box(dark,0,top+rail+.01,0,.039,.035,.12)
		barrel(dark,0,y,0,.043,.23)
		barrel(dark,0,y,.109,.057,.044)
		barrel(lens,0,y,.131,.048,.005)
	else
		box(dark,0,radius+.016,front+length*.85,.008,.04,.015)
	end
	return root
end

return prop
