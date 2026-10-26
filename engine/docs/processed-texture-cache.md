# Processed texture cache

`Renderer::new` reuses processed RGBA mip chains from a persistent derived-data
cache. It does not change the image-processing algorithm or source assets.
All GPU textures are still created and uploaded for each renderer; the cache
removes repeated CPU resizing, alpha-coverage processing and linear RGB filtering.

## Why this cache

The measured `scenario-noon-001/startup.json` in
`C:/dev/perf/warp-sideways-20261024/r26-regression` recorded 2344.88 ms of renderer
CPU construction, including 1815.56 ms for albedo (1559.68 ms parallel chain-build
wall time, 201.16 ms upload calls), and 142.82 ms for normal textures. This is one
configuration, not a speedup claim or a comparison with the earlier 14.76-second
startup. Image decoding happens in scene construction before this constructor.

## Controls and storage

- Enabled by default; `KERN_TEXTURE_CACHE=0` bypasses hashing, reads and writes.
- `KERN_TEXTURE_CACHE_DIR` selects a directory containing only generated `.mips`
  entries and temporary cache writes.
- Windows default: `%LOCALAPPDATA%/Warp/Cache/texture-mips-v1`.
- Other platforms: `$XDG_CACHE_HOME/warp/texture-mips-v1`, falling back to
  `$HOME/.cache/warp/texture-mips-v1`.
- Entries are reconstructible and may be removed while the game is closed.
  There is currently no automatic disk-size eviction policy.

A SHA-256 key includes decoded source pixels and dimensions (or explicit missing
source), output size, UNORM versus sRGB, fallback colour, cutout alpha policy,
linear-cutout-RGB mode, and an algorithm fingerprint. The fingerprint includes
the processing revision, processing source files, locked image-library version
and checksum, OS and architecture. Changes to lighting, camera or stage seed do
not invalidate identical processed images.

Each file has a fixed header with the expected key, exact payload length and a
second SHA-256 covering all mip bytes. Reads validate file length before allocating
the known bounded mip sizes, then verify every byte. Missing, truncated, corrupted
or incompatible entries rebuild through the normal path. Unwritable caches do
not prevent rendering. Unique, exclusively created temporary files are flushed
and closed before an atomic same-directory rename; readers never open temporary
files. Interrupted writes remain irrelevant temporary files. Concurrent writers
for one key produce equivalent content.

## Measurement

Enable `KERN_STARTUP_PROFILE=1` for one structured `renderer_startup` JSON line.
Each texture layer reports cache hits, misses, corruption/read/write errors,
bytes read/written, and hash/read/write CPU times. Read/write times exclude the
separately reported hashing. Layer times run concurrently: use the array's
`build_wall_ms` for elapsed build time instead of summing worker times. Upload
times measure CPU API calls, not GPU completion.

For a controlled comparison, use the same executable, scene and graphics options:
run cache-off, then cache-on with a fresh explicit directory, then cache-on again
with that directory. Report cold-fill and warm-hit costs separately, including
unchanged GPU uploads and unrelated startup work. No cold/warm runtime result is
claimed yet in this document.

## Validation

Five focused CPU tests passed on Windows: exact processed-mip round trips across
16 combinations of source/fallback, UNORM/sRGB, cutout and RGB-correction options;
key invalidation for each input and algorithm revision; corrupt/truncated/wrong-key
and partial-file fallback; eight concurrent writers; disabled/unwritable caches.
The two startup-attribution tests and five existing alpha/radiance tests also pass.
No GPU acceptance or gameplay performance result follows from those CPU tests.
