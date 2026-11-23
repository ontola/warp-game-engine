local missions = require("missions")
local Cast = require("story_cast")
local passenger = require("story_passenger")
local race = require("story_race")
local contract = require("story_contract")
local board = {warm=missions.warm}
local examples = {passenger, race, contract}
board.titles = {passenger.title, race.title, contract.title, "Harbor dispatch"}

function board.clear(state)
	if state.story then missions.stop(state.story, state.storyCast) end
	state.story, state.storyCast = nil, nil
end

function board.start(state, definition, owner)
	local checked, err = missions.validate(definition)
	if not checked then return nil, err end
	if not owner then
		for _, player in players.list() do if not player.bot then owner = player.id; break end end
	end
	if not owner or not state.fleet.players[owner] then return nil, "Join a local seat before starting a story" end
	local cast = Cast.new(state.fleet, state.population)
	local active, message = missions.start(checked, state.fleet, cast, owner, state.population.avatars)
	if not active then cast:clear(); return nil, message end
	board.clear(state)
	state.storyCast, state.story = cast, active
	state.storyBoard.owner = nil
	return active.title
end

-- A pad or keyboard can browse the same missions as the browser editor.
function board.input(state, input)
	state.storyBoard = state.storyBoard or {choice=1}
	local menu = state.storyBoard
	if menu.owner and not state.fleet.players[menu.owner] then menu.owner = nil end
	for _, player in players.list() do
		local id = player.id
		local keys = input[id] and input[id].pressed
		if player.bot or not keys then continue end
		if not menu.owner then
			if keys.west then menu.owner, menu.message = id, nil; input[id] = {} end
		elseif menu.owner == id then
			if keys.west or keys.east then menu.owner = nil
			elseif keys.up then menu.choice = (menu.choice-2) % #board.titles + 1
			elseif keys.down then menu.choice = menu.choice % #board.titles + 1
			elseif keys.south then
				if examples[menu.choice] then
					local _, err = board.start(state, examples[menu.choice], id)
					menu.message = err
				else
					board.clear(state)
					menu.owner = nil
				end
			end
			input[id] = {}
		end
	end
end

function board.tick(state, dt, input)
	if not state.story then return end
	missions.tick(state.story, state.fleet, state.storyCast, dt, input)
end

return board
