extends Node
# External instrumentation only. The packed benchmark and its event/effect caps are unchanged.
const ViolenceScript = preload("res://scripts/violence.gd")
var out_dir: String = ""
var scenario: String = ""
var capture_only: bool = false
var rng_seed: int = 721077
var rows: Array = []
var intervals: Array = []
var gpu_times: Array = []
var render_cpu_times: Array = []
var states: Dictionary = {}
var events: Dictionary = {}
var blast_profile: Dictionary = {}
var blast_start: Dictionary = {}
var peak_projectiles: int = 0
var previous_usec: int = 0
var first_measure_usec: int = 0
var last_measure_usec: int = 0
var was_measuring: bool = false
var finished: bool = false
var scene_done: bool = false
var capture_started: bool = false
var last_drawn: int = -1
var corrections: int = 0
var governor: Node = null
var screen: Node = null
var requested_position: Vector2i = Vector2i(-4000, 0)
var adapter_name: String = "unknown; inspect engine.log"

func _enter_tree() -> void:
	out_dir = OS.get_environment("GG_STRESS_OUT").replace("\\", "/")
	scenario = OS.get_environment("GG_STRESS_SCENARIO")
	capture_only = OS.get_environment("GG_STRESS_CAPTURE_ONLY") == "1"
	var seed_text: String = OS.get_environment("GG_STRESS_SEED")
	if not seed_text.is_empty():
		rng_seed = int(seed_text)
	seed(rng_seed) # Autoload enters before the main scene's _ready and random shooter setup.
	var position_text: PackedStringArray = OS.get_environment("GG_STRESS_POSITION").split(",")
	if position_text.size() == 2:
		requested_position = Vector2i(int(position_text[0]), int(position_text[1]))
	process_mode = Node.PROCESS_MODE_ALWAYS
	process_priority = 100000

func _ready() -> void:
	governor = get_node_or_null("/root/PerfGovernor")
	screen = get_node_or_null("/root/GameNightScreen")
	if screen != null:
		screen.set("_epoch", int(screen.get("_epoch")) + 1)
	if governor != null:
		governor.set_process(false)
		governor.set("quality_scale", 1.0)
		governor.set("shed_level", 0)
		governor.set("casings_enabled", true)
		governor.set("full_blasts_allowed", true)
		governor.set("debris_jobs_per_flush", 99)
	RenderingServer.viewport_set_measure_render_time(get_tree().root.get_viewport_rid(), true)
	if RenderingServer.has_method("get_video_adapter_name"):
		adapter_name = str(RenderingServer.call("get_video_adapter_name"))
	call_deferred("_enforce_controls")
	print("[stress-probe] loaded scenario=%s seed=%d adapter=%s capture_only=%s" % [scenario, rng_seed, adapter_name, capture_only])

func _enforce_controls() -> void:
	Engine.max_fps = 0
	Engine.time_scale = 1.0
	if governor != null:
		governor.set_process(false)
		governor.set("quality_scale", 1.0)
	var root: Window = get_tree().root
	if not is_equal_approx(root.scaling_3d_scale, 1.0):
		root.scaling_3d_scale = 1.0
		corrections += 1
	if DisplayServer.window_get_mode() != DisplayServer.WINDOW_MODE_WINDOWED:
		if screen != null:
			screen.set("_epoch", int(screen.get("_epoch")) + 1)
		DisplayServer.window_set_mode(DisplayServer.WINDOW_MODE_WINDOWED)
		corrections += 1
	if DisplayServer.window_get_size() != Vector2i(1920, 1080):
		DisplayServer.window_set_size(Vector2i(1920, 1080))
		corrections += 1
	if DisplayServer.window_get_position() != requested_position:
		DisplayServer.window_set_position(requested_position)
	if not DisplayServer.window_get_flag(DisplayServer.WINDOW_FLAG_NO_FOCUS):
		DisplayServer.window_set_flag(DisplayServer.WINDOW_FLAG_NO_FOCUS, true)
	if DisplayServer.window_get_vsync_mode() != DisplayServer.VSYNC_DISABLED:
		DisplayServer.window_set_vsync_mode(DisplayServer.VSYNC_DISABLED)

