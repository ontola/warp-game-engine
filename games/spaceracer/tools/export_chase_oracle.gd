extends SceneTree
const Chase=preload("res://src/chase.gd")
func _initialize()->void: call_deferred("run")
func vec(v:Vector3)->Array: return [v.x,v.y,v.z]
func run()->void:
    var cases:=[]
    for aspect in [16./9.,32./9.,8./9.]:
        var viewport:=SubViewport.new();viewport.size=Vector2i(int(720*aspect),720);root.add_child(viewport)
        var camera:=Camera3D.new();viewport.add_child(camera)
        var frames:=[]
        for tick in range(600):
            var t:=tick/120.
            var basis:=Basis(Vector3.UP,t*.2)*Basis(Vector3.RIGHT,t*.5)*Basis(Vector3.BACK,t*.3)
            var pose:=Transform3D(basis,Vector3(t*20.,t*3.,t*100.))
            Chase.update(camera,pose,300.+t*20.,tick>100,1./120.,tick==0,50.,true)
            frames.append({"position":vec(camera.position),"up":vec(camera.basis.y),"forward":vec(-camera.basis.z),"fov":camera.fov})
        cases.append({"aspect":aspect,"frames":frames});root.remove_child(viewport);viewport.free()
    var f:=FileAccess.open(OS.get_cmdline_user_args()[0],FileAccess.WRITE);f.store_string(JSON.stringify(cases));f.close();print("CHASE_ORACLE_OK");quit()
