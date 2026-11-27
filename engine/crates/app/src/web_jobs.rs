//! Background jobs in a browser: CPU work that natively runs on its own
//! thread (a lighting bake) runs in one shared Web Worker instead of
//! stalling the page's only thread. The worker (`job-worker.js` beside the
//! page, lobbies/game-room/examples/browser) instantiates the page's own
//! wasm module and calls its `worker_job(name, input)` export, which picks
//! the game's job by name. Input and output are bytes: the game encodes
//! what the job needs.

use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
let worker = null, next = 1, broken = '';
const waiting = new Map();
export function run_job(name, input, done) {
  const g = globalThis;
  const module = g.warpWasmModule;
  if (broken || !module || typeof Worker === 'undefined') { done(broken || 'no background worker here', null); return; }
  if (!worker) {
    const base = g.document ? g.document.baseURI : g.location.href;
    try { worker = new Worker(new URL('job-worker.js', base), {type: 'module'}); }
    catch (e) { broken = String(e?.message || e); done(broken, null); return; }
    worker.onmessage = ({data}) => { const d = waiting.get(data.id); waiting.delete(data.id); d?.(data.error || null, data.output || null); };
    worker.onerror = e => { broken = e.message || 'job worker failed'; for (const d of waiting.values()) d(broken, null); waiting.clear(); };
  }
  const id = next++;
  waiting.set(id, done);
  const version = g.document ? (g.document.documentElement.dataset.version || '') : (g.warpVersion || '');
  worker.postMessage({id, name, input, module, version}, [input.buffer]);
}
"#)]
extern "C" {
    fn run_job(name: &str, input: js_sys::Uint8Array, done: &JsValue);
}

/// Run job `name` with `input` in the background worker; `done` gets its
/// output on the page's thread later, or an error (no worker here, the job
/// failed): then do the work in place.
pub fn job(name: &str, input: Vec<u8>, done: impl FnOnce(Result<Vec<u8>, String>) + 'static) {
    let callback = Closure::once_into_js(move |error: JsValue, output: JsValue| {
        match error.as_string() {
            Some(e) => done(Err(e)),
            None => done(output.dyn_into::<js_sys::Uint8Array>().map(|a| a.to_vec()).map_err(|_| "job: no output".to_string())),
        }
    });
    run_job(name, js_sys::Uint8Array::from(&input[..]), &callback);
}
