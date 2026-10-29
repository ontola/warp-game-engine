extends SceneTree
# Run with Godot 4.5.2 --headless --path SOURCE --script ABSOLUTE_SCRIPT -- OUTDIR.
const Race=preload("res://src/race.gd")
const Track=preload("res://src/track.gd")
class Road extends RefCounted:
    var length:=100000.0
    var width:=500.0
    var bend:=0.0
    var crest:=0.0
    var slope:=0.0
    var zone:=""
    var hazards:=Track.HazardCollision.new()
    var obstacles:RefCounted
    var biome:="city"
    func sample(d:float)->Dictionary:
        return {"p":Vector3(0,0,d),"frame":Basis.IDENTITY,"width":width,"curve":bend,"crest":crest,"slope":slope,"zone":zone,"rails":true}
    func project(p:Vector3,_reference:float,_reach:float)->Dictionary:
        return {"distance":p.z,"lateral":-p.x,"node":sample(p.z)}
func _initialize()->void: call_deferred("run")
func serial(v:Variant)->Variant:
    if v is Vector3: return [v.x,v.y,v.z]
    if v is Basis: return [serial(v.x),serial(v.y),serial(v.z)]
    if v is Transform3D: return {"basis":serial(v.basis),"origin":serial(v.origin)}
    if v is Dictionary:
        var result:={}
        for key in v:
            if not v[key] is Object: result[str(key)]=serial(v[key])
        return result
    if v is Array:
        var result:=[]
        for item in v: result.append(serial(item))
        return result
    if v is float and not is_finite(v): return null
    return v
func run()->void:
    var out:=OS.get_cmdline_user_args()[0]
    DirAccess.make_dir_recursive_absolute(out)
    var cases:=[]
    for name in ["accelerate","brake_slide","strafe","boost","stack_boost","rail","planted","recharge"]:
        var race:=Race.new([{"slot":0,"name":"Oracle","bot":false}],31)
        var road:=Road.new();race.track=road;race.countdown=0.
        var p:Dictionary=race.racers[0]
        if name!="accelerate": p.speed=265.
        if name in ["boost","stack_boost"]: p.lap=2
        if name=="stack_boost": road.zone="boost"
        if name=="rail": road.width=24.
        if name=="recharge": road.zone="repair";p.energy=35.
        var frames:=[]
        for tick in range(600):
            var c:={"throttle":1.0}
            if name=="brake_slide": c.brake=.8 if tick<60 else 0.;c.steer=.5 if tick<180 else -.2
            if name in ["strafe","rail"]: c.strafe=.7 if tick<240 else -.4
            if name in ["boost","stack_boost"]: c.boost=tick<5
            if name=="planted": c.trim=.8
            race.step(1./120.,[c])
            var state:={}
            for key in ["distance","x","speed","heading","slip","energy","boost","trim","slide","slide_hold","lift","lift_speed","unload","acceleration","flash","energy_refill_delay","energy_previous","on_pad","drifting","airborne","crashed"]:
                state[key]=serial(p[key])
            frames.append({"input":c,"state":state})
        cases.append({"name":name,"width":road.width,"zone":road.zone,"frames":frames})
    var file:=FileAccess.open(out.path_join("handling.json"),FileAccess.WRITE)
    file.store_string(JSON.stringify(cases));file.close()
    var tracks:=[]
    for seed_value in range(31,37):
        for difficulty in ["easy","normal","hard"]:
            var track:=Track.new(seed_value,difficulty,"city")
            var samples:=[]
            for d in [-13.,0.,1.25,100.,1000.,track.length-.01,track.length+5.]:
                var n:Dictionary=track.sample(d)
                samples.append({"distance":d,"node":serial(n),"point":serial(Track.point(n,8.,1.3)),"surface":serial(Track.surface_frame(n,8.))})
            tracks.append({"seed":seed_value,"difficulty":difficulty,"length":track.length,"step":track.step,"nodes":serial(track.nodes),"samples":samples})
    file=FileAccess.open(out.path_join("tracks.json"),FileAccess.WRITE);file.store_string(JSON.stringify(tracks));file.close()
    var flights:=[]
    for name in ["coast","thrust","pitch","roll","yaw","stall"]:
        var race:=Race.new([{"slot":0,"name":"Oracle","bot":false}],31)
        var p:Dictionary=race.racers[0]
        p.airborne=true;p.air_position=Vector3(0,500,0);p.air_frame=Basis.IDENTITY
        p.air_velocity=Vector3(0,0,40 if name=="stall" else 200);p.air_entry_speed=p.air_velocity.length()
        var frames:=[]
        for tick in range(300):
            var c:={"throttle":0. if name=="coast" else 1.,"trim":-.4 if name=="pitch" else 0.,"strafe":.5 if name=="roll" else 0.,"steer":.3 if name=="yaw" else 0.}
            p.trim=c.trim;p.air_time+=1./120.
            Race.Flight.integrate_air(p,1./120.,c.strafe,c.steer,c.throttle,0.)
            frames.append({"input":c,"position":serial(p.air_position),"velocity":serial(p.air_velocity),"frame":serial(p.air_frame),"rates":serial(p.air_rates),"speed":p.speed})
        flights.append({"name":name,"frames":frames})
    file=FileAccess.open(out.path_join("flight.json"),FileAccess.WRITE);file.store_string(JSON.stringify(flights));file.close()
    print("SPACERACER_ORACLE_OK cases=",cases.size()," tracks=",tracks.size())
    quit()
