local ui = require("kit/ui")
local city = require("world")
local dispatch = require("dispatch")
local sidearm = require("sidearm")
local wheel = require("weapon_wheel")
local instruments = {}
local ink, muted, panel = "#fff6df", "#b9cbd0", "#101e26ec"
local gold, aqua = "#ffd360", "#6bdfd0"

local function text(label, x, y, size, color)
	hud.text(label, x, y, {size = size, color = color or ink})
end

-- Rasterize the cape's actual coastline and road curves once into narrow columns.
-- The HUD draws rectangles; each seat only visits the columns inside its map.
local cape = require("west-cape-data")
local columnWidth = 12
local capeWest = cape.coast[1][1]
local mapColumns = {}
local groves = require("west-cape").groves
local parks = {{-84,84},{84,-84}}

local function prepareMap()
    local coastIndex = 1
    local east = cape.coast[#cape.coast][1]
    for left=capeWest,east-1,columnWidth do
        local right = math.min(left+columnWidth,east)
        local mid = (left+right)*.5
        while cape.coast[coastIndex+1][1]<mid do coastIndex+=1 end
        local a,b = cape.coast[coastIndex],cape.coast[coastIndex+1]
        local t = (mid-a[1])/(b[1]-a[1])
        local col = {{a[2]+(b[2]-a[2])*t, a[3]+(b[3]-a[3])*t, "#47574a"}}
        for _,g in groves do
            local r = (mid-g[1])/g[3]
            if math.abs(r)<1 then
                local span = g[4]*math.sqrt(1-r*r)
                table.insert(col,{g[2]-span,g[2]+span,"#234938"})
            end
        end
        local roadSpans = {}
        for route,road in cape.roads do
            for i=2,#road do
                local p,q = road[i-1],road[i]
                if math.min(p[1],q[1])<right and math.max(p[1],q[1])>=left then
                    local z0,z1 = p[2],q[2]
                    local dx = q[1]-p[1]
                    if dx~=0 then
                        z0 = p[2]+(q[2]-p[2])*math.clamp((left-p[1])/dx,0,1)
                        z1 = p[2]+(q[2]-p[2])*math.clamp((right-p[1])/dx,0,1)
                    end
                    local gravel = route==3
                    local width = if gravel then 2.6 else 3.7
                    local color = if gravel then "#d1a16e" else "#bdba9a"
                    table.insert(roadSpans,{math.min(z0,z1)-width,math.max(z0,z1)+width,color})
                end
            end
        end
        table.sort(roadSpans,function(a,b) return a[1]<b[1] end)
        local merged
        for _,span in roadSpans do
            if merged and merged[3]==span[3] and merged[2]>=span[1] then
                merged[2]=math.max(merged[2],span[2])
            else
                merged=span
                table.insert(col,merged)
            end
        end
        table.insert(mapColumns,col)
    end
end
prepareMap()

local function minimap(fleet, id, goal, x, y, size)
    local me = fleet.players[id]
    -- Keep the city overview around downtown. On the west approach the centre
    -- follows each player continuously and opens up to show more of the route.
    local t = math.clamp((-me.pos.x-55)/180,0,1)
    local follow = t*t*(3-2*t)
    local radius = city.BOUND+12+follow*67
    if me.mode=="fly" then radius+=math.clamp(me.pos.y*.45,0,80) end
    local inside = radius*.86
    local cx = math.clamp(me.pos.x*follow,me.pos.x-inside,me.pos.x+inside)
    local cz = math.clamp(me.pos.z*follow,me.pos.z-inside,me.pos.z+inside)
    local left,top = cx-radius,cz-radius
    local right,bottom = cx+radius,cz+radius
    local zoom = size/(2*radius)
    local function rect(x0,z0,x1,z1,color)
        local l,r = math.max(x0,left),math.min(x1,right)
        local u,d = math.max(z0,top),math.min(z1,bottom)
        if r>l and d>u then hud.box(x+(l-left)*zoom,y+(u-top)*zoom,(r-l)*zoom,(d-u)*zoom,color) end
    end
    local function dot(wx,wz,width,color)
        local half = width/(2*zoom)
        rect(wx-half,wz-half,wx+half,wz+half,color)
    end
    hud.box(x-2,y-2,size+4,size+4,"#adc2bca0")
    hud.box(x,y,size,size,"#1c3944")

    for i=math.max(1,math.floor((left-capeWest)/columnWidth)+1),math.min(#mapColumns,math.ceil((right-capeWest)/columnWidth)) do
        local edge = capeWest+(i-1)*columnWidth
        for _,span in mapColumns[i] do
            rect(edge,span[1],math.min(edge+columnWidth,-city.BOUND),span[2],span[3])
        end
    end
    local bound = city.BOUND
    if right > -bound and left < bound and bottom > -bound and top < bound then
        rect(-bound,-bound,bound,bound,"#283f40")
        for _,park in parks do rect(park[1]-18.5,park[2]-18.5,park[1]+18.5,park[2]+18.5,"#466349") end
        local edge = math.max(.6,size*.0065)/zoom
        for n=-3,3 do
            local p = n*city.STEP
            rect(p-edge,-bound,p+edge,bound,"#738c8b")
            rect(-bound,p-edge,bound,p+edge,"#738c8b")
        end
    end
    local function onMap(p)
        return p.x>=left and p.x<=right and p.z>=top and p.z<=bottom
    end
    for _,car in fleet.vehicles do
        if not car.owner and onMap(car.pos) then
            local heli = car.kind=="helicopter"
            dot(car.pos.x,car.pos.z,if heli then size*.046 else size*.025,if heli then aqua else muted)
        end
    end
    for pid,seat in fleet.players do
        if pid~=id and onMap(seat.pos) then dot(seat.pos.x,seat.pos.z,size*.055,"#ec8a87") end
    end
    if goal then
        local p = goal.pos
        if onMap(p) then
            dot(p.x,p.z,size*.077,gold)
            dot(p.x,p.z,size*.033,panel)
        else
            -- Intersect the direction FROM this seat with the inset map edge.
            local dx,dz = p.x-me.pos.x,p.z-me.pos.z
            local inset = radius*.075
            local kx = if dx==0 then math.huge else ((if dx>0 then right-inset else left+inset)-me.pos.x)/dx
            local kz = if dz==0 then math.huge else ((if dz>0 then bottom-inset else top+inset)-me.pos.z)/dz
            local k = math.max(0,math.min(kx,kz))
            local tx,tz = me.pos.x+k*dx,me.pos.z+k*dz
            local back = radius*.066/math.sqrt(dx*dx+dz*dz)
            dot(tx,tz,size*.043,gold)
            dot(tx-back*dx-back*dz*.44,tz-back*dz+back*dx*.44,size*.031,gold)
            dot(tx-back*dx+back*dz*.44,tz-back*dz-back*dx*.44,size*.031,gold)
        end
    end
    local d = size*.053
    dot(me.pos.x,me.pos.z,d+2,panel)
    dot(me.pos.x,me.pos.z,d,"#ffffff")
    local nose = radius*.065
    dot(me.pos.x-math.sin(me.yaw)*nose,me.pos.z-math.cos(me.yaw)*nose,d*.52,"#ffffff")
end
local venue = {office = "Office lobby", hotel = "Hotel lobby", warehouse = "Freight warehouse"}
local verbs = {recover = "Take marked car ", drive = "Drive to ", fly = "Land at ", walk = "Meet on foot at "}
local function drawStories(menu, titles, x, y, w, h, s)
	local panelW = math.min(w * .9, 345*s)
	local lh = 30*s
	local panelH = (112+30*#titles)*s
	local left, top = x+(w-panelW)/2, y+(h-panelH)/2
	hud.box(left,top,panelW,panelH,"#11232af8")
	text("STORIES",left+14*s,top+9*s,20*s,gold)
	for i, title in titles do
		local py = top + (18+i*30)*s
		if menu.choice == i then hud.box(left+6*s,py-2*s,panelW-12*s,lh,"#355454") end
		text(title,left+16*s,py,18*s,ink)
	end
	text(menu.message or "â†‘ â†“ / D-pad   Choose     Space / A   Play",
		left+14*s,top+panelH-47*s,14*s,if menu.message then gold else muted)
	text("R / X   Close",left+14*s,top+panelH-25*s,14*s,muted)
end

local function objective(story, left, top, width, s)
	local headline = if story.status=="active" then story.title .. "   " .. story.step .. "/" .. story.total else story.title
	if story.raceTime then headline ..= string.format("   %d:%04.1f", math.floor(story.raceTime / 60), story.raceTime % 60) end
	local info = if story.status=="complete" then "Mission complete" elseif story.status=="failed" then "Mission failed Â· " .. (story.objective or "") else story.objective or ""
	local lines, line = {}, ""
	for word in string.gmatch(info, "%S+") do
		local nextLine = if line=="" then word else line .. " " .. word
		if line~="" and hud.measure(nextLine, 17*s) > width-22*s then
			table.insert(lines,line)
			line = word
		else line = nextLine end
	end
	table.insert(lines,line)
	local height = (36+#lines*20)*s
	hud.box(left,top,width,height,panel)
	hud.box(left,top,3*s,height,gold)
	hud.text(string.upper(headline),left+11*s,top+6*s,{size=15*s,color=gold,width=width-22*s})
	for i, part in lines do text(part,left+11*s,top+(8+i*20)*s,17*s,ink) end
end

local function reticle(gun,seat,id,x,y,w,h,s)
	local cx,cy=x+w*.5,y+h*.5
	if gun.aiming then
		local c = if gun.onTarget then "#ff9774" else ink
		local gap = 8+36*(gun.shotFlash or 0)
		for _,d in {-1,1} do
			hud.box(cx+(d*gap-1)*s,cy-s,3*s,2*s,c)
			hud.box(cx-s,cy+(d*gap-1)*s,2*s,3*s,c)
		end
	else
		-- The exploration camera looks down toward the avatar. Project the hip
		-- shot direction into this seat's view, where the player can actually fire.
		local origin,ray=sidearm.hipRay(seat)
		cx,cy=hud.project(origin+ray*25,id)
		if cx and cx>x+4*s and cx<x+w-4*s and cy>y+4*s and cy<y+h-4*s then
			local radius=(1.4+9*(gun.shotFlash or 0))*s
			hud.box(cx-radius-s,cy-radius-s,(radius+s)*2,(radius+s)*2,"#18202898")
			hud.box(cx-radius,cy-radius,radius*2,radius*2,ink)
		end
	end
	if gun.hitPoint then
		local hx,hy=hud.project(gun.hitPoint,id)
		if hx and hx>x+14*s and hx<x+w-14*s and hy>y+14*s and hy<y+h-14*s then
			for _,dx in {-1,1} do
				for _,dy in {-1,1} do
					for k=5,9,2 do hud.box(hx+(dx*k-1)*s,hy+(dy*k-1)*s,2.5*s,2.5*s,"#ffe0bd") end
				end
			end
		end
	end
end

function instruments.draw(cityState, fleet, clock, job, player, story, menu, titles, weapon)
	local id = player.id
	local seat = fleet.players[id]
	if not seat then return end
	local goal = if story then (if story.target then {pos=story.target} else nil) elseif job.pause == 0 then dispatch.stop(job, fleet) else nil
	local x,y,w,h = ui.area(id)
	-- Fit labels to the individual viewport, including vertically stacked splits.
	local s = math.max(.49, math.min(h / 610, w / 810, 1.7))
	if seat.weaponWheelOpen and menu.owner~=id then wheel.draw(seat,x,y,w,h,s); return end
	local margin = 15*s
	local left, top = x + margin, y + margin
	local width = math.min(w*0.61, 318*s)
	if story then
		objective(story,left,top,math.min(w*.61,420*s),s)
	else
		hud.box(left,top,width,63*s,panel)
		hud.box(left,top,3*s,63*s,gold)
		text(if job.pause > 0 then "DELIVERY COMPLETE" else string.upper(dispatch.title(job)) .. "   " .. job.leg .. "/3",left+12*s,top+7*s,15*s,gold)
		text(if job.pause > 0 then "+$750  |  Crew $" .. job.cash else verbs[goal.kind] .. goal.name,left+12*s,top+29*s,19*s,ink)
	end
	local hour = math.floor(clock.hour)
	local label = string.format("%02d:%02d", hour, math.floor(clock.hour % 1 * 60))
	local district = if seat.building then venue[cityState.buildings[seat.building].kind] else city.district(seat.pos.x,seat.pos.z)
	local dw = hud.measure(district,19*s)
	local tt = x+w-margin
	hud.box(tt-math.max(dw+22*s,99*s),top,math.max(dw+22*s,99*s),61*s,panel)
	text(label,tt-65*s,top+6*s,18*s,aqua)
	text(district,tt-dw-10*s,top+30*s,19*s,ink)
	local size = 117*s
	local mx,my = left,y+h-size-margin
	minimap(fleet,id,goal,mx,my,size)
	local mapCaption = if goal then tostring(math.floor(vector.magnitude(seat.pos-goal.pos)+.5)).." m" else ""
	hud.box(mx,my-19*s,size,18*s,panel)
	text("N",mx+4*s,my-19*s,15*s,muted)
	text(mapCaption,mx+size-hud.measure(mapCaption,15*s)-3*s,my-19*s,15*s,gold)
	local action
	local mode
	if seat.mode == "walk" then
		mode = "ON FOOT"
		action = if seat.nearby then "F / Y   Take " .. (if seat.nearby == "car" then "car" else "helicopter") elseif seat.doorway then "Walk through the doorway" else "WASD / stick   Move"
	else
		mode = if seat.mode == "drive" then string.format("%d km/h", math.floor(math.abs(seat.speed)*3.6+.5)) else string.format("%d m ALT", math.floor(seat.pos.y+.5))
		action = if seat.mode == "fly" then "Space / A   Rise     Shift / RB   Lower" else "W / RT   Gas     S / LT   Brake"
		if seat.mode == "drive" and math.abs(seat.speed) < 2 then action = "W / RT   Gas     F / Y   Exit" end
	end
	local armed = seat.mode == "walk" and not seat.impact and weapon ~= nil
	local gunHint = "RMB / LT   Aim     LMB / RT   Fire"
	if armed then
		mode = string.upper(wheel.name(seat))
		if weapon.ammo~=nil and weapon.magazine then
			mode ..= string.format("   %d / %d",weapon.ammo,weapon.magazine)
		end
		if (weapon.reloadRemaining or 0)>0 then
			gunHint=string.format("Reloading   %.1f s",weapon.reloadRemaining)
		end
	end
	if armed and menu.owner~=id then reticle(weapon,seat,id,x,y,w,h,s) end
	text("R / X   Stories",mx+2*s,my-39*s,14*s,ink)
	if armed then text("Hold Q / LB   Weapons",mx+2*s,my-57*s,14*s,ink) end
	local aw = math.max(hud.measure(action,16*s), hud.measure(mode,23*s), if armed then hud.measure(gunHint,15*s) else 0) + 24*s
	local height = (if armed then 80 else 58)*s
	local ax, ay = tt-aw, y+h-margin-height
	hud.box(ax,ay,aw,height,panel)
	text(mode,ax+12*s,ay+3*s,23*s,ink)
	text(action,ax+12*s,ay+34*s,16*s,muted)
	if armed then text(gunHint,ax+12*s,ay+55*s,15*s,muted) end
	if menu.owner==id then drawStories(menu,titles,x,y,w,h,s) end
end

return instruments
