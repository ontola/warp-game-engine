-- kit/ui: HUD pieces that look the same in every game and scale with the
-- screen. Call them from draw(). Sizes are in "points": pixels on a 720p
-- output, scaled up on bigger screens. `player` puts a piece in that
-- person's split-screen view; nil uses the whole screen.
-- `local ui = require("kit/ui")`.
--
--   ui.score(p.id, p.name, score, p.color)        -- top-left of their view
--   ui.bar(p.id, "Health", hp / 100, { color = "#e0484f" })
--   ui.banner("Round 2", { sub = "Get ready" })
--   ui.scoreboard(rows, { title = "Results" })     -- rows: { name, score, color }
--   ui.marker(pos, "Flag", { player = p.id })      -- a label over a world point
--   ui.hint("Hold A to jump higher")
--   ui.crosshair(p.id)                             -- first-person games
--   ui.flash(p.id, { 1, 0, 0, 0.3 })               -- a hit

local util = require("kit/util")

local ui = {}

local INK = "#ffffff"
local PANEL = { 0.04, 0.05, 0.08, 0.62 }
local SHADOW = { 0, 0, 0, 0.55 }

-- Pixels per point (720p output: 1).
function ui.scale(): number
	local _, h = hud.size()
	return h / 720
end

-- The area of a player's view (or the screen) in pixels: x, y, w, h.
function ui.area(player: number?): (number, number, number, number)
	if player then return hud.view(player) end
	local w, h = hud.size()
	return 0, 0, w, h
end

-- Text with a soft drop shadow. size in points; align left, center, right
-- within width (points) from x.
function ui.text(text: any, x: number, y: number, opts: { size: number?, color: any?, align: string?, width: number?, shadow: boolean? }?)
	opts = opts or {}
	local s = ui.scale()
	local size = (opts.size or 24) * s
	local o = { size = size, align = opts.align, width = if opts.width then opts.width * s else nil }
	if opts.shadow ~= false then
		o.color = SHADOW
		hud.text(text, x + 2 * s, y + 2 * s, o)
	end
	o.color = opts.color or INK
	hud.text(text, x, y, o)
end

-- A rounded-looking panel (a box with a thin lighter rim). Pixels.
function ui.panel(x: number, y: number, w: number, h: number, color: any?)
	local s = ui.scale()
	hud.box(x - s, y - s, w + 2 * s, h + 2 * s, { 1, 1, 1, 0.08 })
	hud.box(x, y, w, h, color or PANEL)
end

-- Name and score in the top-left of a player's view, with their colour.
function ui.score(player: number?, name: string, score: any, color: any?)
	local x, y = ui.area(player)
	local s = ui.scale()
	local label = name .. "   " .. tostring(score)
	local tw = hud.measure(label, 30 * s)
	ui.panel(x + 16 * s, y + 14 * s, tw + 40 * s, 50 * s)
	hud.box(x + 16 * s, y + 14 * s, 6 * s, 50 * s, if color then util.rgba(color, 1) else INK)
	ui.text(label, x + 34 * s, y + 22 * s, { size = 30 })
end

-- A labelled bar in a player's view (or the screen), stacked under the
-- score: frac 0..1. opts: color, row (0, 1, 2 ... from the top), width.
function ui.bar(player: number?, label: string, frac: number, opts: { color: any?, row: number?, width: number? }?)
	opts = opts or {}
	local x, y = ui.area(player)
	local s = ui.scale()
	local w = (opts.width or 220) * s
	local top = y + (76 + (opts.row or 0) * 34) * s
	ui.panel(x + 16 * s, top, w + 16 * s, 28 * s)
	hud.box(x + 24 * s, top + 16 * s, w, 6 * s, { 1, 1, 1, 0.15 })
	hud.box(x + 24 * s, top + 16 * s, w * math.clamp(frac, 0, 1), 6 * s, opts.color or "#7fd8ff")
	ui.text(label, x + 24 * s, top + 1 * s, { size = 14, shadow = false })
end

-- Big centred text (a round start, a winner), with an optional line under
-- it. opts: sub, player, size (points, 72), color, y (0..1 of the area, 0.38).
function ui.banner(text: string, opts: { sub: string?, player: number?, size: number?, color: any?, y: number? }?)
	opts = opts or {}
	local x, y, w, h = ui.area(opts.player)
	local s = ui.scale()
	local size = opts.size or 72
	local top = y + h * (opts.y or 0.38) - size * s / 2
	ui.text(text, x, top, { size = size, align = "center", width = w / s, color = opts.color })
	if opts.sub then
		ui.text(opts.sub, x, top + size * s * 1.15, { size = size * 0.4, align = "center", width = w / s })
	end
