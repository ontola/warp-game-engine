# Grounded car handling, 27 November

Joep found the cars in Sideways and Warp Theft Auto too slippery and asked
for a model grounded in measured data instead of tuning by feel. Machine:
cloud container (CPU only; the vehicle solver is CPU code). Measured with
`cargo run --release -p physics --example handling_lab` on flat ground,
before (main at b98019d) and after this change.

## What was wrong

- **Made-up tyre numbers.** The normalised curve had a peak of mu 1.25 on
  asphalt (1.12 x a hand-set grip scale of 1.12) at 6 degrees, and after
  the peak dropped to 0.71 of it (C = 1.5). A measured passenger tyre
  peaks at 1.12 at 9 degrees and keeps about 0.9 of it when sliding. A
  curve that falls away past the peak keeps a car sliding once it starts.
- **Gravel gripped like asphalt.** Gravel had mu 1.0 x 1.12: braking from
  100 km/h took 36.7 m on gravel and 37.5 m on asphalt. Measured, gravel
  has 0.6/0.85 of the friction of dry asphalt (Wong).
- **The road car had a rally drivetrain and no ABS.** The stock compact
  (Warp Theft Auto's car) was four-wheel drive with a locked centre and
  plated differentials, and no ABS: a keyboard brake locked all four wheels
  within 0.3 s and the car slid on at up to 25 degrees body slip.
- Not the game but this lab: its rig first settled the car with the brake held, which the
  automatic box read as a request for reverse; fixed by holding the car.
- **The steering assist measured the wrong angle.** It allowed body slip +
  9 degrees, blended in only from 6 to 28 m/s, so at 80 km/h full lock still
  gave 16 degrees and the front tyres ran at 15-38 degrees slip angle.

## What changed

- `vehicle/tyre.rs`: Magic Formula curves built from four measured values
  per direction (slip stiffness, peak, slip at peak, sliding level), as in
  Pacejka sec. 4.3. Stiffness, peak and load sensitivity come from a
  published MF 5.2 file of a 205/60R15 tyre; the sliding level from each
  surface's measured sliding/peak friction. The MF 5.2 file's own curve is
  not used past the peak: its lateral C = 2.13 turns negative at large slip
  under high load (at 8000 N the force is negative at 80 degrees).
  Combined slip by Pacejka's similarity method.
- `vehicle/surfaces.rs`: friction and sliding level per surface from Wong
  (1993) and Bosch (1996), rolling resistance from the same tables. Loose
  gravel and grass have no source yet and keep their earlier ratios.
- `CarSpec::abs`: ABS with EBD; `CarSpec::traction_control`: brakes a
  driven wheel that spins past the car's speed (a front wheel lifted on a
  rough hill start otherwise takes all the torque through the open
  differential). The compact has both and is now front-wheel drive with
  open differentials; the Rally2 cars keep theirs and have neither.
- Steering assist: no further than the tyre's peak slip angle past the
  direction the front axle actually moves.
- Sideways `cars.luau` and the compact use tyre scales 1.0/1.0 (the
  27 October 1.12/0.80 trial was by feel).

## Numbers

| Test | before | after | real |
| --- | ---: | ---: | --- |
| Compact skidpad 30.5 m, asphalt (g) | 1.01 | 1.00 | 0.85-0.95 road tests |
| Compact 100-0 km/h, asphalt, full pedal (m) | 37.2 (locked) | 38.7 (ABS) | 34-38 with ABS |
| Compact 100-0 km/h, gravel (m) | 37.1 | 55.2 | ~50-55 (Wong ratio) |
| Compact 0-100 km/h (s) | 5.3 (AWD) | 6.3 (FWD) | 6-7 for 300 Nm |
| Rally2 skidpad, gravel (g) | 0.91 | 0.61 | 0.6-0.8 road tyre |
| Rally2 100-0 km/h gravel, locked (m) | 37.0 | 49.5 | ~50 (Wong ratio) |
| Compact step steer 80 km/h, yaw response (s) | 0.14 | 0.27 | 0.15-0.35 |
| Compact, full brake + full lock at 80 km/h, max body slip (deg) | 13.2 | 4.3 | ABS keeps steering |
| Compact, full throttle + full lock at 80 km/h, max body slip (deg) | 2.7 | 7.0 | |
| Rally2 gravel, full throttle + full lock at 80 km/h | spins (75 deg) | spins (180 deg) | needs countersteer |

Plot: project file `handling/handling-voor-na.png`. Raw results:
`build/handling_lab_*.json` from the commands above.

## Second step: steering compliance and ESC

- **Suspension K&C.** `CarSpec::force_steer`, `torque_steer` and
  `roll_steer` steer each wheel by its lateral force, aligning torque and
  the body roll, in the K&C rig convention. The aligning torque uses the
  pneumatic trail from the same MF 5.2 file (QBZ1, QCZ1, QDZ1, QEZ1; 19 mm
  at 3000 N). The compact takes the middle of the ranges measured on
  passenger cars (Lee 2018): front 0.19 deg/kN, 1.0 deg/100 Nm,
  0.05 deg/deg; rear -0.10, -0.125, 0.035. Its understeer gradient goes
  from 0.1 to 1.2 deg/g; measured road cars span 0.7-8.2 (front drive
  average 4.2). Rally2 and Sideways' cars stay rigid.
- **ESC** (`CarSpec::esc`, on for the compact): yaw-rate control after
  Bosch ESP. The compact without it spins in the FMVSS 126 sine with dwell
  from 5.5 A (44 degrees body slip at 6.5 A on asphalt, 64 on gravel);
  with it the yaw rate ratios are 0 at 1 and 1.75 s, body slip at most
  8 degrees and the lateral displacement 3.3 m (limits: 0.35, 0.20,
  1.83 m). Plot: project file `handling/handling-esc.png`.
- **Traction control bug.** It compared each wheel's speed with the car's
  speed, so in a tight turn the outer front wheel counted as spinning and
  was braked: a car making a U-turn at full lock crawled at 1.5 m/s. It now
  compares with the ground speed at that wheel.
- Warp Theft Auto tests that hit a pedestrian from a standstill now start
  further away or pin the new impact speeds (the front-drive car
  accelerates like a road car, 0-100 in 6.2 s). The pinned-driver test
  ran wide out of a corner only because the old car slid; it now meets a
  parked car across the lane.

## Open

- Understeer is at the low end of real cars. Roll camber, the largest K&C
  term in Bundorf's budget, needs camber thrust data the tyre file does not
  give. The compact also carries only 55% of its weight on the front axle;
  front-drive compacts usually carry more (not yet checked against data).
- No measured data yet for rally tyres on gravel, or for grass and loose
  gravel. Sideways on gravel now uses road-tyre ratios (0.61 g on the
  skidpad), which is likely below a Rally2 car on gravel tyres.
- The Rally2 car spins under full throttle and full lock on gravel and does
  not recover while the throttle stays down (AWD with locked centre and
  plated diffs); a real one needs countersteer too, but this is not yet
  checked against data.
