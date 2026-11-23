//! Local library sources watched with the hook scripts, indexed by game-relative name.
use super::{Function, Lua, Path, PathBuf, SystemTime, Table, Value, env};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Libraries(BTreeMap<String, Library>, BTreeMap<String, &'static str>);

struct Library {
    stamp: Option<(SystemTime, u64)>,
    source: Result<String, String>,
}

impl Libraries {
    pub fn refresh(&mut self, root: &Path, files: &mut Vec<(String, PathBuf)>) -> BTreeSet<String> {
        let mut present = BTreeSet::new();
        let mut changed = BTreeSet::new();
        files.retain(|(path, file)| {
            let Some(name) = path.strip_suffix(".module.luau") else {
                return true;
            };
            present.insert(name.to_owned());
            let stamp = vfs::stamp(file);
            if stamp.is_none() || !self.0.get(name).is_some_and(|lib| lib.stamp == stamp) {
                // A browser (and the Android host, whose game files are in
                // memory) has no real paths to resolve: its files are only
                // the game's own.
                #[cfg(any(target_arch = "wasm32", target_os = "android"))]
                let source = { let _ = root; vfs::read_to_string(file).map_err(|err| format!("{path}: {err}")) };
                #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
                let source = file
                    .canonicalize()
                    .and_then(|resolved| {
                        if !resolved.starts_with(root.canonicalize()?) {
                            return Err(std::io::Error::other("library outside the game folder"));
                        }
                        std::fs::read_to_string(resolved)
                    })
                    .map_err(|err| format!("{path}: {err}"));
                self.0.insert(name.to_owned(), Library { stamp, source });
                changed.insert(name.to_owned());
            }
            false
        });
        self.0.retain(|name, _| {
            if present.contains(name) {
                true
            } else {
                changed.insert(name.clone());
                false
            }
        });
        changed
    }

    /// A library compiled into the engine (the game kit): `require(name)`
    /// finds it when the game has no local library of that name, so a game
    /// can copy one into its folder and change it.
    pub fn builtin(&mut self, name: &str, source: &'static str) { self.1.insert(name.to_owned(), source); }

    pub fn forget(&mut self, file: &str) {
        if let Some(lib) = file
            .strip_suffix(".module.luau")
            .and_then(|name| self.0.get_mut(name))
        {
            lib.stamp = None;
        }
    }

    /// Each entry load gets a cache. Functions captured by a previous version keep
    /// their original libraries, including when a replacement fails halfway through.
    pub fn eval(
        &self,
        lua: &Lua,
        base: &Table,
        entry: &Table,
        name: &str,
        source: &str,
        dependencies: &mut BTreeSet<String>,
    ) -> mlua::Result<Value> {
        let cache = lua.create_table()?;
        let loading = RefCell::new(BTreeSet::new());
        let reads = RefCell::new(dependencies);
        let functions = lua.create_table()?;
        lua.scope(|scope| {
            let require = scope.create_function(|lua, name: String| {
                reads.borrow_mut().insert(name.clone());
                if let Some(exports) = cache.raw_get::<Option<Table>>(name.as_str())? {
                    return Ok(exports);
                }
                let src: &str = match (self.0.get(&name), self.1.get(&name)) {
                    (Some(lib), _) => lib.source.as_deref().map_err(mlua::Error::runtime)?,
                    (None, Some(src)) => src,
                    (None, None) => return Err(mlua::Error::runtime(format!("unknown local library {name}.module.luau"))),
                };
                if !loading.borrow_mut().insert(name.clone()) {
                    return Err(mlua::Error::runtime(format!("cyclic require of {name}")));
                }
                let result = (|| {
                    let globals = env(lua, base, &lua.create_table()?)?;
                    globals.raw_set("require", functions.raw_get::<Function>("require")?)?;
                    globals.raw_set("libraries", functions.raw_get::<Function>("libraries")?)?;
                    lua.load(src)
                        .set_name(format!("@{name}.module.luau"))
                        .set_environment(globals)
                        .eval::<Table>()
                })();
                loading.borrow_mut().remove(&name);
                let exports = result?;
                cache.raw_set(name, exports.clone())?;
                Ok(exports)
            })?;
            // The libraries in a folder, sorted: content collections where a
            // new file is a new entry (`for _, name in libraries("content/cards")`).
            let libraries = scope.create_function(|lua, folder: String| {
                let prefix = format!("{}/", folder.trim_end_matches('/'));
                let names: BTreeSet<&String> = self.0.keys().chain(self.1.keys()).filter(|name| name.starts_with(&prefix)).collect();
                lua.create_sequence_from(names.into_iter().cloned())
            })?;
            functions.raw_set("require", require.clone())?;
            functions.raw_set("libraries", libraries.clone())?;
            entry.raw_set("require", require)?;
            entry.raw_set("libraries", libraries)?;
            lua.load(source)
                .set_name(format!("@{name}"))
                .set_environment(entry.clone())
                .eval()
        })
    }
}
