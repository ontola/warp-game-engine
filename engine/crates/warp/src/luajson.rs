//! Luau values as JSON, for `state()` and the console: tables with keys
//! 1..n become arrays, other tables objects (keys sorted), vectors
//! `[x, y, z]`; functions and other non-data values are left out.
use script::mlua::{Table, Value};
use serde_json::{json, Map, Value as Json};

const DEPTH: usize = 24;

pub fn to_json(v: &Value) -> Json { value(v, 0) }

fn number(x: f64) -> Json {
    if !x.is_finite() { return Json::Null; }
    if x.fract() == 0.0 && x.abs() < 9.0e15 { json!(x as i64) } else { json!(x) }
}

fn value(v: &Value, depth: usize) -> Json {
    match v {
        Value::Nil => Json::Null,
        Value::Boolean(b) => json!(b),
        Value::Integer(i) => json!(i),
        Value::Number(x) => number(*x),
        Value::Vector(v) => json!([number(v.x() as f64), number(v.y() as f64), number(v.z() as f64)]),
        Value::String(s) => json!(s.to_string_lossy()),
        Value::Table(t) if depth < DEPTH => table(t, depth + 1),
        Value::Table(_) => json!("(too deep)"),
        _ => Json::Null,
    }
}

fn data(v: &Value) -> bool { !matches!(v, Value::Function(_) | Value::Thread(_) | Value::UserData(_) | Value::LightUserData(_) | Value::Error(_)) }

fn table(t: &Table, depth: usize) -> Json {
    let n = t.raw_len();
    let pairs: Vec<(Value, Value)> = t.pairs::<Value, Value>().filter_map(Result::ok).filter(|(_, v)| data(v)).collect();
    if n > 0 && pairs.len() == n {
        return Json::Array((1..=n).map(|i| value(&t.raw_get::<Value>(i).unwrap_or(Value::Nil), depth)).collect());
    }
    let mut m = Map::new();
    let mut keyed: Vec<(String, &Value)> = pairs.iter().filter_map(|(k, v)| {
        let key = match k {
            Value::String(s) => s.to_string_lossy(),
            Value::Integer(i) => i.to_string(),
            Value::Number(x) => x.to_string(),
            Value::Boolean(b) => b.to_string(),
            _ => return None,
        };
        Some((key, v))
    }).collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    for (k, v) in keyed { m.insert(k, value(v, depth)); }
    Json::Object(m)
}

/// A console result: JSON for tables and vectors, the value's text otherwise.
pub fn show(v: &Value) -> String {
    match v {
        Value::Table(_) | Value::Vector(_) => serde_json::to_string(&to_json(v)).unwrap_or_default(),
        Value::String(s) => s.to_string_lossy(),
        Value::Nil => "nil".into(),
        other => other.to_string().unwrap_or_else(|_| other.type_name().into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use script::mlua::Lua;

    #[test]
    fn tables_become_arrays_and_objects() {
        let lua = Lua::new();
        let v: Value = lua.load("return { score = 3, list = {1, 2.5, 'x'}, pos = vector.create(1, 2, 3), f = function() end, empty = {}, [2] = true }").eval().unwrap();
        assert_eq!(to_json(&v), json!({ "score": 3, "list": [1, 2.5, "x"], "pos": [1, 2, 3], "empty": {}, "2": true }));
        let cyc: Value = lua.load("local t = {} t.me = t return t").eval().unwrap();
        assert!(serde_json::to_string(&to_json(&cyc)).unwrap().contains("too deep"));
    }
}