end

-- A line of help at the bottom centre.
function ui.hint(text: string, player: number?)
	local x, y, w, h = ui.area(player)
	local s = ui.scale()
	local tw = hud.measure(text, 20 * s)
	ui.panel(x + (w - tw) / 2 - 14 * s, y + h - 52 * s, tw + 28 * s, 34 * s)
	ui.text(text, x, y + h - 46 * s, { size = 20, align = "center", width = w / s, shadow = false })
end

-- A centred table of rows { name, score, color? } (sorted as given).
-- opts: title, player.
function ui.scoreboard(rows: { { name: string, score: any, color: any? } }, opts: { title: string?, player: number? }?)
	opts = opts or {}
	local x, y, w, h = ui.area(opts.player)
	local s = ui.scale()
	local pw = 460 * s
	local ph = (90 + #rows * 46) * s
	local px, py = x + (w - pw) / 2, y + (h - ph) / 2
	ui.panel(px, py, pw, ph)
	ui.text(opts.title or "Results", px, py + 16 * s, { size = 36, align = "center", width = pw / s })
	for i, r in rows do
		local ry = py + (60 + i * 46) * s
		hud.box(px + 24 * s, ry + 4 * s, 8 * s, 30 * s, if r.color then util.rgba(r.color, 1) else INK)
		ui.text(tostring(i) .. ".  " .. r.name, px + 44 * s, ry, { size = 28 })
		ui.text(tostring(r.score), px, ry, { size = 28, align = "right", width = (pw - 28 * s) / s })
	end
end

-- A label over a world point (a name tag, an objective). Hidden behind
-- the camera. opts: player (the view; default the first), color, size, lift
-- (metres above pos, 0).
function ui.marker(pos: vector, text: string, opts: { player: number?, color: any?, size: number?, lift: number? }?)
	opts = opts or {}
	local px, py = hud.project(pos + vector.create(0, opts.lift or 0, 0), opts.player)
	if not px then return end
	local s = ui.scale()
	local size = opts.size or 20
	local tw = hud.measure(text, size * s)
	ui.text(text, px - tw / 2, py - size * s, { size = size, color = opts.color })
end

-- A crosshair in the middle of a view; opts: color, size (points, 10),
-- spread (points, widens it after a shot).
function ui.crosshair(player: number?, opts: { color: any?, size: number?, spread: number? }?)
	opts = opts or {}
	local x, y, w, h = ui.area(player)
	local s = ui.scale()
	local cx, cy = x + w / 2, y + h / 2
	local len, gap, t = (opts.size or 10) * s, (4 + (opts.spread or 0)) * s, 2 * s
	local c = opts.color or { 1, 1, 1, 0.9 }
	hud.box(cx - t / 2, cy - gap - len, t, len, c)
	hud.box(cx - t / 2, cy + gap, t, len, c)
	hud.box(cx - gap - len, cy - t / 2, len, t, c)
	hud.box(cx + gap, cy - t / 2, len, t, c)
end

-- Tint a whole view (a hit flash, low health): color with its alpha.
function ui.flash(player: number?, color: any)
	local x, y, w, h = ui.area(player)
	hud.box(x, y, w, h, color)
end

-- Seconds as a big centred count ("3", "2", "1", "GO!").
function ui.countdown(seconds_left: number, player: number?)
	local n = math.ceil(seconds_left)
	ui.banner(if n > 0 then tostring(n) else "GO!", { player = player, size = 110 })
end

-- A clock "1:05" at the top centre.
function ui.clock(seconds: number, player: number?)
	local x, y, w = ui.area(player)
	local s = ui.scale()
	local t = math.max(0, math.ceil(seconds))
	local label = string.format("%d:%02d", t // 60, t % 60)
	local tw = hud.measure(label, 34 * s)
	ui.panel(x + (w - tw) / 2 - 16 * s, y + 14 * s, tw + 32 * s, 48 * s)
	ui.text(label, x, y + 20 * s, { size = 34, align = "center", width = w / s })
end

return ui
