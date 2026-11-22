-- A shared world prop for the arcade contract. A unit-scale frame follows the
-- animated wrist; its children keep the authored offsets under pitch and yaw.
local sidearm={}
local v=vector.create
local SCALE=.68
local dark={color="#34414a",roughness=.74}
local slide={color="#a8babc",roughness=.57,metal=.18}
local trim={color="#dc914e",roughness=.66}
local glow={color="#ffb84e",emissive=2.6,roughness=1}

function sidearm.warm()
	for _,look in {dark,slide,trim} do world.warm("cube",look) end
	world.warm("sphere",glow)
end

local function part(root,tag,look,pos,size,pitch)
	local o=table.clone(look)
	o.tag,o.pos,o.size,o.solid=tag,pos*SCALE,size*SCALE,false
	if pitch then o.rot=v(pitch,0,0) end
	return world.spawn("cube",o,root)
end

local function make(hand,rotation)
	local root=world.spawn("cube",{tag="story-sidearm",pos=hand,rot=rotation,visible=false,solid=false})
	part(root,"sidearm-grip",dark,v(0,-.017,.025),v(.076,.167,.082),-.27)
	part(root,"sidearm-frame",dark,v(0,.042,.106),v(.083,.058,.236))
	part(root,"sidearm-slide",slide,v(0,.084,.119),v(.086,.066,.293))
	part(root,"sidearm-trim",trim,v(0,.058,.218),v(.089,.020,.067))
	part(root,"sidearm-sight",dark,v(0,.124,.230),v(.021,.021,.027))
	return root
end

function sidearm.remove(ctx)
	if ctx.prop then world.remove(ctx.prop); ctx.prop=nil end
	ctx.tip,ctx.turn=nil,nil
end

function sidearm.clearFlash(ctx)
	if ctx.flash then world.remove(ctx.flash); ctx.flash=nil end
end

function sidearm.follow(ctx,avatar,seat,ray)
	local wrist=avatar and seat.mode=="walk" and not seat.impact and humans.joint(avatar,"hand_r")
	if not wrist then sidearm.remove(ctx); return end
	local yaw=seat.yaw
	local down=.78
	local dir=ray or v(-math.sin(yaw)*math.cos(down),-math.sin(down),-math.cos(yaw)*math.cos(down))
	-- The local slide extends toward +Z. YXZ is the world entity rotation order.
	local heading=math.atan2(dir.x,dir.z)
	local rotation=v(-math.asin(math.clamp(dir.y,-1,1)),heading,0)
	local right=v(math.cos(heading),0,-math.sin(heading))
	local up=vector.cross(dir,right)
	local hand=wrist+dir*.048-up*.015
	if not ctx.prop then ctx.prop=make(hand,rotation)
	else world.set(ctx.prop,{pos=hand,rot=rotation}) end
	ctx.tip,ctx.turn=hand+(dir*.292+up*.084)*SCALE,rotation
end

function sidearm.fire(ctx)
	if not ctx.tip then return end
	sidearm.clearFlash(ctx)
	local o=table.clone(glow)
	o.tag,o.pos,o.rot,o.size,o.solid="sidearm-flash",ctx.tip,ctx.turn,v(.085,.085,.153)*SCALE,false
	ctx.flash=world.spawn("sphere",o)
end

return sidearm
