local ui = require("kit/ui")
local city = require("world")
local dispatch = require("dispatch")
local instruments = {}
local ink, muted, panel = "#fff6df", "#b9cbd0", "#101e26ec"
local gold, aqua = "#ffd360", "#6bdfd0"

local function text(label, x, y, size, color)
	hud.text(label, x, y, {size = size, color = color or ink})
end

local function minimap(fleet, id, goal, x, y, size)
	local bound = city.BOUND
	local function coord(n) return (math.clamp(n, -bound, bound) + bound) * size / (2*bound) end
	local function dot(pos, width, color)
		local cx, cy = x + coord(pos.x), y + coord(pos.z)
		hud.box(cx-width/2, cy-width/2, width, width, color)
	end
	hud.box(x-2,y-2,size+4,size+4,"#adc2bca0")
	hud.box(x,y,size,size,"#18363a")
	local line = math.max(1, size * 0.013)
	for n=-3,3 do
		local edge = coord(n * city.STEP)
		hud.box(x+edge-line/2,y,line,size,"#6a8385")
		hud.box(x,y+edge-line/2,size,line,"#6a8385")
	end
	for _, car in fleet.vehicles do
		if not car.owner then dot(car.pos, if car.kind=="helicopter" then size*.046 else size*.025, if car.kind=="helicopter" then aqua else muted) end
	end
	if goal then
		dot(goal.pos, size*.077, gold)
		dot(goal.pos, size*.033, panel)
	end
	for pid, seat in fleet.players do
		if pid ~= id then dot(seat.pos, size*.055, "#ec8a87") end
	end
	local me = fleet.players[id]
	local d = size*.053
	dot(me.pos, d+2, panel)
	dot(me.pos, d, "#ffffff")
	local nose = me.pos + vector.create(-math.sin(me.yaw), 0, -math.cos(me.yaw)) * 11
	dot(nose, d*.52, "#ffffff")
end

local venue = {office = "Office lobby", hotel = "Hotel lobby", warehouse = "Freight warehouse"}
local verbs = {recover = "Take marked car ", drive = "Drive to ", fly = "Land at ", walk = "Meet on foot at "}
local function drawStories(menu, titles, x, y, w, h, s)
	local panelW = math.min(w * .9, 345*s)
	local lh = 30*s
	local left, top = x+(w-panelW)/2, y+(h-232*s)/2
	hud.box(left,top,panelW,232*s,"#11232af8")
	text("STORIES",left+14*s,top+9*s,20*s,gold)
	for i, title in titles do
		local py = top + (18+i*30)*s
		if menu.choice == i then hud.box(left+6*s,py-2*s,panelW-12*s,lh,"#355454") end
		text(title,left+16*s,py,18*s,ink)
	end
	text(menu.message or "↑ ↓ / D-pad   Choose     Space / A   Play",
		left+14*s,top+185*s,14*s,if menu.message then gold else muted)
	text("R / X   Close",left+14*s,top+207*s,14*s,muted)
end

local function objective(story, left, top, width, s)
	local headline = if story.status=="active" then story.title .. "   " .. story.step .. "/" .. story.total else story.title
	if story.raceTime then headline ..= string.format("   %d:%04.1f", math.floor(story.raceTime / 60), story.raceTime % 60) end
	local info = if story.status=="complete" then "Mission complete" elseif story.status=="failed" then "Mission failed" else story.objective or ""
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

local function marker(story, id, x, y, w, h, s)
	if not story.target or story.status~="active" then return end
	local px,py=hud.project(story.target+vector.create(0,1.8,0), id)
	if not px or px<x+12*s or px>x+w-12*s or py<y+15*s or py>y+h-12*s then return end
	hud.box(px-9*s,py-8*s,18*s,3*s,gold)
	hud.box(px-5*s,py-5*s,10*s,3*s,gold)
	hud.box(px-2*s,py-2*s,4*s,3*s,gold)
end

local function pistol(story,x,y,w,h,s)
	local cx,cy=x+w*.5,y+h*.5
	if story.aiming then
		local c = if story.onTarget then "#ff9774" else ink
		for _,d in {-1,1} do
			hud.box(cx+d*8*s-1*s,cy-1*s,3*s,2*s,c)
			hud.box(cx-s,cy+d*8*s-s,2*s,3*s,c)
		end
	end
	-- A city arcade sidearm framed at the bottom of the owner's viewport.
	local px=x+w*(if story.aiming then .56 else .69)
	local py=y+h-((if story.aiming then 59 else 35)+(story.shotFlash or 0)*60)*s
	hud.box(px-6*s,py+15*s,23*s,47*s,"#564b46")
	hud.box(px-9*s,py+3*s,30*s,19*s,"#253139")
	hud.box(px-8*s,py,27*s,12*s,"#79939b")
	hud.box(px-3*s,py-3*s,3*s,4*s,"#b9c5c4")
	if (story.shotFlash or 0)>.02 then
		hud.box(px-2*s,py-18*s,10*s,16*s,"#ffbf50")
		hud.box(px-7*s,py-11*s,19*s,5*s,"#ffda8a")
	end
end

function instruments.draw(cityState, fleet, clock, job, player, story, menu, titles)
	local id = player.id
	local seat = fleet.players[id]
	if not seat then return end
	local goal = if story then (if story.target then {pos=story.target} else nil) else dispatch.stop(job, fleet)
	local x,y,w,h = ui.area(id)
	-- Fit labels to the individual viewport, including vertically stacked splits.
	local s = math.max(.49, math.min(h / 610, w / 810, 1.7))
	local margin = 15*s
	local left, top = x + margin, y + margin
	local width = math.min(w*0.61, 318*s)
	if story then
		objective(story,left,top,math.min(w*.61,420*s),s)
		marker(story,id,x,y,w,h,s)
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
	if story and story.owner == id and seat.mode=="walk" and (story.weapon=="Arcade pistol" or (story.shotFlash or 0)>0) then
		pistol(story,x,y,w,h,s)
		if story.weapon then action = "RMB / LT   Aim     LMB / RT   Fire" end
	end
	text("R / X   Stories",mx+2*s,my-39*s,14*s,ink)
	local aw = math.max(hud.measure(action,16*s), hud.measure(mode,23*s)) + 24*s
	local ax, ay = tt-aw, y+h-margin-58*s
	hud.box(ax,ay,aw,58*s,panel)
	text(mode,ax+12*s,ay+3*s,23*s,ink)
	text(action,ax+12*s,ay+34*s,16*s,muted)
	if menu.owner==id then drawStories(menu,titles,x,y,w,h,s) end
end

return instruments
