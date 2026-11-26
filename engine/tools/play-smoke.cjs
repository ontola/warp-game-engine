#!/usr/bin/env node
// Browser smoke test of a /play/ bundle (build/web): Chromium with WebGPU
// (SwiftShader, no GPU needed) opens the Game Room, then plays every game on
// its shelf in turn (switch_game, as Play at the TV does) and goes back to
// the room after each. It fails on an engine panic, a GPU error, a page or
// console error, the crash or GPU-problem notice, a game whose first frame
// never comes, or a picture that is all one colour.
//
//   node engine/tools/play-smoke.cjs build/web [--games a,b] [--report out.json] [--shots DIR] [--shaders DIR]
//
// --shaders DIR writes every WGSL module the page and its game workers
// compiled (one .wgsl per distinct source) for engine/tools/hlsl-check.py,
// which runs them through naga's HLSL output and DXC as Firefox on Windows
// does. The 3D view renders 180 lines high (?render=180) on a 640x360 page:
// SwiftShader draws on the CPU.
//
// Needs Playwright (npm i playwright, its Chromium) and a display: run it
// under `xvfb-run -a` on a server. Headless Chromium loses the SwiftShader
// device after a few frames ("Device was destroyed"); a real (virtual)
// window does not. engine/tools/play-release.py runs this as a release
// check (engine/docs/play-release.md).
'use strict';
const http = require('http'), fs = require('fs'), path = require('path'), zlib = require('zlib');
let chromium;
try { ({ chromium } = require('playwright')); } catch { console.error('play-smoke: needs playwright (npm i playwright)'); process.exit(2); }

const args = process.argv.slice(2);
const opt = (name, dflt) => { const i = args.indexOf(name); return i >= 0 ? args.splice(i, 2)[1] : dflt; };
const report = opt('--report'), shots = opt('--shots'), only = opt('--games'), shaderDir = opt('--shaders');
const roomTimeout = +opt('--room-timeout', 240) * 1000, gameTimeout = +opt('--game-timeout', 300) * 1000;
const dir = path.resolve(args[0] || 'build/web');
const library = JSON.parse(fs.readFileSync(path.join(dir, 'games/library.json'), 'utf8')).filter(g => !g.web_entry);
const games = only ? only.split(',').map(f => library.find(g => g.folder === f) || { folder: f, title: f }) : library;

// Every WGSL module the engine compiles, page and workers alike, reaches
// the console once as `WARP_WGSL {label, code}`.
const CAPTURE = `(() => { const D = self.GPUDevice && GPUDevice.prototype; if (!D || D.__warpWgsl) return; D.__warpWgsl = true;
  const seen = new Set(), real = D.createShaderModule;
  D.createShaderModule = function (desc) { try { const c = desc && desc.code; if (c && !seen.has(c)) { seen.add(c); console.debug('WARP_WGSL ' + JSON.stringify({ label: desc.label || '', code: c })); } } catch (e) {} return real.apply(this, arguments); };
})();\n`;
const types = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.wasm': 'application/wasm', '.json': 'application/json', '.png': 'image/png', '.jpg': 'image/jpeg', '.ttf': 'font/ttf', '.wgsl': 'text/plain' };
const server = http.createServer((q, r) => {
  let p = decodeURIComponent(q.url.split('?')[0]);
  if (p.endsWith('/')) p += 'index.html';
  const file = path.join(dir, path.normalize(p));
  if (!file.startsWith(dir)) { r.writeHead(403); r.end(); return; }
  fs.readFile(file, (e, data) => {
    if (e) { r.writeHead(404); r.end(); return; }
    r.writeHead(200, { 'Content-Type': types[path.extname(file)] || 'application/octet-stream', 'Cache-Control': 'no-cache' });
    r.end(path.basename(file) === 'game-worker.js' ? Buffer.concat([Buffer.from(CAPTURE), data]) : data);
  });
});

// Engine log lines and console messages that mean the build is broken.
const FATAL = /panicked|First GPU error|GPU error:|frame error|start failed|GPU setup failed|WebGPU could not start|device lost|RuntimeError|unreachable executed|Could not start|failed to load|out of memory/i;
// A warmed game worker that is not used is closed; its GPU device then
// reports this loss. Harmless: the game loads on the page instead.
const BENIGN = /A valid external Instance reference no longer exists/;
const READY = /^(.*): first frame completed in (\d+) ms/;

