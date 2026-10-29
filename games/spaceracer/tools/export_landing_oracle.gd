extends SceneTree
const Race=preload("res://src/race.gd")
const Track=preload("res://src/track.gd")
func _initialize()->void: call_deferred("run")
func vec(v:Vector3)->Array: return [v.x,v.y,v.z]
func mat(v:Basis)->Array: return [vec(v.x),vec(v.y),vec(v.z)]
func run()->void:
    var cases:=[]
    for name in ["smooth","hard","tilted","inverted","underside"]:
        var r:=Race.new([{"slot":0,"name":"A","bot":false}],31,3,"easy","city")
        var p:Dictionary=r.racers[0];p.distance=1000.;p.airborne=true
        var n:Dictionary=r.track.sample(p.distance)
        p.air_position=Track.point(n,0.,-10. if name=="underside" else 18.)
        p.air_frame=n.frame
        if name=="tilted": p.air_frame=p.air_frame*Basis(Vector3.BACK,.5)
        if name=="inverted": p.air_frame=p.air_frame*Basis(Vector3.BACK,PI)
        p.air_velocity=n.frame*Vector3(0.,50. if name=="underside" else (-100. if name=="hard" else -20.),160.)
        p.air_entry_speed=p.air_velocity.length()
        var initial:={"position":vec(p.air_position),"frame":mat(p.air_frame),"velocity":vec(p.air_velocity)}
        var frames:=[]
        for tick in range(600):
            Race.Flight.step(p,r.track,1./120.,0.,0.,1.,0.)
            frames.append({"position":vec(p.air_position),"velocity":vec(p.air_velocity),"airborne":p.airborne,"crashed":p.crashed,"distance":p.distance,"x":p.x,"speed":p.speed,"energy":p.energy,"lift":p.lift})
            if not p.airborne: break
        cases.append({"name":name,"initial":initial,"frames":frames})
    var f:=FileAccess.open(OS.get_cmdline_user_args()[0],FileAccess.WRITE);f.store_string(JSON.stringify(cases));f.close();print("LANDING_ORACLE_OK");quit()
