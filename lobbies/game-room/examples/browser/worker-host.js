// The page's side of a game warmed in a Web Worker (worker_game.rs,
// game-worker.js): the worker has its own canvas over the room's and its
// own GPU device. It loads, builds and draws its first frame while the room
// plays, then waits. The room's shell keeps reading pads and keys and hands
// each player's controls here ([gameControls]); the worker plays while they
// come and waits again once they stop (Back: the room is back).
const games = new Map();
let next = 1;

export function spawnGame(folder, players) {
  const id = next++;
  const canvas = document.createElement('canvas');
  canvas.className = 'warp-worker';
  canvas.style.cssText = 'position:fixed;inset:0;width:100vw;height:100vh;z-index:5;visibility:hidden;background:#000;pointer-events:none';
  document.body.appendChild(canvas);
  const scale = () => devicePixelRatio || 1;
  const size = () => [Math.max(1, Math.round(innerWidth * scale())), Math.max(1, Math.round(innerHeight * scale()))];
  [canvas.width, canvas.height] = size();
  const offscreen = canvas.transferControlToOffscreen();
  const worker = new Worker(new URL('game-worker.js', document.baseURI), {type: 'module'});
  const t0 = performance.now();
  const game = {worker, canvas, phase: 'loading', exit: '', error: '', sent: 0, shown: false};
  const resize = () => worker.postMessage({resize: size()});
  game.resize = resize;
  addEventListener('resize', resize);
  worker.onmessage = ({data}) => {
    console.log(`worker game ${folder}:`, JSON.stringify(data), Math.round(performance.now() - t0) + ' ms since spawn');
    if (game.phase === 'error') { if (data.type === 'exit') game.exit = data.why; return; }
    if (data.type === 'loaded') game.phase = 'loaded';
    else if (data.type === 'first-frame') game.phase = 'ready';
    else if (data.type === 'exit') game.exit = data.why;
    else if (data.type === 'error') { game.phase = 'error'; game.error = data.error; }
  };
  worker.onerror = e => { game.phase = 'error'; game.error = e.message || 'worker failed'; };
  worker.postMessage({module: window.warpWasmModule, canvas: offscreen, folder, players, version: document.documentElement.dataset.version || ''}, [offscreen]);
  // Shown while the room hands it controls; hidden once they stop.
  game.watch = setInterval(() => {
    const on = performance.now() - game.sent < 150;
    if (on !== game.shown) { game.shown = on; canvas.style.visibility = on ? 'visible' : 'hidden'; }
  }, 50);
  games.set(id, game);
  return id;
}

/// "loading", "loaded", "ready" (its first frame is drawn) or "error: …".
export function gameState(id) {
  const g = games.get(id);
  if (!g) return 'error: gone';
  return g.phase === 'error' ? 'error: ' + g.error : g.phase;
}

/// "back", "quit" or "" (once each).
export function takeExit(id) {
  const g = games.get(id);
  if (!g) return '';
  const e = g.exit; g.exit = '';
  return e;
}

export function gameControls(id, json) {
  const g = games.get(id);
  if (!g) return;
  g.sent = performance.now();
  if (!g.shown) { g.shown = true; g.canvas.style.visibility = 'visible'; }
  g.worker.postMessage({controls: JSON.parse(json)});
}

export function disposeGame(id) {
  const g = games.get(id);
  if (!g) return;
  games.delete(id);
  clearInterval(g.watch);
  removeEventListener('resize', g.resize);
  g.worker.terminate();
  g.canvas.remove();
}