func _process(_delta: float) -> void:
	if finished:
		return
	_enforce_controls()
	var scene: Node = get_tree().current_scene
	if scene == null:
		return
	var phase: String = str(scene.get("_phase"))
	var now: int = Time.get_ticks_usec()
	if phase == "done":
		scene_done = true
		_read_events(scene)
		_finish("scene_done")
		return
	if phase != "measure":
		was_measuring = false
		return
	_read_events(scene)
	if not was_measuring:
		first_measure_usec = now
		blast_start = ViolenceScript.bench_blast_prof_summary()
		previous_usec = now
		was_measuring = true
	last_measure_usec = now
	if capture_only:
		if not capture_started and now - first_measure_usec >= 2000000:
			capture_started = true
			call_deferred("_capture")
		return
	var interval: int = now - previous_usec
	previous_usec = now
	if interval <= 0:
		return
	var projectiles: int = get_tree().get_nodes_in_group("projectiles").size()
	peak_projectiles = maxi(peak_projectiles, projectiles)
	var root: Window = get_tree().root
	var win_size: Vector2i = DisplayServer.window_get_size()
	var vp_size: Vector2 = root.get_visible_rect().size
	var mode: int = DisplayServer.window_get_mode()
	var quality: float = float(governor.get("quality_scale")) if governor != null else 1.0
	var drawn: int = Engine.get_frames_drawn()
	var new_draw: bool = drawn != last_drawn
	last_drawn = drawn
	var render_cpu: float = RenderingServer.viewport_get_measured_render_time_cpu(root.get_viewport_rid())
	var gpu: float = RenderingServer.viewport_get_measured_render_time_gpu(root.get_viewport_rid())
	if new_draw and gpu > 0.0:
		gpu_times.append(gpu)
	if new_draw and render_cpu > 0.0:
		render_cpu_times.append(render_cpu)
	var state_key: String = "%d,%d,%d,%d,%d,%.4f,%.4f" % [mode, win_size.x, win_size.y, int(vp_size.x), int(vp_size.y), root.scaling_3d_scale, quality]
	states[state_key] = int(states.get(state_key, 0)) + 1
	intervals.append(interval)
	rows.append([rows.size(), now - first_measure_usec, interval, Engine.get_process_frames(), drawn, render_cpu, gpu, int(new_draw), mode, win_size.x, win_size.y, int(vp_size.x), int(vp_size.y), root.scaling_3d_scale, quality, projectiles, int(Performance.get_monitor(Performance.RENDER_TOTAL_DRAW_CALLS_IN_FRAME)), int(Performance.get_monitor(Performance.RENDER_TOTAL_OBJECTS_IN_FRAME))])

func _read_events(scene: Node) -> void:
	for key in ["ev_bullets_spawned", "ev_collisions", "ev_explosions", "ev_casings_spawned"]:
		events[key] = int(scene.get(key))
	if scenario == "bullets":
		events["shots_triggered"] = int(scene.get("_shots_triggered"))
	blast_profile = ViolenceScript.bench_blast_prof_summary()

func _capture() -> void:
	await RenderingServer.frame_post_draw
	var pixels: Image = get_tree().root.get_texture().get_image()
	var capture_path: String = out_dir.path_join("visual.jpg")
	var error: int = pixels.save_jpg(capture_path, 0.88)
	print("[stress-probe] visual-only capture=%s error=%d; no performance samples" % [capture_path, error])
	_finish("visual_capture")
	get_tree().quit(error)

func _stats(values: Array) -> Dictionary:
	if values.is_empty():
		return {"count": 0, "available": false}
	var sorted: Array = values.duplicate()
	sorted.sort()
	var total: float = 0.0
	for value in sorted:
		total += float(value)
	return {"count": sorted.size(), "available": true, "mean": total / sorted.size(), "min": sorted[0], "p50": sorted[int(floor((sorted.size() - 1) * 0.50))], "p95": sorted[int(floor((sorted.size() - 1) * 0.95))], "p99": sorted[int(floor((sorted.size() - 1) * 0.99))], "max": sorted[-1], "sum": total}

