# wgpu 30.0.1, patched for the browser

A copy of the [wgpu](https://crates.io/crates/wgpu) crate 30.0.1 (MIT or
Apache-2.0), used through `[patch.crates-io]` in the workspace Cargo.toml.
Only `src`, `build.rs`, `Cargo.toml` (without dev-dependencies), the licences
and README are kept.

Change, which only matters on wasm32 (the WebGPU backend):

- `src/backend/webgpu.rs` `Error::from_js`: an error that is neither a
  GPUValidationError nor a GPUOutOfMemoryError (Firefox on Windows sends a
  GPUInternalError) becomes `Error::Internal` with its message. wgpu panicked
  with "Unexpected error" there, which ended the page ("Module terminated")
  and hid the browser's message.

- `wgpu::webgpu::created()` (Warp addition): how many shader modules and
  render/compute pipelines the page created and the time spent in those
  calls, for the browser host's startup log ("Preparing graphics").
- `wgpu::webgpu::{set_prewarm, prewarm_take}` (Warp addition): build a
  renderer once with every pipeline compiled through createXPipelineAsync
  (in parallel in the browser) and invalid stand-ins handed back, await
  them, then build it for real: the same pipelines come from the browser's
  cache. The browser host's "Preparing graphics" (app::web).

- A buffer mapped at creation also gets COPY_DST (unless it is MAP_READ or
  MAP_WRITE) and keeps the device's queue. When the browser hands back an
  empty ArrayBuffer for its mapping (WebKit 26.6 in Playwright on macOS 26,
  "spatial instance group bounds"), `get_mapped_range` writes into an
  ArrayBuffer of its own and `unmap` copies it in with `writeBuffer`.

- `WebBuffer::get_mapped_range` panics with the asked range, the mapped
  range and the ArrayBuffer's length instead of WebKit's bare RangeError
  ("Length out of range of buffer", which stopped the room in WebKit).

Drop this copy when a wgpu release handles GPUInternalError.
