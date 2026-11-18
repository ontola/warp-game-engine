-- Resource icons drawn with hud boxes (pixel art, 8 by 8), and text that
-- shows the same icon and colour wherever it names a resource: the bar at
-- the top, the talk cards, the popups. One look for food, ammo, first aid
-- and batteries everywhere.

local M = {}

-- What each kind is called in text, longest first so "first aid kit" wins
-- over "first aid".
local WORDS = {
	{ "first aid kits", "meds" }, { "first aid kit", "meds" }, { "first aid", "meds" }, { "medicine", "meds" }, { "meds", "meds" },
	{ "batteries", "battery" }, { "battery", "battery" },
	{ "rounds", "ammo" }, { "round", "ammo" }, { "ammo", "ammo" }, { "bullets", "ammo" },
	{ "food", "food" },
}

M.COLORS = { ammo = "#e0b84a", food = "#7fc25a", battery = "#7ab8e8", meds = "#f2f2f2" }

-- Palettes: a main, d dark, l light, r red, g grey rim, w white.
local PALETTE = {
	ammo = { a = "#e0b84a", d = "#8a6a24", l = "#c8743a" },
	food = { a = "#7fc25a", d = "#3f6a2c", g = "#9aa0a6", w = "#d8dde2" },
	meds = { w = "#f2f2f2", r = "#d8382e", g = "#8a8f96" },
	battery = { a = "#7ab8e8", d = "#2f5a80", g = "#c8ccd0", w = "#e8f4ff" },
}

local PIXELS = {
	-- Two cartridges: copper tips, brass cases, dark rims.
	ammo = {
		".l...l..",
		"lal.lal.",
		"aaa.aaa.",
		"ddd.ddd.",
		"aaa.aaa.",
		"aaa.aaa.",
		"aaa.aaa.",
		"ddd.ddd.",
	},
	-- A tin can with a green label.
	food = {
		".gggggg.",
		"gwwwwwwg",
		"aaaaaaaa",
		"adddddda",
		"adddddda",
		"aaaaaaaa",
		"gwwwwwwg",
		".gggggg.",
	},
	-- A first aid box: handle, white case, red cross.
	meds = {
		"..gggg..",
		"..g..g..",
		"wwwwwwww",
		"wwwrrwww",
		"wrrrrrrw",
		"wwwrrwww",
		"wwwwwwww",
		"gggggggg",
	},
	-- A battery cell: terminal, shine, charge.
	battery = {
		"...gg...",
		".aaaaaa.",
		".awaaaa.",
		".awaaaa.",
		".aaaaaa.",
		".dddddd.",
		".dddddd.",
		".dddddd.",
	},
}

-- The icon of kind at x, y (top left), size pixels square. alpha 0 to 1.
function M.draw(kind, x, y, size, alpha)
	local rows, pal = PIXELS[kind], PALETTE[kind]
	if not rows then return end
	local p = size / 8
	for j, row in rows do
		local i = 1
		while i <= 8 do
			local ch = row:sub(i, i)
			local n = 1
			while i + n <= 8 and row:sub(i + n, i + n) == ch do n += 1 end
			if ch ~= "." and pal[ch] then
				local c = pal[ch]
				if alpha and alpha < 1 then
					local r, g, b = tonumber(c:sub(2, 3), 16), tonumber(c:sub(4, 5), 16), tonumber(c:sub(6, 7), 16)
					c = { r / 255, g / 255, b / 255, alpha }
				end
				hud.box(x + (i - 1) * p, y + (j - 1) * p, n * p + 0.5, p + 0.5, c)
			end
			i += n
		end
	end
end

-- Text cut into plain runs and resource words: { text, kind? }.
function M.split(text)
	local out, lower, i, plain = {}, string.lower(text), 1, ""
	while i <= #text do
		local hit = nil
		-- Only at a word start.
		local prev = if i > 1 then lower:sub(i - 1, i - 1) else " "
		if not prev:match("%a") then
			for _, w in WORDS do
				local word = w[1]
				if lower:sub(i, i + #word - 1) == word then
					local after = lower:sub(i + #word, i + #word)
					if not after:match("%a") then
						hit = w
						break
					end
				end
			end
		end
		if hit then
			if plain ~= "" then table.insert(out, { plain }) plain = "" end
			table.insert(out, { text:sub(i, i + #hit[1] - 1), hit[2] })
			i += #hit[1]
		else
			plain ..= text:sub(i, i)
			i += 1
		end
	end
	if plain ~= "" then table.insert(out, { plain }) end
	return out
end

-- How wide text is in the HUD font, in pixels. Spaces at the ends count
-- (measure may trim them).
local function width(text, size)
	local core = text:match("^%s*(.-)%s*$")
	local lead = #text:match("^%s*")
	local trail = if core == "" then 0 else #text:match("%s*$")
	local w = if core ~= "" then (hud.measure(core, size)) else 0
	return w + (lead + trail) * size * 0.25
end

-- hud.text that puts the icon before each resource word and colours the
-- word. opts as hud.text (size, color, align, width), plus dim = true to
-- leave the words in color (a choice you cannot afford); text naming no
-- resource is drawn by hud.text unchanged, wrapping and all.
function M.text(text, x, y, opts)
	local parts = M.split(text)
	local any = false
	for _, part in parts do
		if part[2] then any = true end
	end
	if not any then
		hud.text(text, x, y, { size = opts.size, color = opts.color, align = opts.align, width = opts.width })
		return
	end
	local size = opts.size or 20
	local isz = size * 0.8
	local total = 0
	for _, part in parts do
		total += width(part[1], size) + (if part[2] then isz + size * 0.2 else 0)
	end
	local cx = x
	if opts.align == "center" and opts.width then cx = x + (opts.width - total) / 2 end
	if opts.align == "right" and opts.width then cx = x + opts.width - total end
	for _, part in parts do
		if part[2] then
			M.draw(part[2], cx, y + size * 0.18, isz)
			cx += isz + size * 0.2
		end
		local lead = #part[1]:match("^%s*") * size * 0.25
		local color = if part[2] and opts.dim ~= true then M.COLORS[part[2]] else opts.color
		hud.text(part[1]:match("^%s*(.-)%s*$"), cx + lead, y, { size = size, color = color, width = width(part[1], size) + size * 2 })
		cx += width(part[1], size)
	end
end

return M
