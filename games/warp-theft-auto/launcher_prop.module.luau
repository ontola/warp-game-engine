-- An arcade launcher assembled from existing city primitives. The root uses the
-- same animated wrist frame as the pistol; local +Z points towards the reticle.
local prop = {}
local v=vector.create
local shell={color="#748b80",roughness=.69,metal=.12}
local dark={color="#283540",roughness=.78,metal=.10}
local amber={color="#e5ae54",roughness=.65}
local parts={
	{"capsule",shell,v(0,.085,.06),v(.166,.79,.166),math.pi*.5},
	{"capsule",dark,v(0,.085,-.246),v(.19,.15,.19),math.pi*.5},
	{"capsule",dark,v(0,.085,.354),v(.195,.16,.195),math.pi*.5},
	{"cube",amber,v(0,.092,.301),v(.176,.154,.030)},
	{"cube",dark,v(0,.166,.047),v(.068,.075,.15)},
	{"cube",dark,v(0,-.014,.035),v(.075,.145,.083)},
}

function prop.warm()
	for _,part in parts do world.warm(part[1],part[2]) end
end

function prop.make(hand,rotation)
	local root=world.spawn("cube",{tag="arcade-launcher",pos=hand,rot=rotation,visible=false,solid=false})
	for _,part in parts do
		local opts=table.clone(part[2])
		opts.tag,opts.pos,opts.size,opts.solid="launcher-part",part[3],part[4],false
		-- Warp's capsule spans two local metres along Y. The artwork lists full lengths.
		if part[1]=="capsule" then opts.size*=v(1,.5,1) end
		if part[5] then opts.rot=v(part[5],0,0) end
		world.spawn(part[1],opts,root)
	end
	return root
end

function prop.tip(hand,dir,up)
	local tube=parts[1]
	return hand + dir*(tube[3].z+tube[4].y*.5) + up*tube[3].y
end

return prop
