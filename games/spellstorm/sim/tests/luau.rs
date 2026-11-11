//! The grammar, effects and tomes have one copy, mods/base: the built-in
//! files answer before any rules are installed, and installing the base
//! folder answers the same for every queue, form and tome.
use spellstorm::el::{self, El};
use spellstorm::grammar::{compose, FORMS};
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

fn everything() -> (Vec<String>, Vec<el::Effect>, Vec<(String, String)>) {
    let tomes: Vec<Mods> = (0..relics::all().len()).map(|i| Mods::of(&[i])).collect();
    let mut mods = vec![Mods::default()];
    mods.extend(tomes);
    let mut spells = vec![];
    for q in queues() {
        for form in FORMS {
            for m in &mods {
                spells.push(format!("{:?}", compose(&q, form, m)));
            }
        }
    }
    let effects = el::ALL.iter().map(|e| e.effect()).collect();
    let names = relics::all().iter().map(|t| (t.name.clone(), t.text.clone())).collect();
    (spells, effects, names)
}

#[test]
fn installed_base_rules_answer_like_the_built_in_ones() {
    assert!(!rules::installed());
    let built_in = everything();
    assert!(built_in.0.len() > 100_000);
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../mods");
    let lines = rules::install(Some(&dir)).expect("the base rules load");
    assert!(rules::installed());
    assert!(!lines.iter().any(|l| l.contains("rror")), "{lines:?}");
    let installed = everything();
    assert_eq!(built_in.1, installed.1);
    assert_eq!(built_in.2, installed.2);
    assert!(built_in.0 == installed.0, "a spell differs between the built-in and installed base rules");
    assert!(!rules::status().contains("rror"), "{}", rules::status());
}
