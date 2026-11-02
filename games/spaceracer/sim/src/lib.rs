//! Native Space Racer simulation. Source provenance and parity gates live beside this crate.
pub mod chase;
mod curve;
mod contacts;
pub mod flight;
pub mod generation;
pub mod handling;
pub mod motion;
pub mod race_rules;
pub mod race;
pub mod rules;
pub mod track;

#[cfg(test)]
fn oracle(name: &str) -> String {
    use std::io::Read;
    let bytes: &[u8] = match name {
        "ships" => include_bytes!("../../assets/ships.json.gz"),
        "landing" => include_bytes!("../../tests/fixtures/landing.json.gz"),
        "chase" => include_bytes!("../../tests/fixtures/chase.json.gz"),
        "generation" => include_bytes!("../../tests/fixtures/generation.json.gz"),
        "rules" => include_bytes!("../../tests/fixtures/rules.json.gz"),
        "handling" => include_bytes!("../../tests/fixtures/handling.json.gz"),
        "flight" => include_bytes!("../../tests/fixtures/flight.json.gz"),
        "tracks" => include_bytes!("../../tests/fixtures/tracks.json.gz"),
        _ => panic!("unknown fixture"),
    };
    let mut raw = String::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_string(&mut raw)
        .unwrap();
    raw
}

#[cfg(test)]
mod asset_tests;
pub mod environment;

pub mod victory;
mod missile;

pub mod combat;

pub mod track_collision;
