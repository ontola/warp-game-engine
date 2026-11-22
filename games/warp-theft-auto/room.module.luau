-- The city keeps only a seed and footprint for each ground floor. This module
-- builds one furnished, walkable floor when a pedestrian approaches a door or window.
local room = {}
local v = vector.create
local finishes = {}
local function finish(color, surface, emissive)
	local look = {color=color, surface=surface, emissive=emissive}
	table.insert(finishes, look)
	return look
end

local STONE = finish("#d8cbb5", 24)
local SLATE = finish("#819390", 24)
local CONCRETE = finish("#acaaa0", 24)
local PLASTER = finish("#e6e0c9", 26)
local CREAM = finish("#eed9c0", 26)
local PALE_GREY = finish("#c4ccc6", 26)
local OAK = finish("#ac764d", 25)
local WALNUT = finish("#81573d", 25)
local CEILING = finish("#eae6d9")
local STEEL = finish("#435c61")
local SCREEN = finish("#386068")
local LINEN = finish("#f1e4c9")
local BRASS = finish("#c28a4b")
local TEAL = finish("#438c87")
local RUSSET = finish("#b85d46")
local INDIGO = finish("#566f88")
local FABRICS = {TEAL, RUSSET, INDIGO}
-- 3,000 nits at the warm diffuser and 3,400 at the cool panel.
local LIGHT = finish("#fff1d4", nil, .75)
local COOL_LIGHT = finish("#e1f4f7", nil, .85)
local PRINTS = {finish("#528982", 28), finish("#cc865e", 28)}
local CRATES = {finish("#c8aa75", 27), finish("#a68e68", 27), finish("#b47c5b", 27)}
local QUAY_SIGN = finish("#32656b", 11)
local GLASS = {color="#c6d6cf", glass=.045, roughness=.055}

-- Warp creates a renderer mesh on first use of a shape/material combination.
-- Register the finite swatches at startup without creating any room entities.
function room.warm(buildings)
	world.warm("pane", GLASS)
	for _, look in finishes do world.warm("cube", look) end
	local known = {}
	for _, b in buildings do
		local key = b.color .. tostring(b.surface)
		if not known[key] then
			world.warm("cube", {color=b.color, surface=b.surface})
			known[key] = true
		end
	end
end

local function sampler(seed)
	local value = 1 + math.floor(seed) % 2147483646
	return function()
		value = (value * 48271) % 2147483647
		return value / 2147483647
	end
end

