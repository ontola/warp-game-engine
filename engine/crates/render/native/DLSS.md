# NVIDIA NGX / DLSS Super Resolution bridge

This bridge calls the official SDK. It does not emulate DLSS, generate frames,
replace the DLL, use another application's NVIDIA ID, or bypass feature checks.
The stable project GUID `7d8e9c51-2461-4a6b-bc30-cdba9d2e91fc` identifies this
custom Kern/Sideways integration through `NVSDK_NGX_ENGINE_TYPE_CUSTOM`.

Build on Windows MSVC with the `render/dlss` Cargo feature and these external
SDK checkouts (SDK binaries and headers are not re-licensed or vendored here):

- `NVIDIA_DLSS_SDK`: https://github.com/NVIDIA/DLSS at
  `9981c6350dc58c0ce21f024fd65e31e851c31689` (DLSS 310.9.16).
- `VULKAN_HEADERS`: https://github.com/KhronosGroup/Vulkan-Headers at
  `73814d006638aa00bd553215b054f0f02e457e22`.

Put the official `lib/Windows_x86_64/rel/nvngx_dlss.dll` beside the executable,
or set `KERN_DLSS_RUNTIME_DIR` to its directory. Preserve NVIDIA's `LICENSE.txt`
with distributed runtime components and meet its distribution requirements;
the engine's MIT license does not cover the SDK. The development DLL is not a
shipping runtime. This software contains source code provided by NVIDIA
Corporation (the linked inline SDK helper routines).

Native interop uses wgpu's tracked `transition_resources` in ordinary command
buffers before and after a **separate raw-only** `as_hal_mut` command buffer.
wgpu 30 forbids mixing these encoding APIs in one encoder. All segments are
submitted together in order; the split adds no CPU wait or extra queue submission.
All inputs are read-only sampled images;
the output is a storage image. A submission-completion callback retains native
feature histories, texture views and textures until the GPU is finished. The
context retains a wgpu Device handle; dropping the outer context waits for
already-submitted work before releasing its cached histories. No per-frame GPU
wait or readback is used. One feature history is retained per view and geometry
size/quality. Resizing starts a new history and preserves in-flight old ones.
Contexts on the same Vulkan device share one NGX initialization and shutdown;
the registry also serializes replacement during final shutdown. Device creation
queries the SDK's required extensions and adds only supported requirements through
wgpu's HAL callback. Missing instance extensions are an explicit error, rather
than attempting to enable them after creation. Select `WGPU_BACKEND=vulkan`.
The output receives one tracked clear at allocation so wgpu's lazy initialization
cannot erase the subsequent native write. Evaluation is once per view per frame.

Inputs: un-pre-exposed HDR RGBA16F, reversed hardware depth in R32F, low-resolution
RG16F motion (previous-minus-current pixels, Y down, excludes jitter), optional
R32F bias-current-color mask, render-pixel jitter, and explicit exposure.
Evaluation replaces the engine's TAA, before tone mapping and native-resolution
HUD composition. Unsupported SDK/backend/device/quality states return errors;
there is no silent fallback labelled as DLSS.
