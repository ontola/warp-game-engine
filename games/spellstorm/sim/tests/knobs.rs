//! Its own test binary: knobs are process-wide, so changing them here
//! cannot disturb the sim tests.

use spellstorm::knobs::*;

#[test]
fn a_knobs_file_round_trips_and_reports_bad_lines() {
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
