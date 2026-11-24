/* setjmp for Luau in the browser build (engine/tools/build-web.py).
 * wasi-libc before 2024 ships no setjmp; clang lowers setjmp/longjmp to
 * WebAssembly exception handling (-mllvm -wasm-enable-sjlj) and calls the
 * runtime in sjlj.c. Luau is built with LUA_USE_LONGJMP=1, so a Lua error
 * unwinds to its lua_pcall instead of needing C++ exceptions. */
#ifndef WARP_SETJMP_H
#define WARP_SETJMP_H
#ifdef __cplusplus
extern "C" {
#endif
typedef struct { void *slots[4]; } jmp_buf[1];
int setjmp(jmp_buf env) __attribute__((returns_twice));
_Noreturn void longjmp(jmp_buf env, int val);
#define _setjmp setjmp
#define _longjmp longjmp
#ifdef __cplusplus
}
#endif
#endif
