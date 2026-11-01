-- Presentation only. Loaded on the next race; distances in metres.
-- The retained ship meshes all use these same engine and mounting anchors.
return {
    boost_arc_start = 22, boost_arc_sustain = 8, boost_arc_decay = 5,
    boost_arc_spread = 12, boost_arc_height = 7,
    boost_width = 1.9, boost_radiance = 2.0, engine_glare_pixels = 24,
    warp_world_pixels = 48, warp_view_pixels = 64,
    emp_spark_fade = .7, sentry_spark_life = .11,
    damage_smoke_energy = 22, respawn_light_power = 2100,
    nozzle = {2.45, -.12, -3.65}, reverse = {2.45, -.05, 2.2},
    socket = {0, 1.15, -1.8},
    idle_length = .65, throttle_length = 1.4, acceleration_length = 4,
    boost_length = 8.5, width = 1.25,
    response = 22, brake_response = 30,
    radiance = 2.6, light_power = 650, boost_light_power = 3400,
    trail_life = .38, trail_interval = .016,
    heat_pixels = .7, boost_heat_pixels = 2.3,
}
