# Rigged people in the Growing Guns stadium

9 November 2026. Windows, NVIDIA GeForce RTX 5070 Ti, Vulkan, seed 52.

## Regular match

The regular game still draws its shipping crowd. Four headless runs of
`growing-guns bench --seed 52 --views 4 --size 3840x2160 --frames 120`
used full lighting and 120 GPU samples each. The order was old, new, new, old.
The old build recorded 50.0 and 49.0 serialized FPS, with median GPU times of
8.83 and 8.95 ms. The first rig build recorded 48.8 and 48.9 FPS, with
GPU times of 8.75 and 8.77 ms. The run waits for GPU completion each frame.
This comparison checks the existing match path. The FPS cost of replacing its
crowd, including match lighting and shadows, remains unmeasured.

## Warp Humans playtest

Three matched pairs of independent 121-frame camera rides compared 8cf8cbfd
and c39078b6. Each ride draws 8,353 spectators using distant impostors and two
960 by 540 views. One camera approaches a face and the second stays wide.
The table gives the median of three run medians. The close-up spans frames
54–66 inclusive. GPU timestamps cover view selection and the stadium draw.

| GPU time per frame | Earlier people | Rigged people |
| --- | ---: | ---: |
| Fixed wide view | 0.112 ms | 0.115 ms |
| Both views across the camera ride | 0.214 ms | 0.230 ms |
| Both views during the close-up | 0.251 ms | 0.572 ms |

At frame 60 the moving camera drew 41 detailed spectators. The fixed camera
kept all 8,353 spectators in the far representation. Once close detail arrived,
the shared geometry and work buffers occupied 3,220,600 bytes and
the appearance atlas occupied 8,388,608 bytes.

[Machine-readable results and request-to-visible timings](../evidence/warp-humans-rig.json).
[Interactive controls and replay commands](../warp-humans.md).
