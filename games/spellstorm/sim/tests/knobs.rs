//! Its own test binary: knobs are process-wide, so changing them here
//! cannot disturb the sim tests. Its tests take turns (LOCK) for the same
//! reason.

use spellstorm::knobs::*;
use spellstorm::{Seat, Settings, World};
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

#[test]
fn a_knobs_file_round_trips_and_reports_bad_lines() {
    let _turn = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut text = file_text();
    text = text.replace("wizard_speed = 5.2", "wizard_speed = 7");
    text += "nope = 1\nroll_speed = fast\n";
    let errs = load(&text);
    assert_eq!(wizard_speed(), 7.0);
    assert_eq!(roll_speed(), 12.5);
    assert_eq!(errs.len(), 2, "{errs:?}");
    reset();
    assert_eq!(wizard_speed(), 5.2);
}

/// One new player with the shipped knobs.txt: only the walls past the two
/// clearings' fights, no practice strip walls every ten metres, and they
/// start with their first two elements. (Here, not in run.rs: loading the
/// shipped knobs there changed them under the other tests' worlds.)
#[test]
fn a_new_player_meets_only_the_clearing_walls() {
    let _turn = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let errors = load(include_str!("../../knobs.txt"));
    assert!(errors.is_empty(), "{errors:?}");
    let w = World::new(&[Seat::player(0, "Joep")], 7, Settings::default());
    assert!(w.run.practice.is_none());
    assert_eq!(w.run.wards.len(), 2, "{:?}", w.run.wards.iter().map(|x| x.c).collect::<Vec<_>>());
    assert!(w.run.wards.iter().all(|x| x.check.is_none()));
    reset();
}
