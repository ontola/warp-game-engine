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
	dot(goal.pos, size*.077, gold)
	dot(goal.pos, size*.033, panel)
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
function instruments.draw(cityState, fleet, clock, job, player)
	local id = player.id
	local seat = fleet.players[id]
	if not seat then return end
	local goal = dispatch.stop(job, fleet)
	local x,y,w,h = ui.area(id)
	-- Fit labels to the individual viewport, including vertically stacked splits.
	local s = math.max(.49, math.min(h / 610, w / 810, 1.7))
	local margin = 15*s
	local left, top = x + margin, y + margin
	local width = math.min(w*0.61, 318*s)
	hud.box(left,top,width,63*s,panel)
	hud.box(left,top,3*s,63*s,gold)
	text(if job.pause > 0 then "DELIVERY COMPLETE" else string.upper(dispatch.title(job)) .. "   " .. job.leg .. "/3",left+12*s,top+7*s,15*s,gold)
	text(if job.pause > 0 then "+$750  |  Crew $" .. job.cash else verbs[goal.kind] .. goal.name,left+12*s,top+29*s,19*s,ink)
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
	local dist = math.floor(vector.magnitude(seat.pos-goal.pos)+.5)
	local mapCaption = tostring(dist).." m"
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
	local aw = math.max(hud.measure(action,16*s), hud.measure(mode,23*s)) + 24*s
	local ax, ay = tt-aw, y+h-margin-58*s
	hud.box(ax,ay,aw,58*s,panel)
	text(mode,ax+12*s,ay+3*s,23*s,ink)
	text(action,ax+12*s,ay+34*s,16*s,muted)
end

return instruments
