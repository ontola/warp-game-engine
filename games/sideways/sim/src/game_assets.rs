//! Sideways-owned assets and tuning data, independent of the working directory.
//! A bundle contains `sideways.exe`, `assets/` and `data/` beside one another.
//! `SIDEWAYS_GAME_ROOT` explicitly selects that directory, including a missing
//! one (errors must not silently read unrelated checkout assets).
use std::path::{Path, PathBuf};

pub fn source_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("simulation is inside the game").to_path_buf()
}

fn select_root(explicit: Option<PathBuf>, executable: Option<&Path>, source: &Path, is_bundle: impl Fn(&Path) -> bool) -> PathBuf {
    if let Some(path) = explicit { return path; }
    if let Some(parent) = executable.and_then(Path::parent) {
        if is_bundle(parent) { return parent.to_path_buf(); }
    }
    source.to_path_buf()
}

pub fn root() -> PathBuf {
    select_root(std::env::var_os("SIDEWAYS_GAME_ROOT").map(PathBuf::from), std::env::current_exe().ok().as_deref(),
        &source_root(), |p| p.join("assets").is_dir())
}

pub fn asset(relative: impl AsRef<Path>) -> PathBuf { root().join("assets").join(relative) }
pub fn data(relative: impl AsRef<Path>) -> PathBuf { root().join("data").join(relative) }

/// Diagnostic `--repo` retains its existing repository-root spelling; a game
/// root/bundle is also accepted. Normal runtime always uses `root()`.
pub fn diagnostic_root(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let contained = path.join("games/sideways");
    if contained.join("assets").is_dir() { contained } else { path.to_path_buf() }
}

pub fn read_data(relative: &str, built_in: &str) -> Result<String, String> {
    let path = data(relative);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && std::env::var_os("SIDEWAYS_GAME_ROOT").is_none() => Ok(built_in.to_owned()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn explicit_root_never_silently_falls_back() {
        let source=Path::new("checkout/game");let exe=Path::new("bundle/sideways.exe");
        assert_eq!(select_root(Some("missing-root".into()),Some(exe),source,|_|true),PathBuf::from("missing-root"));
    }
    #[test] fn bundle_precedes_source_and_never_depends_on_cwd() {
        let source=Path::new("checkout/game");let exe=Path::new("bundle/sideways.exe");
        assert_eq!(select_root(None,Some(exe),source,|p|p==Path::new("bundle")),PathBuf::from("bundle"));
        assert_eq!(select_root(None,Some(exe),source,|_|false),source);
        assert_eq!(select_root(None,None,source,|_|true),source);
    }
    #[test] fn checked_in_bundle_has_native_models_and_presets() {
        let game=source_root();
        for asset in ["meshes/fir_a_near.kmesh","meshes/fir_b_near.kmesh","meshes/fir_c_near.kmesh","trees/authored_pine_albedo.png","grass/grass_medium_01_alpha_2k.png","sky/sky_2k.hdr"] {
            assert!(game.join("assets").join(asset).is_file(),"missing {asset}");
        }
        for name in ["cars.luau","car_materials.luau","sounds.luau"] {assert!(game.join("data").join(name).is_file(),"missing {name}");}
        assert_eq!(diagnostic_root(game.parent().unwrap().parent().unwrap()),game);
    }
}
