extends SceneTree
## Offline extraction only. Run in the isolated project staged by
## export-characters.ps1; the game and renderer never load Godot resources.
## Animation retargeting below follows original scripts/character_visual.gd.
## Copyright (c) 2026 GameNight contributors. MIT; see
## ../assets/characters/SOURCE-SOFTWARE-LICENSE.txt for the full source notice.

const MODELS = ["knight", "paladin", "ch10"]
const CLIPS = {"pistol_idle": "Pistol Idle", "pistol_run": "Pistol Run", "pistol_jump": "Pistol Jump"}
const FPS = 30.0
var output_dir: String
var source_frame := Basis.IDENTITY
var source_hips := Basis.IDENTITY

func _init() -> void:
	call_deferred("run")

func require(ok: bool, message: String) -> void:
	if not ok:
		push_error(message)
		quit(1)
		assert(ok, message)

func run() -> void:
	var args := OS.get_cmdline_user_args()
	require(args.size() == 1, "Pass the absolute export directory after --")
	output_dir = args[0]
	DirAccess.make_dir_recursive_absolute(output_dir)
	var knight = load("res://assets/models/knight.fbx").instantiate()
	var knight_skel: Skeleton3D = knight.find_child("Skeleton3D", true, false)
	source_frame = frame_in(knight, knight_skel)
	source_hips = knight_skel.get_bone_rest(knight_skel.find_bone("Hips")).basis.orthonormalized()
	knight.free()
	for model_name in MODELS:
		export_model(model_name)
	quit()

func find_bone(skel: Skeleton3D, plain: String) -> int:
	var index := skel.find_bone(plain)
	if index >= 0:
		return index
	for i in skel.get_bone_count():
		if skel.get_bone_name(i).ends_with("_" + plain):
			return i
	return -1

func frame_in(model: Node3D, node: Node3D) -> Basis:
	var basis := Basis.IDENTITY
	var n: Node = node
	while n != null and n != model and n is Node3D:
		basis = (n as Node3D).transform.basis * basis
		n = n.get_parent()
	return basis.orthonormalized()

func retarget(anim: Animation, model: Node3D, skel: Skeleton3D) -> void:
	# Match the game's actual track renaming and synthesized hips rotation.
	# In particular do not invent full humanoid retargeting for the benched
	# Avelange model: that character is deliberately absent from the roster.
	var path := str(model.get_path_to(skel))
	var root_keys: Array = []
	for track in range(anim.get_track_count() - 1, -1, -1):
		var old_path := str(anim.track_get_path(track))
		var colon := old_path.find(":")
		if colon < 0:
			continue
		var name := old_path.substr(colon + 1)
		if find_bone(skel, name) >= 0:
			continue
		if name == "root" and anim.track_get_type(track) == Animation.TYPE_ROTATION_3D:
			for key in anim.track_get_key_count(track):
				root_keys.append([anim.track_get_key_time(track, key), anim.track_get_key_value(track, key)])
		anim.remove_track(track)
	for track in anim.get_track_count():
		var old_path := str(anim.track_get_path(track))
		var colon := old_path.find(":")
		if colon < 0:
			continue
		var index := find_bone(skel, old_path.substr(colon + 1))
		if index >= 0:
			anim.track_set_path(track, NodePath(path + ":" + skel.get_bone_name(index)))
	if root_keys.is_empty():
		return
	var hips := find_bone(skel, "Hips")
	require(hips >= 0, "Rig is missing Hips")
	var hips_path := NodePath(path + ":" + skel.get_bone_name(hips))
	for track in anim.get_track_count():
		if anim.track_get_type(track) == Animation.TYPE_ROTATION_3D and anim.track_get_path(track) == hips_path:
			return
	var pre := frame_in(model, skel).inverse() * source_frame
	var track := anim.add_track(Animation.TYPE_ROTATION_3D)
	anim.track_set_path(track, hips_path)
	for key in root_keys:
		var rotation := (pre * Basis(key[1] as Quaternion) * source_hips).get_rotation_quaternion()
		anim.rotation_track_insert_key(track, float(key[0]), rotation)

