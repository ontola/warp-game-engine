extends SceneTree
## External comparison fixture. Does not modify the reference project.
## Uses production startup, stage, assets/materials, then freezes and copies
## the actual camera/body/wheel/paint inputs recorded by Kern's shot sidecar.

const MAIN_SOURCE := """
extends "res://main.gd"
func _save_shot() -> void:
	set_physics_process(false)
	PhysicsServer3D.set_active(false)
	get_tree().call_deferred("_capture", self)
"""

var _reference: Dictionary = {}
var _metadata_path := ""
var _output := ""
var _capturing := false
var _settle_frames := 4


func _initialize() -> void:
	_run.call_deferred()


func _run() -> void:
	var args := OS.get_cmdline_user_args()
	for i in range(args.size() - 1):
		match args[i]:
			"--kern-metadata": _metadata_path = args[i + 1]
			"--shot": _output = args[i + 1]
			"--match-settle": _settle_frames = maxi(3, int(args[i + 1]))
	if _metadata_path.is_empty() or _output.is_empty():
		_fail("Need --kern-metadata <shot.png.json> and --shot <fresh output.png>")
		return
	if FileAccess.file_exists(_output) or FileAccess.file_exists(_output + ".json"):
		_fail("Refusing to overwrite existing comparison evidence")
		return
	var parsed: Variant = JSON.parse_string(FileAccess.get_file_as_string(_metadata_path))
	if not parsed is Dictionary:
		_fail("Cannot parse Kern metadata")
		return
	_reference = parsed
	var views: Array = _reference.get("views", [])
	if views.size() != 1 or _reference.get("cars", []).is_empty():
		_fail("This fixture currently requires exactly one view and at least one car")
		return
	if DisplayServer.get_name() == "headless":
		_fail("Use Xvfb plus the real Vulkan renderer, not Godot --headless")
		return
	await process_frame # Register the project's actual autoloads before compiling main.
	var script := GDScript.new()
	script.source_code = MAIN_SOURCE
	if script.reload() != OK:
		_fail("Could not compile production-main capture hook")
		return
	var game: Node = script.new()
	root.add_child(game)


