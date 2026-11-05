#pragma once

#define sqrtf(f) __builtin_sqrtf(f)
#define fabsf(f) __builtin_fabsf(f)
// Warp: the newer meshoptimizer sources (opacity maps, quantization) and
// clusterlod.h use these too. The ones without a wasm instruction lower to
// calls that Rust's compiler_builtins provides on wasm32.
#define floorf(f) __builtin_floorf(f)
#define ceilf(f) __builtin_ceilf(f)
#define log2f(f) __builtin_log2f(f)
#define ldexpf(f, e) __builtin_ldexpf(f, e)
#define frexpf(f, e) __builtin_frexpf(f, e)
#define cosf(f) __builtin_cosf(f)