// Distinct colours on a grid of a PNG screenshot (RGB/RGBA, 8 bit).
function colours(png) {
  const w = png.readUInt32BE(16), h = png.readUInt32BE(20), type = png[25];
  const bpp = type === 6 ? 4 : 3;
  const chunks = [];
  for (let o = 8; o < png.length;) {
    const len = png.readUInt32BE(o), kind = png.toString('ascii', o + 4, o + 8);
    if (kind === 'IDAT') chunks.push(png.subarray(o + 8, o + 8 + len));
    o += 12 + len;
  }
  const raw = zlib.inflateSync(Buffer.concat(chunks)), stride = w * bpp, img = Buffer.alloc(h * stride);
  for (let y = 0; y < h; y++) {
    const f = raw[y * (stride + 1)], src = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    for (let x = 0; x < stride; x++) {
      const a = x >= bpp ? img[y * stride + x - bpp] : 0, b = y ? img[(y - 1) * stride + x] : 0, c = x >= bpp && y ? img[(y - 1) * stride + x - bpp] : 0;
      const pa = Math.abs(b - c), pb = Math.abs(a - c), pc = Math.abs(a + b - 2 * c);
      const pred = [0, a, b, (a + b) >> 1, pa <= pb && pa <= pc ? a : pb <= pc ? b : c][f];
      img[y * stride + x] = (src[x] + pred) & 255;
    }
  }
  const seen = new Set();
  for (let y = 0; y < h; y += Math.max(1, h >> 5)) for (let x = 0; x < w; x += Math.max(1, w >> 5)) {
    const i = y * stride + x * bpp;
    seen.add((img[i] >> 3) << 10 | (img[i + 1] >> 3) << 5 | img[i + 2] >> 3);
  }
  return seen.size;
}

