//! Versioned JSON documents for native Warp games. Each write checks the
//! reader's revision and publishes a complete document in one SQLite transaction.
use directories::ProjectDirs;
use jsonschema::{Validator, draft202012};
use mlua::{Lua, LuaSerdeExt, Table, Value};
#[cfg(not(target_arch = "wasm32"))]
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::Value as Json;
use std::{collections::BTreeMap, fs, path::Path};

pub enum Location<'a> {
    Memory,
    User,
    Directory(&'a Path),
}

/// A browser build keeps documents in memory only (no SQLite there yet):
/// (kind, slot) to (version, revision, JSON text).
#[cfg(target_arch = "wasm32")]
type Connection = BTreeMap<(String, String), (u32, i64, String)>;

pub struct Storage {
    db: Connection,
    game: String,
    schemas: BTreeMap<String, Vec<Validator>>,
}

#[derive(Serialize)]
pub struct Document {
    pub version: u32,
    pub revision: i64,
    pub data: Json,
}

type Error = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;

// Splitscreen Club publishes a document with one object definition. A JSON Schema
// validator only applies that definition when the root references it.
fn read_schema(path: &Path, kind: &str) -> Result<Json> {
    let mut schema: Json =
        serde_json::from_slice(&fs::read(path)?).map_err(|e| format!("{}: {e}", path.display()))?;
    if kind == "save"
        && let Some(root) = schema.as_object_mut()
    {
        if let Some(defs) = root.get("$defs").and_then(Json::as_object)
            && defs.len() == 1
            && root.keys().all(|key| {
                matches!(
                    key.as_str(),
                    "$schema" | "$id" | "$comment" | "$defs" | "title" | "description" | "examples"
                ) || key.starts_with("x-")
                    || (key == "type" && root[key] == "object")
            })
            && let Some((name, def)) = defs.iter().next()
            && def.get("type").is_none_or(|ty| ty == "object")
            && def.get("properties").is_some()
        {
            // A local anchor expressed as a URI fragment of a JSON Pointer.
            let token = name.replace('~', "~0").replace('/', "~1");
            let fragment: String = token
                .bytes()
                .map(|b| match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                        char::from(b).to_string()
                    }
                    _ => format!("%{b:02X}"),
                })
                .collect();
            root.insert("type".into(), "object".into());
            root.insert("$ref".into(), format!("#/$defs/{fragment}").into());
        }
    }
    Ok(schema)
}

fn schema_documents(dir: &Path) -> Result<BTreeMap<String, Vec<Json>>> {
    let mut documents = BTreeMap::new();
    for kind in ["save", "settings", "profile"] {
        let current = dir.join(format!("{kind}-schema.json"));
        if !current.try_exists()? {
            continue;
        }
        let history = dir.join("schema-history").join(kind);
        let mut versions = Vec::new();
        if history.try_exists()? {
            let mut numbered = BTreeMap::new();
            for entry in fs::read_dir(history)? {
                let path = entry?.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let version: usize = path
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.parse().ok())
                    .ok_or_else(|| format!("expected a numbered schema: {}", path.display()))?;
                if numbered.insert(version, path).is_some() {
                    return Err(format!("duplicate {kind} v{version}").into());
                }
            }
            for (version, path) in numbered {
                if version != versions.len() + 1 {
                    return Err(
                        format!("gap in {kind} schema history at v{}", versions.len() + 1).into(),
                    );
                }
                versions.push(read_schema(&path, kind)?);
            }
        }
        versions.push(read_schema(&current, kind)?);
        documents.insert(kind.to_owned(), versions);
    }
    let legacy = dir.join("storage.json");
    if legacy.try_exists()? {
        if !documents.is_empty() {
            return Err("remove storage.json after moving its schemas to standalone files".into());
        }
        return Ok(serde_json::from_slice(&fs::read(legacy)?)?);
    }
    if documents.is_empty() {
        return Err("storage enabled without a save-schema.json, settings-schema.json or profile-schema.json".into());
    }
    Ok(documents)
}

