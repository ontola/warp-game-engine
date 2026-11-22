-- kit/round: the flow of a couch match: a countdown, play (to a score or
-- for a time), results, then the next game under Splitscreen Club.
-- `local round = require("kit/round")`.
--
--   state.round = round.new({ win_score = 10, length = 120 })   -- in init
--   state.round:tick(dt)                     -- every tick, first
--   if state.round:playing() then ... end    -- only move things while playing
--   state.round:add(p.id, 1)                 -- a point; may end the round
--   state.round:draw()                       -- in draw: countdown, clock, results
--
-- Scores are per player id. The round ends at win_score, when `length`
-- seconds run out, or when the game calls :finish(winner_id). Results
-- show for `results` seconds, then game.finish() hands back to the party
-- (without a party host New round in the start menu plays again).

local ui = require("kit/ui")

local round = {}

local Round = {}
Round.__index = Round

function round.new(opts: { countdown: number?, length: number?, win_score: number?, results: number?, title: string? }?)
	opts = opts or {}
	local r = setmetatable({
		phase = "countdown", -- then "playing", "results", "done"
		left = opts.countdown or 3,
		countdown = opts.countdown or 3,
		length = opts.length, -- seconds of play; nil: until win_score
		win_score = opts.win_score, -- nil: play for length
		results = opts.results or 6,
		time = 0, -- seconds played
		scores = {},
		winners = {},
		title = opts.title or game.title(),
	}, Round)
	for _, p in players.list() do r.scores[tostring(p.id)] = 0 end
	if r.countdown <= 0 then r.phase = "playing" end
	return r
end

function Round:playing(): boolean return self.phase == "playing" end
function Round:over(): boolean return self.phase == "results" or self.phase == "done" end

-- Seconds of play left, or nil without a time limit.
function Round:time_left(): number?
	if not self.length then return nil end
	return math.max(0, self.length - self.time)
end

function Round:score(id: number): number return self.scores[tostring(id)] or 0 end

-- Add points to a player (negative takes away). Reaching win_score ends the round.
function Round:add(id: number, points: number?)
	if self.phase ~= "playing" then return end
	local k = tostring(id)
	self.scores[k] = (self.scores[k] or 0) + (points or 1)
	if self.win_score and self.scores[k] >= self.win_score then self:finish(id) end
end

function Round:set(id: number, score: number) self.scores[tostring(id)] = score end

-- Players by score, highest first: { {id, name, score, color} }.
function Round:standings()
	local rows = {}
	for _, p in players.list() do
		table.insert(rows, { id = p.id, name = p.name, color = p.color, score = self:score(p.id) })
	end
	table.sort(rows, function(a, b) if a.score ~= b.score then return a.score > b.score end return a.id < b.id end)
	return rows
end

-- End the round now. winner: a player id, a list of ids, or nil for
-- whoever has the most points (ties share the win).
function Round:finish(winner: any?)
	if self:over() then return end
	local ids = {}
	if type(winner) == "number" then ids = { winner }
	elseif type(winner) == "table" then ids = winner
	else
		local rows = self:standings()
		for _, row in rows do
			if row.score == rows[1].score then table.insert(ids, row.id) end
		end
	end
	self.winners = ids
	self.phase = "results"
	self.left = self.results
	local names = {}
	for _, id in ids do
		local p = players.get(id)
		if p then table.insert(names, p.name) end
	end
	if #names > 0 then game.log(table.concat(names, " and ") .. " won " .. self.title) end
end

function Round:winner_names(): string
	local names = {}
	for _, id in self.winners do
		local p = players.get(id)
		if p then table.insert(names, p.name) end
	end
	return table.concat(names, " and ")
end

function Round:tick(dt: number)
	if self.phase == "countdown" then
		self.left -= dt
		if self.left <= 0 then self.phase = "playing" end
	elseif self.phase == "playing" then
		self.time += dt
		if self.length and self.time >= self.length then self:finish() end
	elseif self.phase == "results" then
		self.left -= dt
		if self.left <= 0 then
			self.phase = "done"
			game.finish()
		end
	end
end

-- A player joined mid-game: give them a score.
function Round:join(id: number) if self.scores[tostring(id)] == nil then self.scores[tostring(id)] = 0 end end

-- Countdown, clock and results, over the whole screen.
function Round:draw()
	if self.phase == "countdown" then
		ui.countdown(self.left)
	elseif self.phase == "playing" then
		local t = self:time_left()
		if t then ui.clock(t) end
		if self.time < 0.8 then ui.banner("GO!", { size = 110 }) end
	else
		local rows = self:standings()
		ui.scoreboard(rows, { title = self:winner_names() .. " wins!" })
	end
end

return round
