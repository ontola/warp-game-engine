//! `game.toml`: what a game folder declares about itself.
use serde::Deserialize;
use std::path::Path;

/// Every key is optional; a folder with only `main.luau` runs.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct GameToml {
    /// Window title and the name in `state`.
    pub title: String,
    /// The GameNight game id (lowercase letters, digits and dashes). Default:
    /// the folder name. A party host's GAMENIGHT_GAME_ID wins.
    pub id: String,
    /// Simulation ticks per second; `tick(dt, input)` gets `dt = 1 / tick_rate`.
    pub tick_rate: f32,
    /// People playing without a party host (split screen), 1 to 4.
    /// `--players` overrides it. Under GameNight the host's seats decide.
    pub players: usize,
    /// One view per person (split screen, the default), or false for one
    /// shared view: every `camera.set` then moves the same camera.
    pub split: bool,
    /// Bots without a party host. `--bots` overrides it.
    pub bots: usize,
    /// Bots fill up to this many players, with or without a party host
    /// (a versus game that needs two, say). 1 to 8.
    pub min_players: usize,
    /// Seed of the `random` generator. `--seed` overrides it.
    pub seed: u64,
    /// Entities alive at once (render slots reserved up front).
    pub max_entities: usize,
    /// Window size in pixels.
    pub size: [u32; 2],
    /// Sky at start: `gradient` or a preset such as `noon`, `sunset:haze`.
    pub sky: String,
    /// Luau step budget per hook call (VM interrupts; see engine/docs/scripting.md).
    pub steps: u32,
    /// Luau memory cap in MiB.
    pub memory_mb: usize,
}

impl Default for GameToml {
    fn default() -> Self {
        GameToml { title: "Warp game".into(), id: String::new(), tick_rate: 60.0, players: 1, split: true, bots: 0, min_players: 1, seed: 1, max_entities: 1024, size: [1280, 720], sky: "gradient".into(), steps: 200_000, memory_mb: 64 }
    }
}

/// A folder's one-file game: no `main.luau` and exactly one other `.luau`
/// file at the top (type files `*.d.luau` aside), such as
/// `meteor-dash/meteor-dash.luau`. A packaged copy keeps working.
pub fn one_file_in(dir: &Path) -> Option<std::path::PathBuf> {
    if dir.join("main.luau").exists() { return None; }
    let mut files = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "luau") && !p.to_string_lossy().ends_with(".d.luau"));
    let one = files.next()?;
    files.next().is_none().then_some(one)
}

/// A game: a folder with `main.luau`, a folder holding one `.luau` game, or
/// a `.luau` file.
pub fn is_game(p: &Path) -> bool {
    p.join("main.luau").is_file() || (p.is_file() && p.extension().is_some_and(|e| e == "luau")) || (p.is_dir() && one_file_in(p).is_some())
}

impl GameToml {
    pub fn parse(text: &str) -> Result<Self, String> {
        let cfg: GameToml = toml::from_str(text).map_err(|e| format!("game.toml: {e}"))?;
        cfg.check()?;
        Ok(cfg)
    }

    /// A one-file game's settings: the `--!` lines of its leading comment
    /// block are `game.toml` (`--! title = "Orbs"`). Without a title, the
    /// file's name.
    pub fn parse_header(source: &str, stem: &str) -> Result<Self, String> {
        let toml: Vec<&str> = source.lines().map(str::trim_start).take_while(|l| l.is_empty() || l.starts_with("--"))
            .filter_map(|l| l.strip_prefix("--!")).collect();
        let text = toml.join("\n");
        let titled = toml.iter().any(|l| l.trim_start().starts_with("title"));
        let mut cfg: GameToml = toml::from_str(&text).map_err(|e| format!("--! header: {e}"))?;
        if !titled { cfg.title = stem.to_string(); }
        cfg.check()?;
        Ok(cfg)
    }

    /// A one-file game (`warp run game.luau`): its `--!` header.
    pub fn load_file(file: &Path) -> Result<Self, String> {
        let source = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Warp game".into());
        Self::parse_header(&source, &stem)
    }

    /// `DIR/game.toml`, or the defaults (titled after the folder) without one.
    pub fn load(dir: &Path) -> Result<Self, String> {
        if !dir.join("main.luau").exists() { return Err(format!("{}: not a game folder (no main.luau); `warp new {}` makes one", dir.display(), dir.display())); }
        match std::fs::read_to_string(dir.join("game.toml")) {
            Ok(text) => Self::parse(&text),
            Err(_) => {
                let title = dir.canonicalize().ok().and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "Warp game".into());
                Ok(GameToml { title, ..Default::default() })
            }
        }
    }

    fn check(&self) -> Result<(), String> {
        if !(1.0..=1000.0).contains(&self.tick_rate) { return Err(format!("game.toml: tick_rate {} (1 to 1000)", self.tick_rate)); }
        if !(1..=4).contains(&self.players) { return Err(format!("game.toml: players {} (1 to 4)", self.players)); }
        if self.bots > 7 { return Err(format!("game.toml: bots {} (0 to 7)", self.bots)); }
        if !(1..=8).contains(&self.min_players) { return Err(format!("game.toml: min_players {} (1 to 8)", self.min_players)); }
        if !self.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') { return Err(format!("game.toml: id {:?}: lowercase letters, digits and dashes", self.id)); }
        if !(1..=65536).contains(&self.max_entities) { return Err(format!("game.toml: max_entities {} (1 to 65536)", self.max_entities)); }
        if self.size[0] < 64 || self.size[1] < 64 { return Err("game.toml: size is at least [64, 64]".into()); }
        crate::sky::SkyChoice::parse(&self.sky).map_err(|e| format!("game.toml: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_file_header() {
        let c = GameToml::parse_header("-- Orbs: collect them.\n--! tick_rate = 30\n--! sky = \"night\"\n\nlocal x = 1\n--! players = 4", "orbs").unwrap();
        assert_eq!((c.title.as_str(), c.tick_rate, c.sky.as_str(), c.players), ("orbs", 30.0, "night", 1));
        assert_eq!(GameToml::parse_header("--! title = \"Orb Hunt\"", "orbs").unwrap().title, "Orb Hunt");
        assert!(GameToml::parse_header("--! nonsense = 1", "x").is_err());
        assert!(GameToml::parse_header("--! players = 9", "x").is_err());
    }

    #[test]
    fn defaults_and_errors() {
        let c = GameToml::parse("title = \"Orbs\"\ntick_rate = 30").unwrap();
        assert_eq!((c.title.as_str(), c.tick_rate, c.players), ("Orbs", 30.0, 1));
        assert!(GameToml::parse("titel = \"typo\"").unwrap_err().contains("titel"), "typos are errors");
        assert!(GameToml::parse("players = 9").is_err());
        assert!(GameToml::parse("sky = \"dusk\"").is_err());
    }
}
