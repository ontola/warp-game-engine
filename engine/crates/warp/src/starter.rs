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

/// The game kit's guide, written into templated games for the people and
/// models that edit them.
pub const KIT_GUIDE: &str = include_str!("../../../docs/kit.md");

/// A template: a game on the kit to start from (`warp new DIR --template NAME`).
pub struct Template { pub name: &'static str, pub about: &'static str, pub files: &'static [(&'static str, &'static str)] }

/// Every template. Each has main.luau, game.toml, README.md and a smoke
/// playtest; the starter's sounds and API types are added to it.
pub const TEMPLATES: &[Template] = &[
    Template {
        name: "platformer",
        about: "Coin Climb: side-view platformer, 1-4 players race up ledges for coins",
        files: &[
            ("game.toml", include_str!("../templates/platformer/game.toml")),
            ("main.luau", include_str!("../templates/platformer/main.luau")),
            ("README.md", include_str!("../templates/platformer/README.md")),
            ("scripts/smoke.json", include_str!("../templates/platformer/scripts/smoke.json")),
        ],
    },
];

pub fn template(name: &str) -> Result<&'static Template, String> {
    TEMPLATES.iter().find(|t| t.name == name).ok_or_else(|| {
        let names: Vec<&str> = TEMPLATES.iter().map(|t| t.name).collect();
        format!("unknown template {name}; templates: {}", names.join(", "))
    })
}

/// The files `warp new --template NAME` writes: the template's, the
/// starter's sounds and types, and KIT.md.
pub fn template_files(t: &Template) -> Vec<(&'static str, &'static str)> {
    let mut files: Vec<(&str, &str)> = t.files.to_vec();
    for (rel, text) in FILES {
        if (rel.starts_with("sounds/") || *rel == "types.d.luau") && !files.iter().any(|(r, _)| r == rel) { files.push((rel, text)); }
    }
    files.push(("KIT.md", KIT_GUIDE));
    files
}

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
pub fn write(dir: &Path, title: &str) -> Result<Vec<String>, String> { write_files(dir, title, FILES) }

/// Write a template into `dir`, which may exist but must not hold a game.
pub fn write_template(dir: &Path, title: &str, template: &Template) -> Result<Vec<String>, String> {
    write_files(dir, title, &template_files(template))
}

fn write_files(dir: &Path, title: &str, files: &[(&str, &str)]) -> Result<Vec<String>, String> {
    if dir.join("main.luau").exists() { return Err(format!("{}: already a game (main.luau exists); pick a new folder", dir.display())); }
    let mut written = Vec::new();
    for (rel, text) in files {
        let path = dir.join(rel);
        if path.exists() { return Err(format!("{}: exists; pick a new folder", path.display())); }
        if let Some(p) = path.parent() { std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?; }
        let text = if *rel == "game.toml" { retitle(text, title) } else { text.to_string() };
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        written.push(rel.to_string());
    }
    std::fs::create_dir_all(dir.join("assets")).map_err(|e| e.to_string())?;
    Ok(written)
}

/// game.toml with its `title = ...` line set to `title`.
fn retitle(toml: &str, title: &str) -> String {
    let mut done = false;
    let mut out: Vec<String> = toml.lines().map(|l| {
        if !done && l.starts_with("title = ") { done = true; format!("title = {title:?}") } else { l.to_string() }
    }).collect();
    if toml.ends_with('\n') { out.push(String::new()); }
    out.join("\n")
}
