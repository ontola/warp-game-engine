extends SceneTree
# Development asset extraction, preserving every mesh/control node and shader identity.
# No replacement materials or effect deletion. Warp must implement shader behavior.
const Ship=preload("res://src/ship.gd")
var meshes:Array=[]
var materials:Array=[]
var mesh_ids:Dictionary={}
var material_ids:Dictionary={}
var unsupported:Dictionary={}
func serial(v:Variant)->Variant:
    if v is Vector2: return [v.x,v.y]
    if v is Vector3: return [v.x,v.y,v.z]
    if v is Color: return [v.r,v.g,v.b,v.a]
    if v is Basis: return [serial(v.x),serial(v.y),serial(v.z)]
    if v is Transform3D: return {"basis":serial(v.basis),"origin":serial(v.origin)}
    if v is Texture2D: return {"texture":v.resource_path}
    if v is Object: return {"resource_type":v.get_class(),"path":v.resource_path if v is Resource else ""}
    return v
func material_id(m:Material)->int:
    if m==null: return -1
    var id:=m.get_instance_id()
    if material_ids.has(id): return material_ids[id]
    var index:=materials.size();material_ids[id]=index
    var data:={"class":m.get_class(),"priority":m.render_priority}
    if m is ShaderMaterial:
        data.shader=m.shader.resource_path
        data.parameters={}
        for p in m.shader.get_shader_uniform_list(): data.parameters[p.name]=serial(m.get_shader_parameter(p.name))
    elif m is BaseMaterial3D:
        for key in ["albedo_color","metallic","roughness","transparency","blend_mode","cull_mode","shading_mode","emission_enabled","emission","emission_energy_multiplier","vertex_color_use_as_albedo","albedo_texture","normal_enabled","normal_texture","normal_scale","clearcoat_enabled","clearcoat","clearcoat_roughness"]:
            data[key]=serial(m.get(key))
    else: unsupported[m.get_class()]=true
    materials.append(data);return index
func mesh_id(m:Mesh)->int:
    var id:=m.get_instance_id()
    if mesh_ids.has(id): return mesh_ids[id]
    var index:=meshes.size();mesh_ids[id]=index
    var surfaces:=[]
    for i in range(m.get_surface_count()):
        var a:=m.surface_get_arrays(i)
        var data:={"material":material_id(m.surface_get_material(i))}
        for entry in [["positions",Mesh.ARRAY_VERTEX],["normals",Mesh.ARRAY_NORMAL],["uv",Mesh.ARRAY_TEX_UV],["uv2",Mesh.ARRAY_TEX_UV2],["colors",Mesh.ARRAY_COLOR],["tangents",Mesh.ARRAY_TANGENT],["indices",Mesh.ARRAY_INDEX]]:
            var values:=[]
            if a[entry[1]]!=null:
                for value in a[entry[1]]: values.append(serial(value))
            data[entry[0]]=values
        surfaces.append(data)
    meshes.append({"surfaces":surfaces});return index
func walk(n:Node)->Dictionary:
    var data:={"name":str(n.name),"class":n.get_class(),"children":[]}
    if n is Node3D: data.transform=serial(n.transform);data.visible=n.visible
    if n is MeshInstance3D and n.mesh!=null:
        data.mesh=mesh_id(n.mesh);data.material=material_id(n.material_override);data.shadows=n.cast_shadow;data.layers=n.layers
        data.instance_parameters={}
        for key in ["marking_style","face_surface","face_side"]:
            var value:Variant=n.get_instance_shader_parameter(key)
            if value!=null: data.instance_parameters[key]=serial(value)
        data.surface_overrides=[]
        for i in range(n.mesh.get_surface_count()): data.surface_overrides.append(material_id(n.get_surface_override_material(i)))
    if n is Light3D:
        for key in ["light_color","light_energy","light_indirect_energy","shadow_enabled"]: data[key]=serial(n.get(key))
        if n is OmniLight3D: data.range=n.omni_range;data.attenuation=n.omni_attenuation
    if n is GPUParticles3D:
        data.amount=n.amount;data.lifetime=n.lifetime;data.emitting=n.emitting
        data.process_material=serial(n.process_material);unsupported["GPUParticles3D runtime"] = true
    for child in n.get_children(): data.children.append(walk(child))
    return data
func _initialize()->void: call_deferred("run")
func run()->void:
    var models:=[]
    for variant in range(Ship.Design.VARIANTS.size()):
        var ship:=Ship.build(Color("bf3334"),variant);ship.name=Ship.Design.VARIANTS[variant].name;root.add_child(ship)
        models.append(walk(ship));root.remove_child(ship);ship.free()
    var path:=OS.get_cmdline_user_args()[0]
    var f:=FileAccess.open(path,FileAccess.WRITE)
    f.store_string(JSON.stringify({"source_commit":"66508c8539f5e4b348c0797b649b67b6d7a43dfc","winding":"clockwise","models":models,"meshes":meshes,"materials":materials,"pending_runtime_types":unsupported.keys()}));f.close()
    print("SHIP_EXPORT_OK variants=",models.size()," meshes=",meshes.size()," materials=",materials.size());quit()
