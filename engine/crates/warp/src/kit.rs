//! The game kit: Luau libraries compiled into the executable, so every
//! game can `require("kit/controller")` without copying files. A game's own
//! `kit/NAME.module.luau` wins over the built-in one (copy it to change it).
//! Guide: engine/docs/kit.md.

/// (require name, source)
pub const FILES: &[(&str, &str)] = &[
    ("kit/util", include_str!("../kit/util.luau")),
    ("kit/timer", include_str!("../kit/timer.luau")),
    ("kit/controller", include_str!("../kit/controller.luau")),
    ("kit/camera", include_str!("../kit/camera.luau")),
    ("kit/ui", include_str!("../kit/ui.luau")),
    ("kit/round", include_str!("../kit/round.luau")),
    ("kit/bot", include_str!("../kit/bot.luau")),
];

/// Make the kit `require`-able in `host`.
pub fn install(host: &mut script::Host) {
    for (name, source) in FILES { host.builtin_library(name, source); }
}
