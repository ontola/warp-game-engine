//! `warp new`: the starter game, compiled into the executable. The same
//! files are games/hello-warp in the repository (a test keeps them equal).
use std::path::Path;

/// (path in the game folder, contents)
pub const FILES: &[(&str, &str)] = &[
    ("game.toml", include_str!("../starter/game.toml")),
    ("main.luau", include_str!("../starter/main.luau")),
    ("sounds/pickup.luau", include_str!("../starter/sounds/pickup.luau")),
    ("sounds/jump.luau", include_str!("../starter/sounds/jump.luau")),
    ("types.d.luau", include_str!("../starter/types.d.luau")),
    ("README.md", include_str!("../starter/README.md")),
    ("scripts/smoke.json", include_str!("../starter/scripts/smoke.json")),
];

/// Write the starter into `dir`, which may exist but must not hold a game.
pub fn write(dir: &Path, title: &str) -> Result<Vec<String>, String> {
    if dir.join("main.luau").exists() { return Err(format!("{}: already a game (main.luau exists); pick a new folder", dir.display())); }
    let mut written = Vec::new();
    for (rel, text) in FILES {
        let path = dir.join(rel);
        if path.exists() { return Err(format!("{}: exists; pick a new folder", path.display())); }
        if let Some(p) = path.parent() { std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?; }
        let text = if *rel == "game.toml" { text.replacen("title = \"Hello Warp\"", &format!("title = {title:?}"), 1) } else { text.to_string() };
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        written.push(rel.to_string());
    }
    std::fs::create_dir_all(dir.join("assets")).map_err(|e| e.to_string())?;
    Ok(written)
}
