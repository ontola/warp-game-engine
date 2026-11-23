//! Author a story as a Luau program that returns plain data. Evaluation uses
//! a fresh VM. In a browser, run it in the disposable editor worker because
//! this wasm build cannot unwind a Luau exception.
use script::{Host, Limits, mlua::{chunk::ChunkMode, Table, Value}};
use serde_json::{Map, Number, Value as Json};
use std::collections::BTreeMap;

pub const MAX_SOURCE_BYTES: usize = 64 * 1024;
pub const MAX_DATA_BYTES: usize = 64 * 1024;
const MAX_VALUES: usize = 4096;
const MAX_DEPTH: usize = 24;

#[derive(Default)]
struct Budget { values: usize, bytes: usize }

impl Budget {
    fn node(&mut self, depth: usize) -> Result<(), String> {
        self.values += 1;
        if depth > MAX_DEPTH || self.values > MAX_VALUES { return Err("story data is too large or deeply nested".into()); }
        Ok(())
    }

    fn text(&mut self, bytes: usize) -> Result<(), String> {
        self.bytes += bytes;
        if self.bytes > MAX_DATA_BYTES { return Err("story data exceeds 64 KiB of text".into()); }
        Ok(())
    }

    fn read(&mut self, value: Value, path: &mut Vec<usize>) -> Result<Json, String> {
        self.node(path.len())?;
        match value {
            Value::Boolean(b) => Ok(b.into()),
            Value::Integer(n) => Ok(n.into()),
            Value::Number(n) => Number::from_f64(n).map(Json::Number).ok_or_else(|| "story numbers must be finite".into()),
            Value::String(s) => {
                self.text(s.as_bytes().len())?;
                Ok(Json::String(s.to_str().map_err(|_| "story strings must be UTF-8")?.to_string()))
            }
            Value::Vector(v) if [v.x(), v.y(), v.z()].iter().all(|f| f.is_finite()) =>
                Ok(serde_json::json!({"x":v.x(), "y":v.y(), "z":v.z()})),
            Value::Table(t) => {
                let id = t.to_pointer() as usize;
                if path.contains(&id) { return Err("story data contains a cycle".into()); }
                if t.metatable().is_some() { return Err("story tables must contain plain data".into()); }
                path.push(id);
                let result = self.table(t, path);
                path.pop();
                result
            }
            other => Err(format!("a story cannot contain {} values", other.type_name())),
        }
    }

    fn table(&mut self, t: Table, path: &mut Vec<usize>) -> Result<Json, String> {
        let mut fields = Map::new();
        let mut items = BTreeMap::new();
        for kv in t.pairs::<Value, Value>() {
            let (k, v) = kv.map_err(|e| script::short(&e))?;
            let v = self.read(v, path)?;
            match k {
                Value::String(s) if items.is_empty() => {
                    self.text(s.as_bytes().len())?;
                    fields.insert(s.to_str().map_err(|_| "story keys must be UTF-8")?.to_string(), v);
                }
                Value::Integer(i) if fields.is_empty() && i > 0 && i as usize <= MAX_VALUES => { items.insert(i as usize, v); }
                Value::Number(n) if fields.is_empty() && n.fract() == 0. && n >= 1. && n <= MAX_VALUES as f64 => { items.insert(n as usize, v); }
                _ => return Err("use either named fields or a sequence of items in each story table".into()),
            }
        }
        if items.is_empty() { return Ok(Json::Object(fields)); }
        if items.last_key_value().map(|(&k, _)| k) != Some(items.len()) { return Err("story arrays must be contiguous from 1".into()); }
        Ok(Json::Array(items.into_values().collect()))
    }

    fn check(&mut self, value: &Json, depth: usize) -> Result<(), String> {
        self.node(depth)?;
        match value {
            Json::Object(map) => for (k, v) in map { self.text(k.len())?; self.check(v, depth + 1)?; },
            Json::Array(list) => for v in list { self.check(v, depth + 1)?; },
            Json::String(s) => self.text(s.len())?,
            Json::Null => return Err("story data cannot contain null".into()),
            _ => {}
        }
        Ok(())
    }
}

/// Validate the worker's output again before allocating any live game tables.
pub fn validate_data(data: &Json) -> Result<(), String> {
    if !data.is_object() { return Err("return a story table with named fields".into()); }
    Budget::default().check(data, 0)
}

pub fn evaluate(source: &str) -> Result<Json, String> {
    if source.len() > MAX_SOURCE_BYTES { return Err("story source exceeds 64 KiB".into()); }
    if source.trim().is_empty() { return Err("enter a Luau program that returns a story table".into()); }
    #[cfg(target_arch = "wasm32")]
    script::web::init();
    let host = Host::new(Limits { memory: 4 << 20, steps: 25_000 }, |_, _| Ok(()))?;
    let lua = host.lua();
    let data = (|| -> script::mlua::Result<Value> {
        let env = lua.create_table()?;
        for name in ["assert", "error", "ipairs", "next", "pairs", "select", "tonumber", "tostring", "type", "typeof",
            "math", "string", "table", "utf8", "vector", "bit32"] {
            env.raw_set(name, host.console().get::<Value>(name)?)?;
        }
        env.set_readonly(true);
        lua.load(source).set_name("=story.luau").set_mode(ChunkMode::Text).set_environment(env).into_function()?.call(())
    })().map_err(|e| script::short(&e))?;
    let result = Budget::default().read(data, &mut Vec::new())?;
    validate_data(&result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computed_routes_and_vectors_cross_as_plain_bounded_data() {
        let d = evaluate(r#"
            local route = {}
            for i = 1, 3 do table.insert(route, { target = vector.create(i, 0, i * 2) }) end
            return { title = "Local run", route = route }
        "#).unwrap();
        assert_eq!(d["route"][2]["target"], serde_json::json!({"x":3.0,"y":0.0,"z":6.0}));
        assert!(evaluate("assert(world==nil and require==nil and os==nil and debug==nil and getfenv==nil); return {title='OK'}").is_ok());
    }

    #[test]
    fn rejects_bad_code_budgets_cycles_and_nondata() {
        for (source, message) in [
            ("return {", "story.luau"), ("while true do end", "step budget"),
            ("local a={} a.x=a return a", "cycle"), ("return {f=function() end}", "function"),
            ("return { n=0/0 }", "finite"), ("return { [100]=1 }", "contiguous"),
            ("return { s=string.rep('a', 70000) }", "64 KiB"),
        ] { let error = evaluate(source).unwrap_err(); assert!(error.contains(message), "{error}"); }
        assert!(evaluate("return {title='a repaired story'}").is_ok());
    }
}
