//! The room in metres. The play plane is z = 0, x grows right, y grows up.
//! Collision lives here so the simulation and the 3D room never disagree.
//! A port of the Godot lobby's layout.gd.

pub const LEFT: f64 = -11.6;
pub const RIGHT: f64 = 11.6;
pub const CEILING: f64 = 10.4;
pub const BACK: f64 = -2.6;

/// A one-way platform. `depth` is the z a player stands at on that surface,
/// so the sprite sits on it instead of hovering in front.
#[derive(Clone, Copy, Debug)]
pub struct Platform {
    pub x0: f64,
    pub x1: f64,
    pub top: f64,
    pub depth: f64,
}

const fn p(x0: f64, x1: f64, top: f64, depth: f64) -> Platform {
    Platform { x0, x1, top, depth }
}

/// Players on it stand in front of the Up next cabinet beside it (its
/// titles are at z -1.64): never hidden behind it (Joep, 2026-11-27).
pub const TV_CONSOLE: Platform = p(-3.3, 3.3, 0.9, -1.45);

pub const PLATFORMS: [Platform; 16] = [
    // Game bookcase and the display stand.
    p(-10.3, -8.9, 2.65, -1.95),
    p(-8.7, -6.5, 0.8, -1.75),
    // Shelves on the left wall.
    p(-11.6, -8.6, 4.7, -2.0),
    p(-7.4, -5.2, 5.9, -2.0),
    // Neon shelf above the TV.
    p(-2.6, 2.6, 6.55, -2.0),
    TV_CONSOLE,
    // Little floating speakers either side of the TV.
    p(-4.7, -3.7, 3.6, -2.05),
    p(3.7, 4.7, 3.6, -2.05),
    // Couch seat and back.
    p(4.9, 8.0, 0.85, -1.5),
    p(5.0, 7.9, 1.55, -2.1),
    // Shelf over the couch and the plaque ledge on the right.
    p(5.2, 7.7, 4.3, -2.0),
    p(8.6, 11.6, 3.15, -2.0),
    // High ledges for the brave. None above the neon shelf: it blocked
    // jumps there (Joep).
    p(-11.6, -9.6, 7.3, -2.0),
    p(-6.4, -4.4, 8.2, -2.0),
    p(4.4, 6.4, 8.2, -2.0),
    p(9.0, 11.6, 6.6, -2.0),
];

/// Depth of the surface under a player at (x, y): the highest platform at or
/// below their feet, or the floor at z = 0.
pub fn depth_at(x: f64, y: f64) -> f64 {
    let mut best = 0.0;
    let mut top = 0.05;
    for plat in PLATFORMS {
        if x < plat.x0 || x > plat.x1 || plat.top > y + 0.05 || plat.top < top {
            continue;
        }
        top = plat.top;
        best = plat.depth;
    }
    best
}

pub const SPAWN_X: [f64; 8] = [-3.0, -1.0, 1.0, 3.0, -5.0, 5.0, -2.0, 2.0];

/// Stations: the player stands in [x0, x1] on the floor and presses Y.
pub const BOOKCASE: (f64, f64) = (-10.3, -8.9);
pub const SPINE_ROWS: [f64; 2] = [0.07, 0.93];
pub const DISPLAY: (f64, f64) = (-8.7, -6.5);
/// Selector: Y plays now, X queues, LB/RB browse.
pub const GAME_SHELF: (f64, f64) = BOOKCASE;
pub const UP_NEXT: (f64, f64) = (-5.8, -3.2);
/// The browser host's Public / Private switch, on the display stand by
/// the door (Joep, 2026-11-26).
pub const SWITCH: f64 = -7.6;
/// In front of the TV: Y starts or resumes.
pub const TV_STATION: (f64, f64) = (-2.9, 2.7);
pub const EXIT_DOOR: f64 = -11.0;
pub const JUKEBOX: f64 = 3.6;
pub const MUSIC_PADS: [f64; 3] = [3.05, 3.6, 4.15];
pub const PROFILE_DOORS: [f64; 3] = [8.8, 9.9, 11.0];
pub const DOOR_HALF: f64 = 0.52;

/// Where the camera sits; it looks straight down -z, so x projects simply.
pub const CAMERA: [f64; 3] = [0.0, 4.7, 20.4];
/// The z of the furniture players use: bookcase, cabinet, TV, jukebox, doors.
pub const OBJECTS_Z: f64 = -2.0;

/// The x on the furniture plane that a player at (x, z) appears in front of
/// from the camera. Perspective spreads the near play plane outward, so near
/// the walls a player can look in front of a cabinet before standing at it.
pub fn apparent_x(x: f64, z: f64) -> f64 {
    CAMERA[0] + (x - CAMERA[0]) * (CAMERA[2] - OBJECTS_Z) / (CAMERA[2] - z)
}

/// Where to stand on the floor to appear in front of furniture at x.
pub fn stand_x(x: f64) -> f64 {
    CAMERA[0] + (x - CAMERA[0]) * CAMERA[2] / (CAMERA[2] - OBJECTS_Z)
}

pub const TV_CENTER: [f64; 3] = [0.0, 3.95, BACK + 0.12];
pub const TV_SIZE: [f64; 2] = [5.6, 3.15];
pub const PLAQUE_CENTER: [f64; 3] = [10.1, 4.9, BACK + 0.08];
