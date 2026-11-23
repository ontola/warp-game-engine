local catalog = require("weapon_catalog")
local wheel = {}
local tau = 2*math.pi

-- Slot one points up; stick and mouse use +X right, +Y up in every viewport.
local function sector(x,y,count)
	return math.floor(math.atan2(x,y)*count/tau+.5)%count+1
end

local function choose(seat,control)
	local pressed=control.pressed or {}
	local current=table.find(catalog.order,seat.weaponChoice) or 1
	local step=if pressed.right or pressed.down then 1 elseif pressed.left or pressed.up then -1 else 0
	if step~=0 then
		seat.weaponChoice=catalog.order[(current-1+step)%#catalog.order+1]
		seat.wheelX,seat.wheelY=0,0
		return
	end
	local x,y=control.aim_x or 0,control.aim_y or 0
	if x*x+y*y > .10 then
		seat.weaponChoice=catalog.order[sector(x,y,#catalog.order)]
		seat.wheelX,seat.wheelY=0,0
		return
	end
	-- Look is mouse displacement; integrating it also accepts a slow sweep.
	local dx,dy=control.look_x or 0,control.look_y or 0
	if dx==0 and dy==0 then return end
	x,y=(seat.wheelX or 0)+dx*9,(seat.wheelY or 0)+dy*9
	local length=math.sqrt(x*x+y*y)
	local scale=math.max(1,length)
	seat.wheelX,seat.wheelY=x/scale,y/scale
	if length>.25 then seat.weaponChoice=catalog.order[sector(x,y,#catalog.order)] end
end

local function close(seat)
	seat.weaponWheelOpen,seat.weaponChoice,seat.wheelX,seat.wheelY=nil,nil,nil,nil
end

function wheel.input(state,input)
	for id,seat in state.fleet.players do
		if seat.bot then continue end
		local control=input[id] or {}
		local held=control.held or {}
		local down=held.left_shoulder==true
		local blocked=seat.mode~="walk" or seat.impact or state.storyBoard.owner==id
		local open=seat.weaponWheelOpen
		if blocked then
			if open then close(seat) end
			seat.wheelFireHeld=nil
		elseif down and (open or not seat.wheelWasDown) then
			if not open then
				seat.weaponWheelOpen=true
				seat.weaponChoice=if seat.weapon and table.find(catalog.order,seat.weapon) then seat.weapon else catalog.order[1]
				seat.wheelX,seat.wheelY=0,0
			end
			choose(seat,control)
		elseif open then
			seat.weapon=seat.weaponChoice
			close(seat)
		end
		if not blocked and (open or seat.weaponWheelOpen) then
			-- Browsing consumes the confirm click and the camera axes. Walk and
			-- sprint are independent, and every other seat receives its own input.
			seat.wheelFireHeld=held.fire or held.south or nil
			input[id]={move_x=control.move_x,move_y=control.move_y,
				held={right_shoulder=held.right_shoulder},pressed={}}
		elseif seat.wheelFireHeld then
			if held.fire or held.south then
				local filtered=table.clone(control)
				filtered.held,filtered.pressed=table.clone(held),table.clone(control.pressed or {})
				for _,buttons in {filtered.held,filtered.pressed} do
					buttons.fire,buttons.right_trigger,buttons.south,buttons.jump=false,false,false,false
				end
				input[id]=filtered
			else seat.wheelFireHeld=nil end
		end
		seat.wheelWasDown=down or nil
	end
end

-- The HUD exposes rectangles. Cache runs of equal colour in a radial mask
-- once per slot count, then draw only these runs at each seat's own scale.
local radius,hub,thickness,gap=112,43,1,1.35
local cachedCount,bands=0,{}
local function mask(x,y,count)
	local r=math.sqrt(x*x+y*y)
	if r<=hub-3 then return -1 end
	if r<hub or r>radius then return 0 end
	local s=sector(x,-y,count)
	local angle=math.atan2(x,-y)
	local mid=(s-1)*tau/count
	local delta=math.abs((angle-mid+math.pi)%tau-math.pi)
	if count>1 and r*math.sin(math.pi/count-delta)<gap then return 0 end
	return s+(if r>radius-4 then count else 0)
end

local function ring(count)
	if count==cachedCount then return bands end
	bands={}
	local rays={}
	for i=1,count do
		local a=(i-.5)*tau/count
		table.insert(rays,{math.sin(a),-math.cos(a)})
	end
	for y=-radius,radius-thickness,thickness do
		local cy=y+thickness*.5
		local outer=math.sqrt(radius*radius-cy*cy)
		local cuts={-outer,outer}
		-- Intersect each scanline with the exact circles and gutter edges.
		for _,r in {radius-4,hub,hub-3} do
			if math.abs(cy)<r then
				local span=math.sqrt(r*r-cy*cy)
				table.insert(cuts,-span)
				table.insert(cuts,span)
			end
		end
		for _,ray in rays do
			if math.abs(ray[2])<.0001 then continue end
			for _,offset in {-gap,gap} do
				local x=(cy*ray[1]+offset)/ray[2]
				if math.abs(x)<outer then table.insert(cuts,x) end
			end
		end
		table.sort(cuts)
		local last
		for i=2,#cuts do
			local left,right=cuts[i-1],cuts[i]
			if right-left<.0001 then continue end
			local role=mask((left+right)*.5,cy,count)
			if role==0 then last=nil
			elseif last and last[4]==role then last[3]=right-last[1]
			else
				last={left,y,right-left,role}
				table.insert(bands,last)
			end
		end
	end
	cachedCount=count
	return bands
end
ring(#catalog.order)

local icons={
	pistol={{-16,-7,29,7},{11,-5,6,4},{-13,0,10,4},{-13,3,7,7},{-15,9,7,3},{-3,2,8,2},{4,-1,2,4}},
	launcher={{-21,-5,39,10},{-22,-8,5,15},{16,-9,7,17},{-9,-10,7,4},{-5,5,6,8},{4,5,7,3}},
	rifle={{-22,-3,10,6},{-22,2,4,5},{-11,-5,24,9},{11,-3,12,3},{20,-4,4,4},{-7,3,4,7},{1,3,6,8},{-1,-8,8,3}},
	shotgun={{-22,1,8,9},{-17,-2,12,7},{-7,-5,28,4},{-3,-1,22,3},{1,1,12,4},{-10,3,5,3}},
	uzi={{-15,-5,25,10},{8,-3,9,4},{-13,-8,6,3},{4,-8,4,3},{-6,4,7,10},{-5,13,9,3},{-15,1,4,5}},
	sniper={{-23,-2,10,5},{-23,3,4,4},{-13,-4,23,7},{8,-3,16,2},{21,-4,4,4},{-10,3,4,7},{-6,-8,2,4},{3,-8,2,4},{-9,-10,17,3},{-9,-11,3,5},{6,-11,3,5}},
}

local function label(value,x,y,size,color)
	hud.text(value,x-hud.measure(value,size)*.5,y,{size=size,color=color})
end

function wheel.name(seat)
	return catalog.get(seat.weapon or catalog.order[1],seat).name
end

function wheel.draw(seat,x,y,w,h,s)
	local cx,cy=x+w*.5,y+h*.47
	local k=math.min(s*1.35,h/280,w/265)
	local count=#catalog.order
	local selected=table.find(catalog.order,seat.weaponChoice) or 1
	hud.box(x,y,w,h,"#0915205e")
	for _,row in ring(count) do
		local role=row[4]
		local color
		if role==-1 then color="#12232cf2"
		elseif role>count then color=if role-count==selected then "#ffd360" else "#758b91da"
		else color=if role==selected then "#415959f5" else "#162832f1" end
		hud.box(cx+row[1]*k,cy+row[2]*k,row[3]*k,thickness*k,color)
	end
	for i,id in catalog.order do
		local a=(i-1)*tau/count
		local ix,iy=cx+math.sin(a)*78*k,cy-math.cos(a)*78*k
		local color=if i==selected then "#fff6df" else "#adbec4"
		for _,p in icons[id] or icons.rifle do hud.box(ix+p[1]*k,iy+p[2]*k,p[3]*k,p[4]*k,color) end
	end
	local spec=catalog.get(catalog.order[selected],seat)
	local lines={}
	local font=16*k
	for word in string.gmatch(string.upper(spec.name),"%S+") do
		local line=lines[#lines]
		if line and hud.measure(line.." "..word,font)<72*k then lines[#lines]=line.." "..word
		else table.insert(lines,word) end
	end
	for _,line in lines do font=math.min(font,72*k/math.max(1,hud.measure(line,16*k))*(16*k)) end
	for i,line in lines do label(line,cx,cy+(i-1-#lines*.5)*font,font,"#fff6df") end
	local hint="Mouse / right stick / ← →     Release Q / LB to equip"
	local hs=math.min(14*k,w*.90/hud.measure(hint,14*k)*(14*k))
	local hw=math.min(w,hud.measure(hint,hs)*1.07+24*k)
	local hy=cy+(radius+8)*k
	hud.box(cx-hw*.5,hy,hw,22*k,"#101e26ed")
	label(hint,cx,hy+2*k,hs,"#fff6df")
end

return wheel
