//! The Luau base rules (mods/base) are the built-in Rust ones, ported: the
//! same spell for every queue and form, the same effects, the same tomes.
//! When the Luau rules change on purpose, change the Rust ones with them
//! (or drop the Rust ones and this test).
use spellstorm::el::{self, El};
use spellstorm::grammar::{builtin, compose, FORMS};
use spellstorm::relics::{self, Mods};
use spellstorm::rules;

fn queues() -> Vec<Vec<El>> {
    let mut out = vec![vec![]];
    for &a in &el::ALL {
        out.push(vec![a]);
        for &b in &el::ALL {
            out.push(vec![a, b]);
            for &c in &el::ALL {
                out.push(vec![a, b, c]);
            }
        }
    }
    out
}

#[test]
fn the_luau_base_rules_are_the_rust_ones() {
    // The built-in tomes and effects, before Luau replaces them.
    let rust_tomes: Vec<Mods> = (0..relics::all().len()).map(|i| Mods::of(&[i])).collect();
    let rust_names: Vec<(String, String)> = relics::all().iter().map(|t| (t.name.clone(), t.text.clone())).collect();
    let rust_effects: Vec<_> = el::ALL.iter().map(|e| e.effect()).collect();
    let lines = rules::install(None).expect("the base rules load");
    assert!(rules::installed());
    assert!(!lines.iter().any(|l| l.contains("rror")), "{lines:?}");

    let lua_effects: Vec<_> = el::ALL.iter().map(|e| e.effect()).collect();
    assert_eq!(rust_effects, lua_effects);
    let lua_names: Vec<(String, String)> = relics::all().iter().map(|t| (t.name.clone(), t.text.clone())).collect();
    assert_eq!(rust_names, lua_names);
    let lua_tomes: Vec<Mods> = (0..relics::all().len()).map(|i| Mods::of(&[i])).collect();
    assert_eq!(rust_tomes, lua_tomes);

    let mut mods = vec![Mods::default()];
    mods.extend(lua_tomes.iter().copied());
    let mut checked = 0;
    for q in queues() {
        for form in FORMS {
            for m in &mods {
                let (a, b) = (compose(&q, form, m), builtin(&q, form, m));
                assert_eq!(a, b, "{q:?} {form:?} {m:?}");
                checked += 1;
            }
        }
    }
    assert!(checked > 100_000);
    assert!(!rules::status().contains("rror"), "{}", rules::status());
}
