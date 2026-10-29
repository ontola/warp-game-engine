extends SceneTree
const Track=preload("res://src/track.gd")
func _initialize()->void: call_deferred("run")
func run()->void:
    var cases:=[]
    for seed_value in range(31,37):
        for difficulty in ["easy","normal","hard"]:
            for biome in ["city","forest","cell","desert","ocean"]:
                var t:=Track.new(seed_value,difficulty,biome)
                var points:=[]
                var raw:=[]
                for i in range(129):
                    var u:=float(i)/128.
                    var p:=t.base_position(u)
                    points.append([p.x,p.y,p.z])
                    p=t.raw_position(u);raw.append([p.x,p.y,p.z])
                cases.append({"seed":seed_value,"difficulty":difficulty,"biome":biome,"radius":t.radius,"phase":t.phase,"points":points,"raw":raw})
    var f:=FileAccess.open(OS.get_cmdline_user_args()[0],FileAccess.WRITE)
    f.store_string(JSON.stringify(cases));f.close();print("GENERATION_ORACLE_OK cases=",cases.size());quit()
