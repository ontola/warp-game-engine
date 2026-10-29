-- The backdrop: a plane under the arena drawn by surface.wgsl (kind 0),
-- tissue cells that drift and breathe. Its own module, so main.luau's live
-- edits never touch it. `world.clear()` (a new game) removes it, so it
-- comes back on the next tick.

-- Brightness of the cells (emissive; surface.wgsl's colours are scaled by it).
local GLOW = 0.0016

local function spawn()
	world.spawn("cube", { pos = vector.create(0, -11.5, 0), size = vector.create(80, 0.1, 50), color = "#000000", emissive = GLOW, surface = 0, solid = false, tag = "backdrop" })
end

local function ensure()
	if #world.find("backdrop") == 0 then spawn() end
end

return {
	init = ensure,
	tick = ensure,
	-- An edit to GLOW shows at once.
	reload = function()
		for _, id in world.find("backdrop") do world.remove(id) end
		spawn()
	end,
}
