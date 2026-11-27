// A game in its own worker and canvas (worker_game.rs, app/src/web_offscreen.rs):
// it loads and draws its first frame while the page keeps the room running.
// First message: {module, canvas, folder, players, version} (canvas transferred).
// Later: {controls: [...]} per frame, {resize: [w, h]}.
let api = null;
const early = [];
self.onmessage = async ({data}) => {
  // The page's sound node: the game's sound goes straight there (audio device_worker.rs).
  if (data.audio) { self.warpAudioPort = data.audio; return; }
  if (data.canvas) {
    // Background jobs (app web_jobs.rs) start the same module in their own worker.
    self.warpWasmModule = data.module;
    self.warpVersion = data.version;
    try {
      const glue = new URL('./pkg/browser.js', import.meta.url);
      glue.searchParams.set('v', data.version);
      api = await import(glue.href);
      await api.default({module_or_path: data.module});
      for (const m of early.splice(0)) self.onmessage({data: m});
      await api.game_in_worker(data.canvas, data.folder, data.players, data.version);
    } catch (error) {
      self.postMessage({type: 'error', error: String(error?.message || error)});
    }
  } else if (!api) {
    early.push(data);
  } else if (data.controls) {
    api.worker_controls(JSON.stringify(data.controls));
  } else if (data.resize) {
    api.worker_resize(data.resize[0], data.resize[1]);
  }
};
