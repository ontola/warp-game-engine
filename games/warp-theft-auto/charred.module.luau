-- All four paints use one scorched shell. The glTF descriptors reference the
-- original geometry so part names, dent influence and wheel pivots still match.
local BASE = "assets/cars/"
local charred = {body = BASE .. "wreck.gltf"}

function charred.wheel(x, burnt)
	local side = if x < 0 then "left" else "right"
	return BASE .. (if burnt then "wreck-wheel-" else "wheel-") .. side .. ".gltf"
end

function charred.piece(id)
	local shape = world.get(id).shape
	local suffix = string.match(shape, "(#part%-.+)$")
	local target
	if suffix then
		target = charred.body .. suffix
	elseif shape == charred.wheel(-1, false) then
		target = charred.wheel(-1, true)
	elseif shape == charred.wheel(1, false) then
		target = charred.wheel(1, true)
	end
	if target and target ~= shape then world.set(id, {shape = target}) end
end

function charred.apply(car)
	if car.charred then return end
	car.charred, car.model = true, charred.body
	world.set(car.body, {shape = charred.body})
	for _, wheel in car.wheels do charred.piece(wheel) end

	-- Changing the mesh releases its old detailed dent slot. Keep missing
	-- panels off the replacement even when it is using the cheap distant shell.
	local mask = car.missingParts or 0
	if car.dentActive and world.dent(car.body, car.damage.dents or {}, mask) then
		car.damageDirty, car.shellMask = false, nil
	else
		world.dent(car.body, nil, mask)
		car.dentActive, car.damageDirty, car.shellMask = nil, true, mask
	end
end

return charred
