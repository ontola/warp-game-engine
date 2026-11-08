-- Inputs are wrist sockets in gun-local metres. Forward is -Z, left is -X.
-- Return the support wrist and an extra palm roll in degrees.
return {
    pose = function(magazine, foregrip, extension, seconds)
        local t = math.clamp((extension - 0.18) / (0.92 - 0.18), 0, 1)
        t = t * t * (3 - 2 * t)
        local lift = vector.create(-0.035 * 4 * t * (1 - t), 0, 0)
        return magazine + (foregrip - magazine) * t + lift, 0
    end,
}
