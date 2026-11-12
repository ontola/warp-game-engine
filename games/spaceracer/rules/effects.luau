-- Artistic policy. Native code handles bulk mesh motion, collision and GPU work.
-- When an edit applies: at the next race (read when its visuals are built).
local config={
    camera={distance=16.5,height=4.6,speed_pullback=.004,boost_pullback=1,
        fov_base=72,fov_speed=8,fov_surge=8,fov_boost=12,shake=1.5},
    blur_start_speed=90, blur_full_speed=320, blur_strength=1,
    boost_blur=.3, shutter_seconds=1/75, max_blur_pixels=48,
    baked_explosion_smoke=true,
    crash_radius=12, explosion_light_scale=500, debris_life=4.5, max_pieces=28,
    gravity=28, drag=.55, restitution=.32, inherited_velocity=.18,
    max_inherited_speed=65, flame_seconds=2.2, flame_interval=.09,
}
local function shard(c)
    local function r(n) return (math.sin((c.seed+c.index*71+n)*12.9898)*43758.5453)%1 end
    return {
        velocity={(r(1)-.5)*28,6+r(2)*16,(r(3)-.5)*28},
        axis={r(4)-.5,r(5)-.5,r(6)-.5},spin=(r(7)-.5)*13,
        life=config.debris_life*(.8+r(8)*.4),burn=c.index%5==0,
    }
end
return {config=config,shard=shard}
