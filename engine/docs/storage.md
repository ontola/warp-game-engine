# Savegames, profiles and settings

Enable storage with `storage = true` and a stable `id = "my-game"` in
`game.toml`. For a one-file game use the equivalent header lines. Package
`storage.json` beside the game. Its keys may be `save`, `profile` and
`settings`. Each array contains independent JSON Schemas (Draft 2020-12) in
version order, starting at one. Only declare the kinds your game uses.
Keep released schemas in place and append new versions. Warp compiles them
at startup. Use local `$defs` for references within a schema.

```json
{
  "save": [
    {"type": "integer", "minimum": 0},
    {
      "type": "object",
      "required": ["score"],
      "properties": {"score": {"type": "integer", "minimum": 0}},
      "additionalProperties": false
    }
  ],
  "settings": [{"type": "object", "properties": {"volume": {"type": "number", "minimum": 0, "maximum": 1}}}],
  "profile": [{"type": "object", "required": ["name"], "properties": {"name": {"type": "string"}}}]
}
```

## Read, migrate and save

`storage.read(kind, slot)` returns nil for an unused slot or a table with
`version`, `revision` and `data`. Warp validates the document against its
stored schema version. The game supplies the migration logic. For the save
schemas above a v1 score is a number and v2 stores it in an object:

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
Atomic-lib provides Atomic Schema, resources and commit history. Those are
candidates for a future sync layer with identities and conflict resolution.
This implementation stores one current revision locally per slot.
