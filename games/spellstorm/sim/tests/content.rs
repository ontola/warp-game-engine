//! Monster kinds and biomes come from mods/base/monsters.luau and
//! biomes.luau and reload while the game runs: values change at once, a
//! changed list of kinds or biomes waits for a restart.
use spellstorm::biome::biomes;
use spellstorm::enemy::{kind, kinds};
use spellstorm::rules;
use std::path::PathBuf;

fn mods_copy() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("spellstorm-monsters-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("base")).unwrap();
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../mods/base");
    for f in ["biomes.luau", "monsters.luau", "spells.luau", "tomes.luau"] {
        std::fs::copy(src.join(f), dir.join("base").join(f)).unwrap();
    }
    dir
}

#[test]
fn monster_numbers_reload_and_the_list_waits() {
    let built = kinds().len();
    assert_eq!(kinds().last().unwrap().key, "none");
    let ghoul = kind("ghoul");
    assert_eq!(kinds()[ghoul].hp, 40.0);

    let dir = mods_copy();
    let path = dir.join("base/monsters.luau");
    let lines = rules::install(Some(&dir)).expect("the base rules load");
    assert!(!lines.iter().any(|l| l.contains("rror")), "{lines:?}");
    assert_eq!(kinds().len(), built);

    // A changed number applies at once.
    let src = std::fs::read_to_string(&path).unwrap();
    let tougher = src.replace("{ key = \"ghoul\", name = \"Ghoul\", speed", "{ key = \"ghoul\", name = \"Ghoul\", hp = 4000, speed");
    assert_ne!(src, tougher);
    std::fs::write(&path, &tougher).unwrap();
    rules::poll();
    assert_eq!(kinds()[ghoul].hp, 4000.0);

    // A bad value keeps the kinds that ran and says why.
    std::fs::write(&path, tougher.replace("attack = \"slam\"", "attack = \"hug\"")).unwrap();
    rules::poll();
    assert!(rules::status().contains("\"hug\" is not melee"), "{}", rules::status());
    assert_eq!(kinds()[ghoul].hp, 4000.0);

    // Dropping a kind would move every index: it waits for the next start.
    let fewer: String = tougher.lines().filter(|l| !l.contains("key = \"shambler\"")).map(|l| format!("{l}\n")).collect();
    assert!(fewer.len() < tougher.len());
    std::fs::write(&path, fewer).unwrap();
    rules::poll();
    assert!(rules::status().contains("next start"), "{}", rules::status());
    assert_eq!(kinds()[ghoul].key, "ghoul");

    std::fs::write(&path, &tougher).unwrap();
    // A biome whose boss is not a boss is refused as a whole.
    let bpath = dir.join("base/biomes.luau");
    let bsrc = std::fs::read_to_string(&bpath).unwrap();
    std::fs::write(&bpath, bsrc.replace("boss = \"gravekeeper\"", "boss = \"ghoul\"")).unwrap();
    rules::poll();
    assert!(rules::status().contains("is not a boss"), "{}", rules::status());
    assert_eq!(biomes()[0].boss, "gravekeeper");
    // A changed colour applies.
    std::fs::write(&bpath, bsrc.replacen("fog = { 0.08, 0.1, 0.12 }", "fog = { 0.5, 0.1, 0.12 }", 1)).unwrap();
    rules::poll();
    assert_eq!(biomes()[0].fog[0], 0.5);

    rules::uninstall();
    assert_eq!(biomes()[0].fog[0], 0.08);
    assert_eq!(kinds()[ghoul].hp, 40.0);
    let _ = std::fs::remove_dir_all(&dir);
}