func _state_bounds() -> Dictionary:
	var bounds: Dictionary = {}
	var columns: Dictionary = {"window_width": 9, "window_height": 10, "viewport_width": 11, "viewport_height": 12, "render_scale": 13, "quality_scale": 14}
	for key in columns:
		var values: Array = []
		for row in rows:
			values.append(row[int(columns[key])])
		if not values.is_empty():
			bounds[key] = {"min": values.min(), "max": values.max()}
	return bounds

func _finish(reason: String) -> void:
	if finished:
		return
	finished = true
	if out_dir.is_empty():
		push_error("GG_STRESS_OUT is missing")
		return
	DirAccess.make_dir_recursive_absolute(out_dir)
	var csv: FileAccess = FileAccess.open(out_dir.path_join("frames.csv"), FileAccess.WRITE)
	if csv != null:
		csv.store_line("sample,elapsed_us,process_loop_interval_us,process_frame,drawn_frame,viewport_render_cpu_ms,viewport_gpu_ms,draw_counter_changed,window_mode,window_width,window_height,viewport_width,viewport_height,render_scale,quality_scale,projectiles,draw_calls,drawn_objects")
		for row in rows:
			var cells: PackedStringArray = PackedStringArray()
			for value in row:
				cells.append(str(value))
			csv.store_csv_line(cells)
		csv.close()
	var summary: Dictionary = {
		"schema": 1, "scenario": scenario, "reason": reason, "scene_completed": scene_done,
		"visual_only": capture_only, "adapter_name": adapter_name, "seed_before_main_scene_ready": rng_seed,
		"perf_scene_seed_override": 721077 if scenario == "explosions" else null,
		"original_warmup_seconds": 2.0 if scenario == "bullets" else 3.0,
		"original_measure_seconds": 12.0 if scenario == "bullets" else 15.0,
		"raw_process_loop_interval_us": _stats(intervals), "viewport_gpu_ms": _stats(gpu_times),
		"viewport_render_cpu_ms": _stats(render_cpu_times), "observed_measure_interior_seconds": float(last_measure_usec - first_measure_usec) / 1000000.0,
		"state_keys_mode_windowWH_viewportWH_renderScale_quality": states, "window_corrections": corrections,
		"actual_state_min_max": _state_bounds(),
		"event_totals_scene_measure_phase": events, "peak_projectiles_sampled": peak_projectiles,
		"blast_profile_at_end": blast_profile, "blast_profile_at_first_observed_measure": blast_start,
		"controls": {"resolution": [1920,1080], "position": [requested_position.x,requested_position.y], "windowed": true, "no_focus": true, "max_fps": 0, "vsync": 0, "render_scale": 1.0, "quality_scale": 1.0, "governor_processing_disabled": true, "screen_claim_epoch_cancelled": true},
		"scope_notes": ["Intervals are successive monotonic wall-clock callbacks wholly inside the observed scene measure phase; boundary intervals and final result printing are excluded.", "Process-loop rate is not presentation FPS or total CPU time. Viewport CPU is render work only; GPU queries may lag and are not synchronized to CSV rows. Zero timings are not treated as valid samples.", "No render wait or image readback in performance runs. Probe polling, counting projectiles and GPU timestamp instrumentation add overhead.", "Adaptive governor disabled and VFX quality held at 1; original scene timings, weapon caps, blast full/cheap/time budgets and counters remain active.", "Scene event totals cover the scene's full measurement phase and can differ slightly from the probe's interior interval window.", "Blast profile full/cheap counts are the game's branch counters, not a count of visible concurrent effects. Manual wall spray is absent from ev_explosions; use blast_calls too.", "Fixed RNG seed does not make scheduling, physics and frame-budget effects deterministic.", "Offscreen windowed rendering is not a measurement of onscreen presentation latency."]
	}
	var json_file: FileAccess = FileAccess.open(out_dir.path_join("summary.json"), FileAccess.WRITE)
	if json_file != null:
		json_file.store_string(JSON.stringify(summary, "\t"))
		json_file.close()
	print("[stress-probe] saved %s reason=%s samples=%d scene_completed=%s" % [out_dir, reason, rows.size(), scene_done])

func _exit_tree() -> void:
	_finish("tree_exit")