-- The GPU window preview uses the same mirror, upholstery and wood as the
-- furnished room. A small set of surface kinds shares meshes between lots.
local function style(kind, draw)
	local mirror = if draw()<.5 then -1 else 1
	local mirrorBit = if mirror<0 then 1 else 0
	if kind=="warehouse" then return mirror, nil, nil, 66+3*mirrorBit end
	local fabric = math.floor(draw()*#FABRICS)
	local dark = (draw()<.5) == (kind=="hotel")
	local variant = mirrorBit + 2*fabric + (if dark then 6 else 0)
	local surface = (if kind=="hotel" then 65 else 64) + 3*variant
	return mirror, FABRICS[fabric+1], if dark then WALNUT else OAK, surface
end

function room.proxy_surface(kind, seed)
	local _, _, _, surface = style(kind, sampler(seed))
	return surface
end

-- These dimensions also drive the distant glaze drawn by pm_ground_floor.
local function sideWindow(b)
	if b.kind=="warehouse" then return 7.2, 0, 2.45, 3.58 end
	if b.kind=="hotel" then return 5.6, 1.4, .85, 3.58 end
	return 7.4, 1.4, .72, 3.58
end

local function storefront(b)
	-- Smaller observation windows keep the loading bays beside the warehouse door.
	if b.kind=="warehouse" then return 2.24, 4.78, 1.02, 2.50 end
	local sill = if b.kind=="hotel" then .60 else .47
	return 1.76, b.w/2-.66, sill, 2.50
end

function room.window_distance_sq(b, feet)
	local x, z, eye = math.abs(feet.x-b.x), feet.z-b.z, feet.y+1.5
	local span, center, sill, head = sideWindow(b)
	local dx = x-(b.w/2-.115)
	local dz = math.max(0,math.abs(z-center)-span/2)
	local dy = math.max(sill-eye,0,eye-head)
	local lateral = dx*dx + dy*dy + dz*dz
	local left, right, bottom, top = storefront(b)
	dx, dy, dz = math.max(left-x,0,x-right), math.max(bottom-eye,0,eye-top), z-(b.d/2-.115)
	return math.min(lateral, dx*dx + dy*dy + dz*dz)
end

local function envelope(b, put, pane, wall, floor, lamp)
	local w, d, h = b.w, b.d, b.h
	local thick, lining = .23, .045
	local doorW, doorH = 2.8, 2.5
	local wing = (w - doorW) / 2
	local exterior = {color=b.color, surface=b.surface}
	-- Outside skins match the distant podium. A thinner interior finish lets
	-- the same facade keep its street-facing windows and plastered inner face.
	for _, side in {-1, 1} do
		local x = side * (w - thick) / 2
		local sideD = d-2*thick
		local function sidePanel(y,z,height,depth)
			put(x,y,z,thick,height,depth,exterior)
			put(x-side*(thick+lining)/2,y,z,lining,height,depth,wall,false)
		end
		local span, windowZ, sill, head = sideWindow(b)
		local left, right = windowZ-span/2, windowZ+span/2
		sidePanel(h/2,(left-sideD/2)/2,h,left+sideD/2)
		sidePanel(h/2,(right+sideD/2)/2,h,sideD/2-right)
		sidePanel(sill/2,windowZ,sill,span)
		sidePanel((h+head)/2,windowZ,h-head,span)
		pane(x,(sill+head)/2,windowZ,span,head-sill,side*math.pi/2)
		put(x,(sill+head)/2,windowZ,.14,head-sill,.085,STEEL)
		local frontX = side * (doorW+wing) / 2
		local frontZ = (d-thick)/2
		local inner, outer, base, top = storefront(b)
		local center, width = (inner+outer)/2, outer-inner
		put(frontX,base/2,frontZ,wing,base,thick,exterior)
		put(frontX-side*thick/2,base/2,d/2-thick-lining/2,wing-thick,base,lining,wall,false)
		-- Slender door jamb, masonry corner pier, aluminium transom and stiles.
		put(side*(inner+doorW/2)/2,(top+base)/2,frontZ,inner-doorW/2,top-base,thick,STEEL)
		put(side*(outer+w/2)/2,(top+base)/2,frontZ,w/2-outer,top-base,thick,exterior)
		pane(side*center,(top+base)/2,frontZ,width,top-base,0)
		local divisions = if b.kind=="office" then 3 else 2
		for i=1,divisions-1 do
			put(side*(inner+width*i/divisions),(top+base)/2,frontZ,.068,top-base,.11,STEEL)
		end
		put(side*center,2.17,frontZ,width,.068,.11,STEEL)
	end
	put(0,h/2,(-d+thick)/2,w,h,thick,exterior)
	put(0,h/2,-d/2+thick+lining/2,w-2*thick,h,lining,wall,false)
	put(0,(h+doorH)/2,(d-thick)/2,w,h-doorH,thick,exterior)
	put(0,(h+doorH)/2,d/2-thick-lining/2,w-2*thick,h-doorH,lining,wall,false)
	-- Feet remain on the continuous island collider through the threshold.
	put(0,.012,0,w-2*thick,.024,d-2*thick,floor,false)
	put(0,h-.085,0,w,.17,d,CEILING)
	for _, xSide in {-1,1} do
		for _, zSide in {-1,1} do
			put(xSide*w*.23,h-.215,zSide*d*.22,1.5,.09,.60,lamp,false)
		end
	end
end

local function desk(put,x,z,width,wood)
	put(x,.46,z,width-.22,.88,.94,wood)
	put(x,.955,z,width,.11,1.12,LINEN)
	put(x,1.13,z,.13,.26,.14,STEEL,false)
	put(x,1.40,z,.73,.43,.09,SCREEN,false)
end

local function chair(put,x,z,back,color)
	put(x,.29,z,.58,.54,.58,color)
	put(x,.71,z+back*.23,.58,.55,.12,color)
end

local function sofa(put,x,z,length,color)
	put(x,.32,z,1.03,.62,length,color)
	put(x+.42,.77,z,.19,.72,length,color)
end

local function office(b,put,pane,activity,draw,fabric,wood)
	local w,d = b.w,b.d
	envelope(b,put,pane,PLASTER,SLATE,COOL_LIGHT)
	local receptionZ = d*(.21+.05*draw())
	desk(put,-w*.26,receptionZ,4.8,wood)
	activity("receptionist","reach",-w*.26,receptionZ-.93,0,1)
	activity("visitor","idle",-w*.26,receptionZ+1.0,math.pi,1)
	sofa(put,w*.29,d*.23,4.4,fabric)
	activity("guest","sit",w*.29-.12,d*.23,-math.pi/2)
	-- A low return and a taller side partition enclose the conference area.
	local meetingX, meetingZ = -w*.26,-d*(.23+.04*draw())
	put(-w*.115,1.28,-d*.31,.18,2.56,d*.33,PLASTER)
	put(-w*.36,.76,-d*.10,w*.24,1.52,.18,PLASTER)
	put(meetingX,.45,meetingZ,2.9,.84,.83,STEEL)
	put(meetingX,.93,meetingZ,4.4,.15,1.65,wood)
	for _, side in {-1,1} do
		for _, endOfTable in {-1,1} do
			local seatX,seatZ = meetingX+endOfTable*1.25,meetingZ+side*1.23
			chair(put,seatX,seatZ,side,fabric)
			if side<0 or endOfTable<0 then
				activity("colleague","sit",seatX,seatZ-side*.04,if side>0 then math.pi else 0,2)
			end
		end
	end
	local workX,workZ = w*.25,-d*.20
	desk(put,workX,workZ,3.1,wood)
	chair(put,workX,workZ-.94,-1,fabric)
	activity("clerk","sit",workX,workZ-.90,0,3)
	activity("visitor","wave",workX,workZ+.99,math.pi,3)
	activity("courier","walk",2.2,d*.31,math.pi,nil,2.2,-d*.16)
	activity("colleague","walk",-2.2,-d*.32,0,nil,-2.2,.8)
	put(0,2.08,-d/2+.32,4.8,1.95,.065,PRINTS[1],false)
end

local function hotel(b,put,pane,activity,draw,fabric,wood)
	local w,d = b.w,b.d
	envelope(b,put,pane,CREAM,STONE,LIGHT)
	put(0,.032,0,2.15,.014,d*.89,fabric,false)
	local receptionZ = d*(.20+.06*draw())
	desk(put,-w*.285,receptionZ,w*.26,wood)
	activity("concierge","reach",-w*.285,receptionZ-.94,0,1)
	activity("traveler","idle",-w*.285,receptionZ+1.02,math.pi,1)
	sofa(put,w*.34,d*.21,2.80,fabric)
	activity("guest","sit",w*.34-.12,d*.21+.55,-math.pi/2,2)
	activity("guest","sit",w*.34-.12,d*.21-.56,-math.pi/2,2)
	put(w*.215,.22,d*.21,.41,.40,.68,BRASS)
	put(w*.215,.46,d*.21,.70,.09,.96,wood)
	-- A lounge and a writing desk share the lobby. The bedroom at the back
	-- opens onto the central passage on its inner side.
	local suiteX,bedZ = w*.265,-d*.32
	put(w*.365,1.58,-d*.105,w*.25,3.16,.16,CREAM)
	put(w*.155,1.58,-d*.375,.15,3.16,d*.21,CREAM)
	put(suiteX,.30,bedZ,2.22,.55,3.15,wood)
	put(suiteX,.64,bedZ,2.17,.24,3.08,LINEN)
	put(suiteX,.83,bedZ-1.55,2.31,1.56,.13,wood)
	put(suiteX,.77,bedZ+.65,2.18,.08,.86,fabric)
	for _,side in {-1,1} do
		put(suiteX+side*.49,.82,bedZ-1.03,.81,.18,.48,LINEN,false)
	end
	local workX,workZ = -w*.285,-d*(.24+.06*draw())
	desk(put,workX,workZ,w*.23,wood)
	chair(put,workX,workZ+.94,1,fabric)
	activity("clerk","sit",workX,workZ+.90,math.pi)
	activity("porter","idle",suiteX,-d*.04,math.pi)
	activity("porter","walk",1.02,d*.36,math.pi,nil,1.02,.40)
	activity("traveler","walk",-1.02,-d*.35,0,nil,-1.02,.20)
	put(0,2.17,-d/2+.32,2.60,1.75,.065,PRINTS[2],false)
end

local function warehouse(b,put,pane,activity,draw)
	local w,d = b.w,b.d
	envelope(b,put,pane,PALE_GREY,CONCRETE,COOL_LIGHT)
	local rackD, rackZ = d*.60,-d*.05
	for _, side in {-1,1} do
		local rackX = side*w*(.265+.05*draw())
		local inside = rackX-side*1.73
		activity("picker","reach",inside,rackZ-rackD*.17,side*math.pi/2)
		activity("stocktaker","idle",inside,rackZ+rackD*.20,side*math.pi/2)
		for _, xEnd in {-1,1} do
			for _, zEnd in {-1,1} do
				put(rackX+xEnd*1.07,1.62,rackZ+zEnd*(rackD/2-.09),.15,3.24,.15,STEEL)
			end
		end
		for level = 0,1 do
			local shelfY = .29+1.58*level
			put(rackX,shelfY,rackZ,2.30,.17,rackD,RUSSET)
			for _,bay in {-1,1} do
				local height = .74+.57*draw()
				local z = rackZ+bay*rackD*(.19+.11*draw())
				local color = CRATES[1+math.floor(draw()*#CRATES)]
				put(rackX,shelfY+.085+height/2,z,1.46+.40*draw(),height,rackD*(.19+.10*draw()),color)
			end
		end
		put(side*2.28,.033,0,.12,.016,d*.86,BRASS,false)
	end
	put(-w*.29,.14,d*.37,2.7,.25,1.8,WALNUT)
	put(-w*.29,.88,d*.37,2.35,1.23,1.51,CRATES[1+math.floor(draw()*#CRATES)])
	put(0,2.46,-d/2+.32,3.60,.99,.065,QUAY_SIGN,false)
	activity("loader","reach",-w*.29+1.84,d*.37,-math.pi/2,1)
	activity("driver","wave",-w*.29+3.0,d*.41,-math.pi/2,1)
	activity("supervisor","idle",2.3,-d*.41,0)
	activity("runner","walk",2.15,d*.29,math.pi,nil,2.15,-d*.19)
	activity("checker","walk",-2.15,-d*.36,0,nil,-2.15,.80)
end

function room.build(b)
	local ids, activities = {}, {}
	local draw = sampler(b.seed)
	local mirror, fabric, wood = style(b.kind, draw)
	local function point(x,y,z) return v(b.x+mirror*x,y,b.z+z) end
	local function put(x,y,z,w,h,d,look,solid)
		local blocking = solid ~= false
		local id = world.spawn("cube", {
			pos=point(x,y,z), size=v(w,h,d),
			color=look.color, surface=look.surface, emissive=look.emissive,
			solid=blocking, tag=if blocking then "city" else "interior_detail",
		})
		table.insert(ids,id)
	end
	local function pane(x,y,z,width,height,yaw)
		local id = world.spawn("pane", {
			pos=point(x,y,z), size=v(width,height,1), rot=v(0,mirror*yaw,0),
			color=GLASS.color, roughness=GLASS.roughness, glass=GLASS.glass,
			solid=true, tag="city",
		})
		table.insert(ids,id)
	end
	-- Activity positions share the furniture's seeded rotation and layout. A
	-- seated root is on the floor under the hips; +Z is forward for Warp Humans.
	local function activity(role,pose,x,z,yaw,group,endX,endZ)
		table.insert(activities,{
			role=role, pose=pose, pos=point(x,0,z), yaw=mirror*yaw, group=group,
			to=if endX then point(endX,0,endZ) else nil,
		})
	end
	if b.kind=="office" then office(b,put,pane,activity,draw,fabric,wood)
	elseif b.kind=="hotel" then hotel(b,put,pane,activity,draw,fabric,wood)
	else warehouse(b,put,pane,activity,draw) end
	return ids, activities
end

return room