func v3(value: Vector3) -> Array:
	return [value.x, value.y, value.z]

func rows(value: Transform3D) -> Array:
	return [value.basis.x.x, value.basis.y.x, value.basis.z.x, value.origin.x,
		value.basis.x.y, value.basis.y.y, value.basis.z.y, value.origin.y,
		value.basis.x.z, value.basis.y.z, value.basis.z.z, value.origin.z]

func put_floats(file: FileAccess, values) -> void:
	for value in values:
		file.store_float(value)

func export_model(model_name: String) -> void:
	print("Extracting ", model_name)
	var model: Node3D = load("res://assets/models/" + model_name + ".fbx").instantiate()
	root.add_child(model)
	var skel: Skeleton3D = model.find_child("Skeleton3D", true, false)
	require(skel != null, "Missing skeleton")
	var skeleton_xf := skel.global_transform
	var positions: Array[Vector3] = []
	var normals: Array[Vector3] = []
	var uvs: Array[Vector2] = []
	var vertex_joints: Array[PackedInt32Array] = []
	var vertex_weights: Array[PackedFloat32Array] = []
	var indices := PackedInt32Array()
	var surfaces: Array = []
	var joints: Array = []
	var palette: Array = []
	var palette_keys: Dictionary = {}
	var original_winding_score := 0
	var reverse_winding_score := 0
	for mesh: MeshInstance3D in model.find_children("*", "MeshInstance3D", true, false):
		var mesh_xf := mesh.global_transform
		var mesh_normal := mesh_xf.basis.inverse().transposed()
		var skin: Skin = mesh.skin
		require(skin != null, "Missing skin: " + mesh.name)
		# Indexed by the original surface's skin binding, not Skeleton3D index.
		var remap: Dictionary = {}
		for surface in mesh.mesh.get_surface_count():
			var arrays := mesh.mesh.surface_get_arrays(surface)
			var p: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
			var n: PackedVector3Array = arrays[Mesh.ARRAY_NORMAL]
			var uv: PackedVector2Array = arrays[Mesh.ARRAY_TEX_UV]
			var b: PackedInt32Array = arrays[Mesh.ARRAY_BONES]
			var w: PackedFloat32Array = arrays[Mesh.ARRAY_WEIGHTS]
			var source_indices: PackedInt32Array = arrays[Mesh.ARRAY_INDEX]
			var influences: int = b.size() / p.size()
			require(influences in [4, 8] and n.size() == p.size() and uv.size() == p.size(), "Unexpected mesh attributes")
			var first_vertex := positions.size()
			for vertex in p.size():
				positions.append(mesh_xf * p[vertex])
				normals.append((mesh_normal * n[vertex]).normalized())
				uvs.append(uv[vertex])
				var js := PackedInt32Array()
				var ws := PackedFloat32Array()
				js.resize(8)
				ws.resize(8)
				var total := 0.0
				for influence in influences:
					var weight := w[vertex * influences + influence]
					if weight <= 0.0:
						continue
					var binding := b[vertex * influences + influence]
					if not remap.has(binding):
						var bone := skel.find_bone(skin.get_bind_name(binding)) if not skin.get_bind_name(binding).is_empty() else skin.get_bind_bone(binding)
						require(bone >= 0, "Unmapped skin binding")
						var inverse_bind := skin.get_bind_pose(binding) * mesh_xf.affine_inverse()
						# Deduplicate identical bindings shared by multiple mesh nodes.
						var key := str(bone) + ":" + str(rows(inverse_bind))
						if not palette_keys.has(key):
							palette_keys[key] = palette.size()
							palette.append({"bone": bone, "inverse_bind": inverse_bind})
							joints.append({"name": str(skel.get_bone_name(bone)), "skeleton_index": bone,
								"inverse_bind_rows": rows(inverse_bind), "rest_rows": rows(skeleton_xf * skel.get_bone_global_rest(bone))})
						remap[binding] = palette_keys[key]
					js[influence] = remap[binding]
					ws[influence] = weight
					total += weight
				require(total > 0.99 and total < 1.01, "Invalid weight sum")
				# Imported quantized skin weights can differ minutely from one.
				# Normalize once so matrix interpolation preserves translations.
				for influence in 8:
					ws[influence] /= total
				vertex_joints.append(js)
				vertex_weights.append(ws)
			var first_index := indices.size()
			for triangle in range(0, source_indices.size(), 3):
				var a := first_vertex + source_indices[triangle]
				var bb := first_vertex + source_indices[triangle + 1]
				var c := first_vertex + source_indices[triangle + 2]
				var dot := (positions[bb] - positions[a]).cross(positions[c] - positions[a]).dot(normals[a] + normals[bb] + normals[c])
				if dot > 0.000000001: original_winding_score += 1
				if dot < -0.000000001: reverse_winding_score += 1
				# Godot imported mesh winding is clockwise. Warp is CCW.
				indices.append_array(PackedInt32Array([a, c, bb]))
			var material: BaseMaterial3D = mesh.mesh.surface_get_material(surface)
			require(material != null and material.albedo_texture != null, "Missing source material/texture")
			var texture := material.albedo_texture.resource_path.get_file()
			var source_texture := ProjectSettings.globalize_path(material.albedo_texture.resource_path)
			require(DirAccess.copy_absolute(source_texture, output_dir.path_join(texture)) == OK, "Cannot copy albedo texture")
			var normal_texture := ""
			if material.normal_enabled and material.normal_texture != null:
				normal_texture = material.normal_texture.resource_path.get_file()
				require(DirAccess.copy_absolute(ProjectSettings.globalize_path(material.normal_texture.resource_path), output_dir.path_join(normal_texture)) == OK, "Cannot copy normal texture")
			surfaces.append({"name": str(mesh.name) + "/" + str(surface), "first_vertex": first_vertex, "vertex_count": p.size(),
				"first_index": first_index, "index_count": source_indices.size(), "material": {
				"name": material.resource_name, "albedo_texture": texture,
				"albedo": [material.albedo_color.r, material.albedo_color.g, material.albedo_color.b, material.albedo_color.a],
				"albedo_color_space": "sRGB", "normal_texture": normal_texture, "normal_scale": material.normal_scale,
				"roughness": material.roughness, "metallic": material.metallic,
				"two_sided": material.cull_mode == BaseMaterial3D.CULL_DISABLED, "alpha_mode": "OPAQUE"}})
	require(reverse_winding_score > original_winding_score * 20, "Unexpected source mesh winding")
	require(palette.size() <= 256, "Skin palette exceeds format capacity")
	var mesh_file := model_name + ".mesh.bin"
	var mesh_out := FileAccess.open(output_dir.path_join(mesh_file), FileAccess.WRITE)
	require(mesh_out != null, "Cannot create mesh file")
	for vertex in positions.size():
		put_floats(mesh_out, v3(positions[vertex]))
		put_floats(mesh_out, v3(normals[vertex]))
		put_floats(mesh_out, [uvs[vertex].x, uvs[vertex].y])
		for joint in vertex_joints[vertex]: mesh_out.store_32(joint)
		put_floats(mesh_out, vertex_weights[vertex])
	for index in indices: mesh_out.store_32(index)
	mesh_out.close()
	var anim_file := model_name + ".anim.bin"
	var anim_out := FileAccess.open(output_dir.path_join(anim_file), FileAccess.WRITE)
	var clips: Array = []
	var all_min := Vector3(INF, INF, INF)
	var all_max := Vector3(-INF, -INF, -INF)
	var player := AnimationPlayer.new()
	model.add_child(player)
	player.root_node = player.get_path_to(model)
	player.callback_mode_process = AnimationMixer.ANIMATION_CALLBACK_MODE_PROCESS_MANUAL
	var library := AnimationLibrary.new()
	player.add_animation_library("clips", library)
	for clip_name in CLIPS:
		var imported = load("res://assets/animations/" + CLIPS[clip_name] + ".fbx").instantiate()
		var source_player: AnimationPlayer = imported.find_child("AnimationPlayer", true, false)
		var anim: Animation = source_player.get_animation(source_player.get_animation_list()[0]).duplicate()
		var looping: bool = clip_name != "pistol_jump"
		anim.loop_mode = Animation.LOOP_LINEAR if looping else Animation.LOOP_NONE
		retarget(anim, model, skel)
		library.add_animation(clip_name, anim)
		imported.free()
		skel.reset_bone_poses()
		player.play("clips/" + clip_name)
		var duration: float = anim.length
		var intervals := int(round(duration * FPS))
		var frames := intervals if looping else intervals + 1
		var offset := anim_out.get_position()
		var bounds_min := Vector3(INF, INF, INF)
		var bounds_max := Vector3(-INF, -INF, -INF)
		var socket_frames: Array = []
		for frame in frames:
			var time := duration * float(frame) / float(intervals)
			player.seek(time, true)
			skel.force_update_all_bone_transforms()
			var transforms: Array[Transform3D] = []
			for entry in palette:
				var matrix: Transform3D = skeleton_xf * skel.get_bone_global_pose(entry.bone) * entry.inverse_bind
				transforms.append(matrix)
				put_floats(anim_out, rows(matrix))
			for vertex in positions.size():
				var point := Vector3.ZERO
				for influence in 8:
					var weight := vertex_weights[vertex][influence]
					if weight > 0.0:
						point += (transforms[vertex_joints[vertex][influence]] * positions[vertex]) * weight
				bounds_min = bounds_min.min(point)
				bounds_max = bounds_max.max(point)
			var socket: Dictionary = {}
			for name in ["Head", "RightHand", "Hips"]:
				var bone := find_bone(skel, name)
				if bone >= 0: socket[name] = rows(skeleton_xf * skel.get_bone_global_pose(bone))
			socket_frames.append(socket)
		all_min = all_min.min(bounds_min)
		all_max = all_max.max(bounds_max)
		clips.append({"name": clip_name, "duration": duration, "frames": frames, "looping": looping,
			"matrix_offset_bytes": offset, "bounds_min": v3(bounds_min), "bounds_max": v3(bounds_max), "sockets": socket_frames})
		print("  ", clip_name, " ", frames, " frames, bounds ", bounds_min, " .. ", bounds_max)
	anim_out.close()
	var document := {"version": 1, "name": model_name, "mesh_file": mesh_file, "animation_file": anim_file,
		"vertex_count": positions.size(), "index_count": indices.size(), "joint_count": palette.size(),
		"vertex_stride_bytes": 96, "index_offset_bytes": positions.size() * 96, "matrix_stride_bytes": 48,
		"coordinate_system": "right-handed, +Y up, +Z model forward, metres; no gameplay 180Y or -0.9 foot offset baked",
		"winding": "counter-clockwise", "uv_origin": "top-left", "albedo_texture_color_space": "sRGB",
		"bounds_min": v3(all_min), "bounds_max": v3(all_max), "surfaces": surfaces, "joints": joints, "clips": clips,
		"validation": {"original_ccw_triangles": original_winding_score, "converted_ccw_triangles": reverse_winding_score},
		"source": {"model": "assets/models/" + model_name + ".fbx", "retarget": "scripts/character_visual.gd",
			"importer": "Godot 4.6.2 FBX importer", "animation_fps": FPS}}
	var json_out := FileAccess.open(output_dir.path_join(model_name + ".json"), FileAccess.WRITE)
	json_out.store_string(JSON.stringify(document, "\t", false, true) + "\n")
	json_out.close()
	print("  wrote ", positions.size(), " vertices, ", indices.size() / 3, " triangles, ", palette.size(), " used bindings")
	model.free()
