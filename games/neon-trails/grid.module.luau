local W, H = 48, 26

local function key(x, y)
	return y * W + x
end

-- A cell's centre in the world: 1 m cells, the arena centred on the origin.
local function at(x, y, height)
	return vector.create(x - (W - 1) / 2, height, y - (H - 1) / 2)
end

return { width = W, height = H, key = key, at = at }
