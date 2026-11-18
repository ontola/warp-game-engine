-- Materials: which surface.wgsl kind each of the land's colours is drawn
-- with. Everything in the world is a box or a ball; surface.wgsl gives it
-- asphalt, brick, plaster, wood and so on around its colour. The land and
-- the props spawn and warm through M.spawn and M.warm, so a colour picks
-- its material by itself (pass `surface` to choose another, or false for
-- none).

local M = {}

M.ASPHALT, M.PAVEMENT, M.GRASS, M.DIRT, M.BRICK, M.PLASTER, M.CONCRETE, M.WOOD = 0, 1, 2, 3, 4, 5, 6, 7
M.METAL, M.RUST, M.ROOF, M.FOLIAGE, M.CLOTH, M.RUBBER, M.GLASS, M.PAINT, M.LOGS = 8, 9, 10, 11, 12, 13, 14, 15, 16
M.TILES, M.BARK, M.FLOOR, M.PLAIN = 17, 18, 19, 20
M.INDOOR = 64 -- added to a kind: the same, in the dark of a room

local K = M
-- By colour (lower case).
M.BY_COLOR = {
	-- Roads and paths.
	["#2c2c2e"] = K.ASPHALT, ["#1f1f22"] = K.ASPHALT, ["#3a3a3c"] = K.ASPHALT,
	["#6b5a42"] = K.DIRT, ["#7b6646"] = K.DIRT, ["#4d4433"] = K.DIRT, ["#5d5032"] = K.DIRT,
	["#8c8984"] = K.PAVEMENT, ["#8a857c"] = K.PAVEMENT,
	-- Ground: grass out of town, a dusty lot in the city.
	["#55623f"] = K.GRASS, ["#3f4a2e"] = K.GRASS, ["#6b6a3c"] = K.GRASS, ["#4a4236"] = K.DIRT,
	["#7d8a3a"] = K.FOLIAGE, ["#8f8a40"] = K.FOLIAGE, ["#6a7a32"] = K.FOLIAGE,
	-- Walls: brick in the reds, render elsewhere, logs in the woods.
	["#7a4a3a"] = K.BRICK, ["#8a5a44"] = K.BRICK, ["#7a3a2c"] = K.BRICK, ["#8a4a38"] = K.BRICK,
	["#a8998a"] = K.PLASTER, ["#9aa294"] = K.PLASTER, ["#b0a08a"] = K.PLASTER, ["#8d97a0"] = K.PLASTER, ["#a38c78"] = K.PLASTER,
	["#9a845e"] = K.PLASTER, ["#a89c86"] = K.PLASTER, ["#5a524a"] = K.CONCRETE, ["#6e7a72"] = K.PLASTER,
	["#5f4c3a"] = K.LOGS, ["#6a5640"] = K.LOGS,
	-- Floors.
	["#6a4e36"] = K.WOOD, ["#7a5e42"] = K.WOOD, ["#827868"] = K.CONCRETE, ["#4e3e2e"] = K.WOOD,
	["#6a5440"] = K.WOOD, ["#5a5446"] = K.CONCRETE, ["#7a6a52"] = K.WOOD, ["#5e4e36"] = K.WOOD, ["#6a5c40"] = K.WOOD,
	-- Roofs, rubble, rock.
	["#5a5650"] = K.ROOF, ["#6a665e"] = K.ROOF, ["#4e4a44"] = K.ROOF, ["#625a50"] = K.ROOF,
	["#5f5a55"] = K.CONCRETE, ["#7a7470"] = K.CONCRETE, ["#4a4640"] = K.CONCRETE, ["#3a3632"] = K.CONCRETE,
	-- Cars and wrecks.
	["#6b2b2b"] = K.PAINT, ["#2f4d6b"] = K.PAINT, ["#5d6650"] = K.PAINT, ["#8a8478"] = K.PAINT, ["#3a3a40"] = K.PAINT,
	["#7a5a2a"] = K.PAINT, ["#e8e8f0"] = K.PAINT,
	["#2a2624"] = K.RUST, ["#6a4030"] = K.RUST, ["#5a4a3a"] = K.RUST, ["#4a5048"] = K.RUST, ["#6a5a48"] = K.RUST,
	["#202024"] = K.GLASS, ["#161618"] = K.GLASS, ["#08080a"] = K.GLASS,
	-- Set dressing.
	["#3b2a1e"] = K.WOOD, ["#6b4e37"] = K.WOOD, ["#8b6b4a"] = K.WOOD, ["#5a4632"] = K.WOOD, ["#c9b25a"] = K.CLOTH,
	["#8a7a5a"] = K.CLOTH, ["#4a5a6a"] = K.CLOTH, ["#1d1d1f"] = K.RUBBER, ["#1a1a1a"] = K.RUBBER,
	["#2f4a3a"] = K.METAL, ["#6a3a2a"] = K.RUST, ["#3a3d42"] = K.METAL, ["#7a7d82"] = K.METAL, ["#5f6266"] = K.METAL,
	["#5a5a5e"] = K.METAL,
	-- Buildings: roof tiles, frames, sills and copings, shop fascias.
	["#6a3226"] = K.TILES, ["#5a2e24"] = K.TILES, ["#3a3a3e"] = K.TILES, ["#4a3a30"] = K.TILES,
	["#4a3e32"] = K.BARK, ["#56703a"] = K.FOLIAGE, ["#5e6a30"] = K.FOLIAGE, ["#2a3f24"] = K.FOLIAGE,
	["#9a968e"] = K.CONCRETE, ["#26272a"] = K.METAL, ["#8a2420"] = K.PAINT, ["#2a4a7a"] = K.PAINT, ["#2a4a2a"] = K.GLASS,
	["#6b4a22"] = K.FOLIAGE, ["#5a3e1e"] = K.FOLIAGE, ["#7a5a30"] = K.FOLIAGE,
	["#c8c2b4"] = K.CONCRETE,
	-- Indoors: furniture, fabrics, appliances, tiled floors.
	["#7a6a58"] = K.WOOD, ["#2a2622"] = K.WOOD, ["#c8c8c0"] = K.PAINT, ["#303034"] = K.METAL,
	["#c8c0ae"] = K.CLOTH, ["#d8d2c4"] = K.CLOTH, ["#6a4040"] = K.CLOTH, ["#4a5a40"] = K.CLOTH, ["#5a4a5a"] = K.CLOTH,
	["#5a5040"] = K.CLOTH, ["#6a3a34"] = K.CLOTH, ["#3a4a5a"] = K.CLOTH, ["#5a4a32"] = K.CLOTH, ["#3a5a3a"] = K.PAINT,
	["#a8a49a"] = K.FLOOR, ["#8a8a84"] = K.FLOOR, ["#b0aca2"] = K.FLOOR, ["#6a2a24"] = K.PAINT, ["#2a4a5a"] = K.PAINT, ["#5a5a2a"] = K.PAINT, ["#3a3a3a"] = K.PAINT,
	["#2f4a26"] = K.FOLIAGE, ["#3c5a2c"] = K.FOLIAGE, ["#26401f"] = K.FOLIAGE, ["#4a6a32"] = K.FOLIAGE,
}

-- The options with their material filled in (the table itself, changed).
function M.dress(o)
	if o.surface == nil and type(o.color) == "string" and not o.emissive then
		local k = M.BY_COLOR[string.lower(o.color)]
		if k then o.surface = k end
	end
	if o.surface == false then o.surface = nil end
	-- indoor = true: darker, for what stands in a room (see surface.wgsl).
	if o.indoor then
		if not o.emissive then o.surface = (o.surface or M.PLAIN) % M.INDOOR + M.INDOOR end
		o.indoor = nil
	end
	return o
end

function M.spawn(shape, o, parent)
	return world.spawn(shape, M.dress(o), parent)
end

function M.warm(shape, look)
	local o = {}
	for k, x in look do o[k] = x end
	world.warm(shape, M.dress(o))
end

return M