impl Storage {
    pub fn open(dir: &Path, game: &str, location: Location<'_>) -> Result<Self> {
        if game.is_empty() {
            return Err("persistent storage needs a stable id in game.toml".into());
        }
        let manifest = schema_documents(dir)?;
        let mut schemas = BTreeMap::new();
        for (kind, versions) in manifest {
            if !matches!(kind.as_str(), "save" | "profile" | "settings") || versions.is_empty() {
                return Err(format!("invalid kind or empty schema list: {kind}").into());
            }
            let mut compiled = Vec::new();
            for (i, schema) in versions.iter().enumerate() {
                draft202012::meta::validate(schema)
                    .map_err(|e| format!("{kind} v{}: {e}", i + 1))?;
                compiled.push(draft202012::new(schema)?);
            }
            schemas.insert(kind, compiled);
        }
        let directory = match location {
            Location::Memory => None,
            Location::Directory(dir) => Some(dir.to_owned()),
            Location::User => Some(
                ProjectDirs::from("io", "Ontola", "Warp")
                    .ok_or("no user data directory")?
                    .data_local_dir()
                    .to_owned(),
            ),
        };
        #[cfg(target_arch = "wasm32")]
        let db = { let _ = directory; Connection::new() };
        #[cfg(not(target_arch = "wasm32"))]
        let db = if let Some(root) = directory {
            fs::create_dir_all(&root)?;
            Connection::open(root.join("storage.sqlite3"))?
        } else {
            Connection::open_in_memory()?
        };
        #[cfg(not(target_arch = "wasm32"))]
        db.execute_batch(
            "PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS documents (
                game TEXT NOT NULL, kind TEXT NOT NULL, slot TEXT NOT NULL,
                version INTEGER NOT NULL, revision INTEGER NOT NULL, data TEXT NOT NULL,
                PRIMARY KEY (game, kind, slot)
            ) WITHOUT ROWID;",
        )?;
        Ok(Self {
            db,
            schemas,
            game: game.into(),
        })
    }

    fn versions(&self, kind: &str, slot: &str) -> Result<&[Validator]> {
        if slot.is_empty() || slot.len() > 128 {
            return Err("slot needs 1 to 128 UTF-8 bytes".into());
        }
        self.schemas
            .get(kind)
            .map(Vec::as_slice)
            .ok_or_else(|| format!("undeclared storage kind: {kind}").into())
    }

    pub fn read(&self, kind: &str, slot: &str) -> Result<Option<Document>> {
        let versions = self.versions(kind, slot)?;
        #[cfg(target_arch = "wasm32")]
        let row = self.db.get(&(kind.to_owned(), slot.to_owned())).cloned();
        #[cfg(not(target_arch = "wasm32"))]
        let row = self
            .db
            .query_row(
                "SELECT version, revision, data FROM documents WHERE game=? AND kind=? AND slot=?",
                params![self.game, kind, slot],
                |r| Ok((r.get::<_, u32>(0)?, r.get(1)?, r.get::<_, String>(2)?)),
            )
            .optional()?;
        let Some((version, revision, text)) = row else {
            return Ok(None);
        };
        let schema = version
            .checked_sub(1)
            .and_then(|i| versions.get(i as usize))
            .ok_or("unsupported stored schema version")?;
        let data = serde_json::from_str(&text)?;
        schema
            .validate(&data)
            .map_err(|e| format!("invalid stored document: {e}"))?;
        Ok(Some(Document {
            version,
            revision,
            data,
        }))
    }

    pub fn write(&mut self, kind: &str, slot: &str, expected: i64, data: Json) -> Result<i64> {
        let versions = self.versions(kind, slot)?;
        let version = u32::try_from(versions.len())?;
        versions[version as usize - 1]
            .validate(&data)
            .map_err(|e| format!("{kind} v{version}: {e}"))?;
        let text = serde_json::to_string(&data)?;
        if text.len() > 1 << 20 {
            return Err("document exceeds 1 MiB".into());
        }
        #[cfg(target_arch = "wasm32")]
        {
            let key = (kind.to_owned(), slot.to_owned());
            let (revision, old_version) = self.db.get(&key).map_or((0, 0), |d| (d.1, d.0));
            if revision != expected {
                return Err("storage revision conflict: read the slot again".into());
            }
            if old_version > version {
                return Err("cannot overwrite a newer schema version".into());
            }
            let next = revision.checked_add(1).ok_or("revision exhausted")?;
            self.db.insert(key, (version, next, text));
            return Ok(next);
        }
        #[cfg(not(target_arch = "wasm32"))]
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        #[cfg(not(target_arch = "wasm32"))]
        let current: Option<(i64, u32)> = tx
            .query_row(
                "SELECT revision, version FROM documents WHERE game=? AND kind=? AND slot=?",
                params![self.game, kind, slot],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (revision, old_version) = current.unwrap_or((0, 0));
            if revision != expected {
                return Err("storage revision conflict: read the slot again".into());
            }
            if old_version > version {
                return Err("cannot overwrite a newer schema version".into());
            }
            let next = revision.checked_add(1).ok_or("revision exhausted")?;
            tx.execute("INSERT INTO documents(game,kind,slot,revision,version,data) VALUES(?,?,?,?,?,?)
                ON CONFLICT(game,kind,slot) DO UPDATE SET revision=excluded.revision, version=excluded.version, data=excluded.data",
                params![self.game,kind,slot,next,version,text])?;
            tx.commit()?;
            Ok(next)
        }
    }
}

