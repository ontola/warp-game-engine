-- F12 Explosions. Each numeric knob becomes a slider. Its comment gives the label and range.
-- @knobs
local K = {
    fireball_size = 1.0, -- Fireball size, 0.1 to 3
    dust_reach = 1.0, -- Ground dust reach, 0 to 3
    fragment_impulse = 1.0, -- Fragment impulse, 0 to 3
    flash_green = 0.70, -- Flash green, 0 to 1
    flash_blue = 0.20, -- Flash blue, 0 to 1
}
-- @end

-- Add a knob above and use it in any expression below to try a new control live.
return { blast_look = {
    size = K.fireball_size,
    dust = K.dust_reach,
    impulse = K.fragment_impulse,
    green = K.flash_green,
    blue = K.flash_blue,
} }
