/* The runtime clang's WebAssembly setjmp/longjmp lowering calls (as in
 * wasi-libc's setjmp/wasm32/rt.c). Both ABIs: LLVM 19 and newer call
 * __wasm_setjmp and __wasm_setjmp_test; LLVM 18 and older call
 * saveSetjmp, testSetjmp and getTempRet0. longjmp throws the C_LONGJMP
 * tag (1) with {env, val}; the lowered setjmp's catch compares env. */
#include <stdint.h>
#include <stdlib.h>

struct arg { void *env; int val; };

/* Throws the C_LONGJMP tag; in longjmp.s, since clang 18 cannot throw
 * an undefined tag from C. */
void warp_throw_longjmp(struct arg *arg);
struct jmp_buf_impl { void *func_invocation_id; uint32_t label; struct arg arg; };

void __wasm_setjmp(void *env, uint32_t label, void *func_invocation_id) {
    struct jmp_buf_impl *jb = env;
    if (label == 0 || func_invocation_id == 0) abort();
    jb->func_invocation_id = func_invocation_id;
    jb->label = label;
}

uint32_t __wasm_setjmp_test(void *env, void *func_invocation_id) {
    struct jmp_buf_impl *jb = env;
    return jb->func_invocation_id == func_invocation_id ? jb->label : 0;
}

void __wasm_longjmp(void *env, int val) {
    struct jmp_buf_impl *jb = env;
    jb->arg.env = env;
    jb->arg.val = val == 0 ? 1 : val;
    warp_throw_longjmp(&jb->arg);
    abort();
}

/* LLVM 18: a table of (setjmp id, label) pairs per function. */
struct entry { uint32_t id, label; };
static uint32_t next_id = 1, temp_ret0;

void *saveSetjmp(void *env, uint32_t label, void *table, uint32_t size) {
    struct entry *t = table;
    uint32_t id = next_id++;
    *(uint32_t *)env = id;
    uint32_t i = 0;
    for (; i < size; i++) {
        if (t[i].id == 0) { t[i].id = id; t[i].label = label; t[i + 1].id = 0; temp_ret0 = size; return t; }
    }
    t = realloc(t, sizeof(struct entry) * (2 * size + 1));
    t[i].id = id; t[i].label = label; t[i + 1].id = 0;
    temp_ret0 = 2 * size;
    return t;
}

uint32_t testSetjmp(uint32_t id, void *table, uint32_t size) {
    struct entry *t = table;
    for (uint32_t i = 0; i < size && t[i].id; i++) if (t[i].id == id) return t[i].label;
    return 0;
}

uint32_t getTempRet0(void) { return temp_ret0; }
