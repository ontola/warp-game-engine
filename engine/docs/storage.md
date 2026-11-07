# Savegames, profiles and settings

Enable storage with `storage = true` and a stable `id = "my-game"` in
`game.toml`. For a one-file game use the equivalent header lines. Use **one authored save schema**, `save-schema.json`, beside the game. GameNight's publisher reads the same file beside `gamenight.json` and uses atomic-lib to derive Atomic properties. Its generated `save-schema.lock.json` records their identities. Warp validates the save payload with JSON Schema Draft 2020-12. The SQLite storage backend does not affect the schema format.

For example, the current `save-schema.json` for a score can be:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$defs": {
    "Progress": {
      "type": "object",
      "properties": {"score": {"type": "integer", "minimum": 0}},
      "required": ["score"],
      "additionalProperties": false
    }
  }
}
```

GameNight publishes the properties of its single object definition. Warp validates the payload against that definition. Keep the published
document in GameNight's form: the atomic-lib 0.41.0-beta.8 importer accepts
`$ref` on properties but does not accept it at the document root. A generic
JSON Schema validator needs to select the definition explicitly; validating
only the container would miss its rules. GameNight handles its
`x-gamenight-lenses` and `x-gamenight-dropped` publishing annotations. Local
save migrations are game code. [GameNight's schema contract](https://github.com/ontola/gamenight/blob/170936a/docs/player-store.md)

If your game stores separate settings or profile documents, describe those
payloads in `settings-schema.json` or `profile-schema.json` using the same JSON
Schema format. Only the save schema is published by GameNight's save-schema command.

For a brand-new game the current schema is version 1. Before changing the shape of a released save, archive its schema under `schema-history/save/1.json` and edit `save-schema.json` for version 2. Keep historical files numbered consecutively. The same convention applies to the other two kinds. Warp compiles current and historical schemas at startup.

Games released with Warp's original `storage.json` can still load. To convert,
move each kind's last schema to its current file and its earlier versions to
the numbered history directory, then remove `storage.json`. Keep the game ID
and version order to retain existing slots and revisions.

## Read, migrate and save

`storage.read(kind, slot)` returns nil for an unused slot or a table with
`version`, `revision` and `data`. Warp validates the document against its
stored schema version. The game supplies the migration logic. For example, if version 1 stored a number and version 2 uses the object above:

```lua
local function load_score()
    local record = storage.read("save", "campaign")
    if record == nil then return {score = 0}, 0 end
    if record.version == 1 then
        local data = {score = record.data}
        local revision = storage.write("save", "campaign", record.revision, data)
        return data, revision
    end
    return record.data, record.revision
end
```

`storage.write(kind, slot, expectedRevision, data)` validates against the
latest declared schema and returns the new revision after commit. Use zero
for a new slot. Keep the returned revision for the next save. A stale writer
gets an error. Read again and decide which changes to keep. An older game
cannot overwrite a slot that uses a newer schema. Validation errors and
aborted transactions leave the previous committed document intact.

Use `pcall` to report save errors to the player. Read during initialization
and write at checkpoints or when settings are confirmed. Disk I/O is
synchronous. The API rejects calls from `draw`.

## Data and scope

Use strings, booleans, finite numbers, tables with string keys, or dense
1-based arrays. `{}` represents an empty object, `storage.array()` creates an
empty array, and `storage.null` represents JSON null. A slot name uses 1 to
128 UTF-8 bytes. A document can contain up to 65,536 values, nested at most
64 levels below the root, and its encoded JSON must fit in 1 MiB.

A record belongs to one game ID, kind and slot. Games using the same ID
share that namespace. A game chooses a stable profile key for each local
player. A controller seat number alone is session-specific.

`warp run` uses the OS user's local Warp data directory. `warp check`,
`warp serve` and `warp script` start with an empty in-memory database each
launch. Pass `--storage-dir DIR` to any of these commands to reuse a chosen
directory, for example to test migration across separate processes. Keep
that directory separate from real saves for repeatable playtests.
Rust hosts select the equivalent `storage::Location` through
`LuaGame::load_with_storage`.

## Backend choice

SQLite provides local durable transactions and revision checks across game
processes. JSON Schema describes the payload independently of the backend.
GameNight already uses atomic-lib to import JSON Schema and synchronize player data. Warp's local storage does not itself synchronize these slots.