// Preserve JSON objects, dense arrays and null, and reject values that a
// best-effort diagnostic serializer could silently truncate or reinterpret.
fn json(
    lua: &Lua,
    v: Value,
    depth: usize,
    remaining: &mut usize,
    bytes: &mut usize,
) -> mlua::Result<Json> {
    let fail = mlua::Error::runtime;
    *remaining = remaining
        .checked_sub(1)
        .ok_or_else(|| fail("too many JSON values"))?;
    if depth > 64 {
        return Err(fail("JSON is too deep or cyclic"));
    }
    match v {
        Value::Boolean(b) => Ok(b.into()),
        Value::Integer(n) => Ok(n.into()),
        Value::Number(n) => serde_json::Number::from_f64(n)
            .map(Json::Number)
            .ok_or_else(|| fail("non-finite number")),
        Value::String(s) => {
            *bytes = bytes
                .checked_sub(s.as_bytes().len())
                .ok_or_else(|| fail("document exceeds 1 MiB"))?;
            Ok(s.to_str()?.to_string().into())
        }
        v if v == lua.null() => Ok(Json::Null),
        Value::Table(t) => {
            let array = t.raw_len() > 0 || t.metatable().as_ref() == Some(&lua.array_metatable());
            let mut object = serde_json::Map::new();
            let mut items = Vec::new();
            for pair in t.pairs::<Value, Value>() {
                let (key, value) = pair?;
                let value = json(lua, value, depth + 1, remaining, bytes)?;
                match (array, key) {
                    (false, Value::String(k)) => {
                        *bytes = bytes
                            .checked_sub(k.as_bytes().len())
                            .ok_or_else(|| fail("document exceeds 1 MiB"))?;
                        object.insert(k.to_str()?.to_string(), value);
                    }
                    (true, Value::Integer(i)) if i >= 1 => items.push((i, value)),
                    _ => return Err(fail("JSON requires string object keys or a dense array")),
                }
            }
            if !array {
                return Ok(Json::Object(object));
            }
            items.sort_unstable_by_key(|(i, _)| *i);
            if !items
                .iter()
                .enumerate()
                .all(|(n, (i, _))| *i as usize == n + 1)
            {
                return Err(fail("JSON array has missing indices"));
            }
            Ok(Json::Array(items.into_iter().map(|(_, v)| v).collect()))
        }
        _ => Err(fail("expected JSON data")),
    }
}

pub(crate) fn register(lua: &Lua, api: &Table, core: crate::api::Shared) -> mlua::Result<()> {
    use crate::api::{lock, sim};
    let storage = lua.create_table()?;
    let reader = core.clone();
    storage.raw_set(
        "read",
        lua.create_function(move |lua, (kind, slot): (String, String)| {
            let c = lock(&reader);
            sim(&c, "storage.read")?;
            let db = c
                .storage
                .as_ref()
                .ok_or_else(|| mlua::Error::runtime("enable storage in game.toml"))?;
            match db.read(&kind, &slot).map_err(mlua::Error::external)? {
                Some(record) => lua.to_value(&record),
                None => Ok(Value::Nil),
            }
        })?,
    )?;
    storage.raw_set(
        "write",
        lua.create_function(
            move |lua, (kind, slot, revision, value): (String, String, i64, Value)| {
                let mut c = lock(&core);
                sim(&c, "storage.write")?;
                let db = c
                    .storage
                    .as_mut()
                    .ok_or_else(|| mlua::Error::runtime("enable storage in game.toml"))?;
                let mut remaining = 65536;
                db.write(
                    &kind,
                    &slot,
                    revision,
                    json(lua, value, 0, &mut remaining, &mut (1 << 20))?,
                )
                .map_err(mlua::Error::external)
            },
        )?,
    )?;
    storage.raw_set("null", lua.null())?;
    storage.raw_set(
        "array",
        lua.create_function(|lua, ()| {
            let t = lua.create_table()?;
            t.set_metatable(Some(lua.array_metatable()))?;
            Ok(t)
        })?,
    )?;
    storage.set_readonly(true);
    api.raw_set("storage", storage)
}
