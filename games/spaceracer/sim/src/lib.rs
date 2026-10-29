//! Native Space Racer simulation. Source provenance and parity gates live beside this crate.
pub mod flight;
pub mod handling;
pub mod race_rules;
pub mod track;

#[cfg(test)]
fn oracle(name: &str) -> String {
    use std::io::Read;
    let bytes: &[u8] = match name {
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
