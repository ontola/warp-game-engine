-- Presentation only. Loaded on the next race; distances in metres.
-- The retained ship meshes all use these same engine and mounting anchors.
return {
    -- A short ignition discharge; the bright jet continues for the whole boost.
    boost_arc_start = 38, boost_arc_sustain = 5, boost_arc_decay = 12,
    boost_arc_duration = .32, boost_arc_width = .20,
    sonic_speed = 343, sonic_reset_speed = 325, vapor_life = .48, vapor_opacity = .18,
    boost_arc_spread = 12, boost_arc_height = 7,
    boost_width = 1.9, boost_radiance = 2.0, engine_glare_pixels = 14,
    warp_world_pixels = 48, warp_view_pixels = 64,
    emp_spark_fade = .7, sentry_spark_life = .11,
    damage_smoke_energy = 22, respawn_light_power = 2100,
    nozzle = {2.45, -.12, -3.65}, reverse = {2.45, -.05, 2.2},
    socket = {0, 1.15, -1.8},
    idle_length = .4, throttle_length = 1.0, acceleration_length = 2,
    boost_length = 3.8, width = 1.25,
    response = 22, brake_response = 30,
    radiance = 2.6, exhaust_color = {.12, .4, 1}, trail_radiance = .065, light_power = 650, boost_light_power = 3400,
    trail_life = .24, trail_interval = .016,
    heat_pixels = .7, boost_heat_pixels = 2.3,
}
