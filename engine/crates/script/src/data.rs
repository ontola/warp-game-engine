//! Data in Luau: a module that returns tables of numbers and names (cars,
//! weapons, levels), read into a Rust type with serde. The file runs in the
//! same sandbox as every script (no clock, no files, a step budget), so it
//! can compute values, share a DEFAULT row with `table.clone`, or loop.
//! Read it again at the game's natural boundary (next round, next stage)
//! and a saved edit applies there; engine/docs/luau-or-rust.md, "Live
//! reload".
use crate::{Host, Limits};
use mlua::{LuaSerdeExt, Value};
use serde::de::DeserializeOwned;

/// The field `key` of what module `name` (its source) returns, as `T`.
/// Errors name the file and line, or the field serde could not read.
pub fn read<T: DeserializeOwned>(name: &str, source: &str, key: &str) -> Result<T, String> {
    let mut host = Host::new(Limits::default(), |_, _| Ok(()))?;
    host.add_source(name, source)?;
    let exports = host.modules()[0].exports.clone().ok_or_else(|| format!("{name}: does not return a table"))?;
    let v: Value = exports.raw_get(key).map_err(|e| format!("{name}: {key}: {}", crate::short(&e)))?;
    if v.is_nil() {
        return Err(format!("{name}: returns no `{key}`"));
    }
    host.unguarded(|lua| lua.from_value(v)).map_err(|e| format!("{name}: {key}: {}", crate::short(&e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize, Debug, PartialEq)]
    struct Car {
        name: String,
        mass: f64,
        drive: [f64; 2],
    }

    #[test]
    fn a_luau_table_reads_into_a_struct() {
        let src = "local base = { mass = 1230, drive = { 0.5, 0.5 } }\nlocal a = table.clone(base)\na.name = 'A'\nreturn { cars = { a } }";
        let cars: Vec<Car> = read("cars.luau", src, "cars").unwrap();
        assert_eq!(cars, vec![Car { name: "A".into(), mass: 1230.0, drive: [0.5, 0.5] }]);
        assert!(read::<Vec<Car>>("cars.luau", src, "trucks").unwrap_err().contains("returns no `trucks`"));
        assert!(read::<Vec<Car>>("cars.luau", "return { cars = { { name = 'B' } } }", "cars").unwrap_err().contains("mass"));
        assert!(read::<Vec<Car>>("cars.luau", "return {", "cars").unwrap_err().contains("cars.luau:1"));
    }
}
