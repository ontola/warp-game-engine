-- The numbers that make a car (sim/src/spec.rs). Lengths m, masses kg,
-- torque Nm. Local frame: -Z forward, +X right, +Y up; y = 0 is the
-- wheel-centre height at rest, z = 0 midway between the axles.
--
-- When an edit applies: the game reads this file again for every new stage
-- (N, or the end of a stage) and at GameNight Prepare, never in the middle
-- of one, so a stage, its replay and the bench always run on one set of
-- numbers. `sideways bench` reads it at every run. A file with an error
-- keeps the cars from before and says why on screen.
--
-- Every car is `base` with its own numbers on top (`model` below).

-- A four-wheel drive gravel car in the spirit of today's Rally2 cars.
local base = {
	name = "Rally2",
	mass = 1230.0,
	inertia = {1900.0, 2100.0, 520.0}, -- pitch, yaw, roll
	cg = {0.0, 0.16, -0.12},
	wheelbase = 2.50,
	track = 1.56,
	wheel_radius = 0.33,
	wheel_inertia = 1.1,
	droop_y = -0.10,
	travel = 0.24,
	spring = {34000.0, 30000.0}, -- front, rear N/m
	bump = {3200.0, 2800.0},
	rebound = {4800.0, 4200.0},
	arb = {9000.0, 6000.0},
	max_steer_deg = 30.0,
	-- 27 October handling trial: more available grip and earlier lateral bite.
	-- Both scales accept 0.5..1.5; 1.0/1.0 restores the original tyre response.
	tyre_grip_scale = 1.12,
	lateral_peak_scale = 0.80,
	brake = {1650.0, 1050.0},
	handbrake = 2600.0,
	drive = {0.5, 0.5},
	centre_lock = 900.0,
	diff_preload = {80.0, 120.0},
	diff_ramp = {0.45, 0.65},
	idle_rpm = 950.0,
	limit_rpm = 7600.0,
	engine_inertia = 0.16,
	torque = {{1000.0, 160.0}, {2000.0, 250.0}, {3000.0, 390.0}, {4000.0, 440.0}, {5000.0, 430.0}, {6000.0, 385.0}, {7000.0, 330.0}, {7600.0, 290.0}, {8200.0, 0.0}},
	friction = {18.0, 0.0085}, -- engine braking: a + b * rpm
	gears = {3.20, 3.00, 2.15, 1.67, 1.36, 1.15, 1.00}, -- index 0 is reverse
	final = 4.40,
	shift_time = 0.08,
	clutch = 650.0,
	upshift_rpm = 7150.0,
	downshift_rpm = 3600.0,
	drag = 0.42,
	downforce = 0.12,
	-- Collision boxes: [size x, y, z, centre x, y, z]. The bench uses this one.
	boxes = {{1.75, 0.7, 4.0, 0.0, 0.45, 0.0}},
}

local cars = { base = base }

-- base with `own` on top.
local function model(own)
	local car = table.clone(base)
	for k, v in own do
		car[k] = v
	end
	return car
end

-- The grid: the base numbers with each body's own size and character.
-- Boxes come from the glTF bodies (tools/oracle_dump.gd, CarModel.collision).
cars.asti = model {
	name = "Asti Stradale",
	model = "res://assets/cars/asti_stradale.glb",
	wheelbase = 2.522,
	track = 1.438,
	track_rear = 1.475,
	wheel_radius = 0.30,
	mass = 1250.0,
	drive = {0.47, 0.53},
	ride = 0.03,
	boxes = {{1.784507, 0.501109, 3.787738, 0.0, 0.310555, -0.023666}, {1.398171, 0.522014, 1.952442, 0.0, 0.822117, -0.023666}},
}

cars.mv6 = model {
	name = "MV6",
	model = "res://assets/cars/mv6.glb",
	wheelbase = 2.284,
	track = 1.572,
	track_rear = 1.593,
	wheel_radius = 0.325,
	-- Short, light and rear-biased: quick to rotate, quick to bite back.
	mass = 1120.0,
	inertia = {1550.0, 1750.0, 480.0},
	cg = {0.0, 0.17, 0.05},
	drive = {0.38, 0.62},
	ride = 0.08,
	boxes = {{1.890907, 0.616636, 3.776485, 0.0, 0.368318, 0.033006}, {1.481535, 0.684274, 1.946642, 0.0, 1.018773, 0.033006}},
}

cars.toyama = model {
	name = "Toyama XR",
	model = "res://assets/cars/toyama_xr.glb",
	wheelbase = 2.524,
	track = 1.518,
	wheel_radius = 0.33,
	-- Long and planted, a bit heavy at the nose.
	mass = 1290.0,
	inertia = {2000.0, 2250.0, 540.0},
	cg = {0.0, 0.17, -0.18},
	ride = 0.04,
	boxes = {{1.888421, 0.514456, 4.303236, 0.0, 0.317228, -0.00234}, {1.479588, 0.554393, 2.218163, 0.0, 0.851653, -0.00234}},
}

return { cars = cars }
