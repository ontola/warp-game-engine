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

/// The same game in one file (`warp new GAME.luau`): settings in `--!`
/// lines, the sound patches in its `sounds` table.
pub const ONE_FILE: &str = include_str!("../starter/one-file.luau");

/// Write the one-file starter to `file`, which must not exist.
pub fn write_file(file: &Path, title: &str) -> Result<(), String> {
    if file.exists() { return Err(format!("{}: exists; pick a new name", file.display())); }
    if let Some(p) = file.parent().filter(|p| !p.as_os_str().is_empty()) { std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?; }
    let text = ONE_FILE.replacen("--! title = \"Hello Warp\"", &format!("--! title = {title:?}"), 1)
        .replacen("warp run hello.luau", &format!("warp run {}", file.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()), 1);
    std::fs::write(file, text).map_err(|e| format!("{}: {e}", file.display()))
}

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
