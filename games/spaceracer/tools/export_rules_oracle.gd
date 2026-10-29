extends SceneTree
const Race=preload("res://src/race.gd")
const Checkpoints=preload("res://src/checkpoints.gd")
func _initialize()->void: call_deferred("run")
func run()->void:
    var out:=OS.get_cmdline_user_args()[0]
    var cases:=[]
    for name in ["wake","lateral","height","crossing","lapped","blocked","countdown"]:
        var race:=Race.new([{"slot":0,"name":"A","bot":false},{"slot":1,"name":"B","bot":true}],31)
        race.countdown=1. if name=="countdown" else 0.
        var a:Dictionary=race.racers[0];var b:Dictionary=race.racers[1]
        a.distance=300.;b.distance=340.;a.speed=250.;b.speed=240.
        if name=="lateral": a.x=10.
        if name=="height": a.lift=8.
        if name=="crossing": a.heading=PI*.5
        if name=="lapped": b.distance+=race.track.length
        if name=="blocked": b.emp_time=2.
        var frames:=[]
        for tick in range(100):
            Race.Slipstream.update(race,1./120.)
            frames.append([a.slipstream,b.slipstream])
        var poses:=[]
        for p in race.racers:
            var t:Transform3D=Race.Flight.ground_pose(p,race.track.sample(p.distance),race.clock)
            poses.append({"origin":[t.origin.x,t.origin.y,t.origin.z],"basis":[[t.basis.x.x,t.basis.x.y,t.basis.x.z],[t.basis.y.x,t.basis.y.y,t.basis.y.z],[t.basis.z.x,t.basis.z.y,t.basis.z.z]]})
        cases.append({"name":name,"length":race.track.length,"poses":poses,"frames":frames})
    var bump_cases:=[]
    for name in ["left","right","both","held","retrigger","disabled","countdown"]:
        var race:=Race.new([{"slot":0,"name":"A","bot":false}],31)
        var p:Dictionary=race.racers[0]
        if name=="disabled": p.airborne=true
        var frames:=[]
        for tick in range(240):
            var left:bool=name=="left" or name=="both"
            var right:bool=name in ["right","both","held","disabled","countdown"] or (name=="retrigger" and tick%120<30)
            Race.Bump.begin(p,{"left":left,"right":right},1./120.,1. if name=="countdown" else 0.)
            frames.append({"left":left,"right":right,"time":p.bump_time,"cooldown":p.bump_cooldown,"side":p.bump_side,"velocity":Race.Bump.velocity(p)})
        bump_cases.append({"name":name,"frames":frames})
    var f:=FileAccess.open(out,FileAccess.WRITE)
    f.store_string(JSON.stringify({"slipstream":cases,"bump":bump_cases}));f.close()
    print("SPACERACER_RULES_ORACLE_OK");quit()