func _capture(game: Node) -> void:
	if _capturing:
		return
	_capturing = true
	game.set_process(false)
	game.set_physics_process(false)
	PhysicsServer3D.set_active(false)
	var players: Array = game.get("players")
	var poses: Array = _reference.cars
	if players.size() != poses.size():
		_fail("Car count differs: Godot %d, Kern %d. Match --rivals." % [players.size(), poses.size()])
		return
	var expected_size := Vector2i(int(_reference.output_size[0]), int(_reference.output_size[1]))
	if root.size != expected_size:
		_fail("Actual Godot output %s differs from Kern %s" % [root.size, expected_size])
		return
	if int(game.get("seed_value")) != int(_reference.seed) or not is_equal_approx(float(game.get("km_override")), float(_reference.km)) or not is_equal_approx(float(game.get("start_at")), float(_reference.start_at_m)) or str(game.get("weather")) != str(_reference.weather_actual):
		_fail("Seed, stage length, starting distance or weather differs from sidecar")
		return
	if int(game.get("frame")) != int(_reference.requested_frames):
		_fail("Requested shot tick differs from Kern")
		return
	var original_cars: Array = []
	var matched_cars: Array = []
	for i in players.size():
		var car: RigidBody3D = players[i].car
		var pose: Dictionary = poses[i]
		var spec: Dictionary = car.get("spec")
		if str(spec.get("model", "")).get_file() != str(pose.model).get_file():
			_fail("Car model mismatch at index %d" % i)
			return
		original_cars.append({"position": _vec(car.global_position), "rotation_xyzw": _quat(car.global_basis.get_rotation_quaternion())})
		car.freeze = true
		car.set_physics_process(false)
		car.set_process(false)
		car.physics_interpolation_mode = Node.PHYSICS_INTERPOLATION_MODE_OFF
		car.linear_velocity = Vector3.ZERO
		car.angular_velocity = Vector3.ZERO
		var q: Array = pose.rotation_xyzw
		car.global_transform = Transform3D(Basis(Quaternion(float(q[0]), float(q[1]), float(q[2]), float(q[3])).normalized()), _v3(pose.position))
		var model := car.get_node_or_null("Model") as Node3D
		if model != null:
			model.position.y = float(pose.ride)
			var p: Array = pose.paint_linear
			var paint := Color(float(p[0]), float(p[1]), float(p[2]), 1.0).linear_to_srgb()
			for child in model.get_children():
				if child is MeshInstance3D:
					var mesh := child as MeshInstance3D
					for surface in mesh.mesh.get_surface_count():
						var mat := mesh.get_surface_override_material(surface) as StandardMaterial3D
						if mat != null and (mat.resource_name.contains("Bodymat") or mat.resource_name.contains("Paint")):
							mat.albedo_color = paint
		var wheels: Array = car.get("wheels")
		var desired_wheels: Array = pose.wheels
		if wheels.size() != desired_wheels.size():
			_fail("Wheel count mismatch")
			return
		for j in wheels.size():
			var w: RefCounted = wheels[j]
			var wanted: Dictionary = desired_wheels[j]
			var knuckle: Node3D = w.get("node")
			var spinner: Node3D = w.get("spinner")
			knuckle.position = _v3(wanted.anchor) + Vector3.UP * float(wanted.compression)
			knuckle.basis = Basis(Vector3.UP, float(wanted.steer))
			if spinner != null:
				spinner.basis = Basis(Vector3.RIGHT, -float(wanted.spin))
			else:
				knuckle.basis *= Basis(Vector3.RIGHT, -float(wanted.spin))
			# Kern currently draws rigid wheels. Do not retain unrelated Godot
			# physics loads as deformation after replacing the displayed pose.
			for tyre: GeometryInstance3D in w.get("tyres"):
				tyre.set_instance_shader_parameter("squash", 0.0)
				tyre.set_instance_shader_parameter("lean", 0.0)
		car.reset_physics_interpolation()
		matched_cars.append({"model": spec.model, "position": _vec(car.global_position), "rotation_xyzw": _quat(car.global_basis.get_rotation_quaternion()), "matched_wheels": desired_wheels, "matched_paint_linear": pose.paint_linear})
	var camera: Camera3D = players[0].camera
	var desired: Dictionary = _reference.views[0]
	camera.set_process(false)
	camera.set_physics_process(false)
	camera.physics_interpolation_mode = Node.PHYSICS_INTERPOLATION_MODE_OFF
	camera.keep_aspect = Camera3D.KEEP_HEIGHT
	camera.fov = rad_to_deg(float(desired.fov_y_radians))
	camera.near = float(desired.near)
	camera.global_position = _v3(desired.position)
	camera.look_at(_v3(desired.target), Vector3.UP)
	camera.reset_physics_interpolation()
	var vp: SubViewport = camera.get_viewport()
	if vp.size != expected_size or not is_equal_approx(vp.scaling_3d_scale, 1.0):
		_fail("The comparison requires native internal resolution and one full-size viewport")
		return
	vp.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	var sun: DirectionalLight3D = game.get("sun")
	var sun_dir := _v3(_reference.light.sun_direction).normalized()
	sun.basis = Basis.looking_at(-sun_dir, Vector3.UP)
	var env: Environment = game.get("environment")
	# Orientation is comparable; energy/tonemapping intentionally remain each
	# renderer's authored clear-weather defaults, not an invented lux conversion.
	var wind_time := float(_reference.simulation_steps) / float(Engine.physics_ticks_per_second)
	RenderingServer.global_shader_parameter_set("wind_time", wind_time)
	# TreeSets need two actually drawn frames to swap spare buffers. Let their
	# normal process callbacks run against the forced camera before capture.
	for n in _settle_frames:
		await RenderingServer.frame_post_draw
	var img: Image = vp.get_texture().get_image()
	if img.get_size() != expected_size:
		_fail("Captured texture has the wrong size")
		return
	var result := img.save_png(_output)
	if result != OK:
		_fail("PNG save failed: %d" % result)
		return
	var stage: Node3D = game.get("stage")
	var samples: Array = []
	for metres in [0.0, 100.0, 200.0, 250.0, 300.0, 350.0, 400.0, 500.0, 1000.0, 2000.0, 3000.0]:
		var pos: Vector3 = stage.call("point_at", metres)
		samples.append({"along_m": metres, "position": _vec(pos)})
	var report := {
		"schema": 1, "renderer": "godot", "engine": Engine.get_version_info(),
		"source_project": ProjectSettings.globalize_path("res://"), "script": get_script().resource_path,
		"kern_metadata": _metadata_path, "kern_metadata_sha256": FileAccess.get_sha256(_metadata_path),
		"seed": game.get("seed_value"), "km": game.get("km_override"), "start_at_m": game.get("start_at"),
		"godot_frame": game.get("frame"), "kern_requested_frames": _reference.requested_frames,
		"kern_simulation_steps": _reference.simulation_steps, "physics_hz": Engine.physics_ticks_per_second,
		"settings": game.get("settings"), "weather_actual": game.get("weather"), "output_size": [img.get_width(), img.get_height()],
		"viewport": {"size": [vp.size.x, vp.size.y], "scale": vp.scaling_3d_scale, "scale_mode": vp.scaling_3d_mode, "taa": vp.use_taa, "screen_aa": vp.screen_space_aa, "msaa": vp.msaa_3d},
		"camera": {"position": _vec(camera.global_position), "target": desired.target, "basis_x": _vec(camera.global_basis.x), "basis_y": _vec(camera.global_basis.y), "basis_z": _vec(camera.global_basis.z), "fov_y_degrees": camera.fov, "near": camera.near, "far": camera.far},
		"cars_before_override": original_cars, "cars_after_override": matched_cars,
		"sun": {"direction_towards": _vec(sun.global_basis.z), "energy": sun.light_energy, "color": [sun.light_color.r, sun.light_color.g, sun.light_color.b], "angular_distance": sun.light_angular_distance, "shadow_distance": sun.directional_shadow_max_distance},
		"environment": {"tonemap_mode": env.tonemap_mode, "exposure": env.tonemap_exposure, "ambient_energy": env.ambient_light_energy, "ambient_sky_contribution": env.ambient_light_sky_contribution, "sky_rotation": _vec(env.sky_rotation), "ssao": env.ssao_enabled, "glow": env.glow_enabled, "fog": env.fog_enabled},
		"stage_samples": samples, "wind_time": wind_time, "settle_draws_after_pose_override": _settle_frames,
		"limitations": ["Matched actual recorded camera and displayed body/wheel/paint inputs; not a matched simulation benchmark.", "Same seed does not prove terrain/vegetation identity across the two implementations; stage samples are recorded for checking.", "Godot lighting, sky integration, material processing, AA, foliage, fog and postprocessing remain production defaults; Kern uses its own physical-light defaults.", "Godot tyre load deformation is zeroed only for this static comparison because Kern currently draws rigid wheels.", "Kern has an infinite reversed depth projection; Godot retains its 1400m far plane.", "Captured the native 3D SubViewport, excluding Godot HUD; game props/door decals can differ from Kern.", "This Xvfb/lavapipe image is visual evidence, not hardware-GPU performance evidence."]
	}
	var file := FileAccess.open(_output + ".json", FileAccess.WRITE)
	if file == null:
		_fail("Cannot save metadata")
		return
	file.store_string(JSON.stringify(report, "\t"))
	file.close()
	print("matched shot: ", _output)
	print("matched metadata: ", _output + ".json")
	game.queue_free()
	await process_frame
	quit(0)


func _v3(a: Array) -> Vector3:
	return Vector3(float(a[0]), float(a[1]), float(a[2]))


func _vec(v: Vector3) -> Array:
	return [v.x, v.y, v.z]


func _quat(q: Quaternion) -> Array:
	return [q.x, q.y, q.z, q.w]


func _fail(message: String) -> void:
	printerr("matched shot FAILED: ", message)
	quit(1)