(async () => {
  await new Promise(ok => server.listen(0, '127.0.0.1', ok));
  const url = `http://127.0.0.1:${server.address().port}/?cloud=off&players=2&render=180`;
  const browser = await chromium.launch({
    headless: false,
    args: ['--enable-unsafe-webgpu', '--enable-features=Vulkan', '--use-vulkan=swiftshader', '--use-webgpu-adapter=swiftshader',
      '--disable-vulkan-surface', '--ignore-gpu-blocklist', '--autoplay-policy=no-user-gesture-required'],
  });
  const page = await browser.newPage({ viewport: { width: 640, height: 360 } });
  await page.addInitScript(CAPTURE);
  const shaders = new Map();
  const t0 = Date.now(), log = [], problems = [];
  const stamp = () => ((Date.now() - t0) / 1000).toFixed(1);
  // Games warmed in a Web Worker (game-worker.js) draw and log there.
  const onConsole = m => {
    const text = m.text();
    if (text.startsWith('WARP_WGSL ')) {
      try { const s = JSON.parse(text.slice(10)); if (!shaders.has(s.code)) shaders.set(s.code, s.label); } catch {}
      return;
    }
    if (/^loading: /.test(text)) return;
    log.push(text);
    console.log(`${stamp()} ${m.type() === 'log' ? '' : m.type() + ' '}${text.slice(0, 400)}`);
    if ((m.type() === 'error' || FATAL.test(text)) && !BENIGN.test(text)) problems.push(text.slice(0, 600));
  };
  page.on('console', onConsole);
  page.on('worker', w => {
    w.on('console', onConsole);
    w.on('error', e => problems.push('worker error: ' + (e.message || e)));
  });
  page.on('pageerror', e => { problems.push('page error: ' + e.message); console.log(`${stamp()} PAGE ERROR ${e.message}`); });
  page.on('crash', () => problems.push('the page crashed'));
  page.on('requestfailed', r => problems.push(`request failed: ${r.url()} (${r.failure()?.errorText})`));
  page.on('response', r => { if (r.status() >= 400) problems.push(`HTTP ${r.status()}: ${r.url()}`); });

  const results = [];
  // Wait for a new "first frame completed" line from `from` on, or a problem.
  const ready = async (from, timeout, accept) => {
    const end = Date.now() + timeout;
    while (Date.now() < end) {
      if (problems.length) return { error: problems.join('\n') };
      const notice = await page.evaluate(() => [document.getElementById('warp-crash'), document.getElementById('warp-gpu-problem')]
        .filter(e => e && !e.hidden).map(e => e.textContent.trim().slice(0, 300)).join(' | ')).catch(e => 'page gone: ' + e.message);
      if (notice) return { error: notice };
      for (let i = from; i < log.length; i++) {
        const m = READY.exec(log[i]);
        if (m && accept(m[1])) return { title: m[1], ms: +m[2] };
      }
      await page.waitForTimeout(500);
    }
    return { error: `no first frame within ${timeout / 1000} s` };
  };
  const look = async (name) => {
    const png = await page.screenshot({ timeout: 180000 });
    if (shots) { fs.mkdirSync(shots, { recursive: true }); fs.writeFileSync(path.join(shots, name + '.png'), png); }
    return colours(png);
  };
  const settle = async (ms) => { const end = Date.now() + ms; while (Date.now() < end && !problems.length) await page.waitForTimeout(250); };
  const record = (name, r, started) => {
    const ok = !r.error;
    results.push({ name, ok, detail: ok ? r.detail : r.error, seconds: +((Date.now() - started) / 1000).toFixed(1) });
    console.log(`${ok ? 'ok  ' : 'FAIL'} browser:${name}: ${ok ? r.detail : r.error}`);
    return ok;
  };

  let started = Date.now();
  await page.goto(url);
  let room = await ready(0, roomTimeout, () => true);
  const roomTitle = room.title;
  if (!room.error) {
    await settle(3000);
    const n = await look('room').catch(() => -1);
    if (problems.length) room = { error: problems.join('\n') };
    else if (n < 0) room = { error: 'no picture within 180 s (screenshot timed out)' };
    else if (n < 8) room = { error: `the room's picture is nearly one colour (${n} colours)` };
    else room.detail = `first frame after ${room.ms} ms, ${n} colours`;
  }
  if (record('room', room, started)) {
    for (const g of games) {
      started = Date.now();
      const from = log.length;
      await page.evaluate(name => window.warp.switch_game(name), g.folder).catch(e => problems.push('switch_game: ' + e.message));
      let r = await ready(from, gameTimeout, t => t !== roomTitle);
      if (!r.error) {
        await settle(5000);
        const n = await look(g.folder).catch(() => -1);
        if (problems.length) r = { error: problems.join('\n') };
        else if (n < 0) r = { error: 'no picture within 180 s (screenshot timed out)' };
        else if (n < 8) r = { error: `the picture is nearly one colour (${n} colours)` };
        else r.detail = `${r.title}: first frame after ${r.ms} ms, ${n} colours`;
      }
      const ok = record(g.folder, r, started);
      if (!ok) break;  // A broken game can leave the engine unusable for the next.
      await page.evaluate(() => window.warp.back_to_room()).catch(e => problems.push('back_to_room: ' + e.message));
      await settle(3000);
      if (problems.length) { record(`back-from-${g.folder}`, { error: problems.join('\n') }, Date.now()); break; }
    }
  }
  for (const g of games) if (!results.some(r => r.name === g.folder)) results.push({ name: g.folder, ok: false, detail: 'not reached' });
  if (shaderDir) {
    fs.mkdirSync(shaderDir, { recursive: true });
    let i = 0;
    for (const [code, label] of shaders) {
      const name = String(i++).padStart(3, '0') + '-' + (label || 'unnamed').replace(/[^A-Za-z0-9_.-]+/g, '_').slice(0, 60);
      fs.writeFileSync(path.join(shaderDir, name + '.wgsl'), code);
    }
    console.log(`${shaders.size} WGSL modules in ${shaderDir}`);
  }
  if (report) fs.writeFileSync(report, JSON.stringify({ url, results, log: log.slice(-400) }, null, 1));
  await browser.close();
  server.close();
  process.exit(results.every(r => r.ok) ? 0 : 1);
})().catch(e => { console.error(e); process.exit(2); });
