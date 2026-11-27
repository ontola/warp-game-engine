#!/usr/bin/env node
// Browser test of "Play on this screen" and share links (a /play/ bundle,
// build/web), against a stand-in cloud on 127.0.0.1: room registration
// (/v1/lobbies/register) and the signaling WebSocket (/v1/signal/ROOM, which
// refuses a follower while the room has no host, as the real cloud does).
//
//   1. host: a page opens the Game Room. It registers one room, puts that
//      code in the address bar (?r=CODE) and opens it to other couches while
//      it still sits in the lobby (the follower would find nobody otherwise
//      and bounce back to the app, Joep, 27 November 2026).
//   2. follow: a second browser opens /play/?follow=CODE&r=OTHER (as a stale
//      share link did). It must follow that room: no room of its own, one
//      code in the address bar, connected to the host's signaling room, and
//      still on the page (not sent back to /app/).
//
//   node engine/tools/follow-smoke.cjs build/web [--browser chromium|firefox] [--report out.json] [--room-timeout 240]
//
// Needs Playwright and a display (xvfb-run -a), as play-smoke.cjs does;
// engine/tools/play-release.py runs it with the browser smoke test.
'use strict';
const http = require('http'), fs = require('fs'), path = require('path'), crypto = require('crypto');
let playwright;
try { playwright = require('playwright'); } catch { console.error('follow-smoke: needs playwright (npm i playwright)'); process.exit(2); }

const args = process.argv.slice(2);
const opt = (name, dflt) => { const i = args.indexOf(name); return i >= 0 ? args.splice(i, 2)[1] : dflt; };
const browserName = opt('--browser', 'chromium');
const report = opt('--report');
const roomTimeout = +opt('--room-timeout', 240) * 1000;
const dir = path.resolve(args[0] || 'build/web');
const CODE = 'HOSTQ7', STALE = 'TRKFF3';

const types = { '.html': 'text/html', '.css': 'text/css', '.svg': 'image/svg+xml', '.webp': 'image/webp', '.ogg': 'audio/ogg', '.mp3': 'audio/mpeg', '.woff2': 'font/woff2', '.js': 'text/javascript', '.mjs': 'text/javascript', '.wasm': 'application/wasm', '.json': 'application/json', '.png': 'image/png', '.jpg': 'image/jpeg', '.ttf': 'font/ttf', '.wgsl': 'text/plain' };
const registered = [];             // devices that registered a room
const sockets = [];                // { room, role, at }
const hosts = new Set();           // signaling rooms with a host
const server = http.createServer((q, r) => {
  const p0 = decodeURIComponent(q.url.split('?')[0]);
  if (p0 === '/v1/lobbies/register' && q.method === 'POST') {
    let body = '';
    q.on('data', c => body += c);
    q.on('end', () => {
      registered.push(body);
      r.writeHead(200, { 'Content-Type': 'application/json' });
      r.end(JSON.stringify({ token: 'smoke-token-' + registered.length, room_code: registered.length === 1 ? CODE : 'EXTRA' + registered.length }));
    });
    return;
  }
  if (p0 === '/v1/ice') { r.writeHead(200, { 'Content-Type': 'application/json' }); r.end('[]'); return; }
  // The room poll: its code, nothing else new.
  if (p0 === '/v1/lobbies/poll') { r.writeHead(200, { 'Content-Type': 'application/json' }); r.end(JSON.stringify({ room_code: CODE })); return; }
  if (p0.startsWith('/v1/')) { r.writeHead(204); r.end(); return; }
  let p = p0;
  if (p.endsWith('/')) p += 'index.html';
  const file = path.join(dir, path.normalize(p));
  if (!file.startsWith(dir)) { r.writeHead(403); r.end(); return; }
  fs.readFile(file, (e, data) => {
    if (e) { r.writeHead(404); r.end(); return; }
    r.writeHead(200, { 'Content-Type': types[path.extname(file)] || 'application/octet-stream', 'Cache-Control': 'no-cache' });
    r.end(data);
  });
});
// A WebSocket that stays open: the handshake and one text frame ({"you": n}).
server.on('upgrade', (q, socket) => {
  const u = new URL(q.url, 'http://x');
  const m = /^\/v1\/signal\/([^/]+)$/.exec(u.pathname);
  const role = u.searchParams.get('role');
  if (!m || (role === 'follow' && !hosts.has(m[1])) || (role === 'host' && hosts.has(m[1]))) {
    socket.end('HTTP/1.1 404 Not Found\r\n\r\n');
    return;
  }
  const accept = crypto.createHash('sha1').update(q.headers['sec-websocket-key'] + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
  socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
  const you = Buffer.from(JSON.stringify({ you: role === 'host' ? 0 : 1 }));
  socket.write(Buffer.concat([Buffer.from([0x81, you.length]), you]));
  if (role === 'host') hosts.add(m[1]);
  sockets.push({ room: m[1], role, at: Date.now() });
  socket.on('data', () => {});
  socket.on('error', () => {});
  socket.on('close', () => { if (role === 'host') hosts.delete(m[1]); });
});

(async () => {
  await new Promise(ok => server.listen(0, '127.0.0.1', ok));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const browser = browserName === 'firefox'
    ? await playwright.firefox.launch({ headless: false, firefoxUserPrefs: { 'dom.webgpu.enabled': true, 'gfx.webgpu.ignore-blocklist': true } })
    : await playwright.chromium.launch({
      headless: false,
      args: ['--enable-unsafe-webgpu', '--enable-features=Vulkan', '--use-vulkan=swiftshader', '--use-webgpu-adapter=swiftshader',
        '--disable-vulkan-surface', '--ignore-gpu-blocklist', '--disable-gpu-watchdog', '--disable-gpu-process-crash-limit'],
    });
  console.log(`${browserName} ${browser.version()} at ${origin}`);
  const results = [], log = [];
  const t0 = Date.now(), stamp = () => ((Date.now() - t0) / 1000).toFixed(1);
  const record = (name, error, detail) => { results.push({ name, ok: !error, detail: error || detail }); console.log(`${stamp()} ${error ? 'FAIL' : 'ok'} ${name}: ${error || detail}`); };
  const until = async (page, test, ms) => { const end = Date.now() + ms; while (Date.now() < end) { if (await test()) return true; await page.waitForTimeout(500); } return false; };
  const open = async (who, url) => {
    // Each its own browser profile: another device, with its own storage.
    const context = await browser.newContext({ viewport: { width: 640, height: 360 } });
    const page = await context.newPage();
    const problems = [], visited = [];
    page.on('console', m => { const t = `${who}: ${m.text()}`; log.push(t); if (/panicked|RuntimeError/.test(t)) problems.push(t); });
    page.on('pageerror', e => problems.push(`${who} page error: ${e.message}`));
    page.on('framenavigated', f => { if (f === page.mainFrame()) visited.push(f.url()); });
    await page.goto(url);
    return { page, problems, visited };
  };
  const roomUp = who => log.some(l => l.startsWith(who + ':') && /room|first frame/i.test(l));

  // 1. The host, in the lobby.
  const host = await open('host', `${origin}/?players=1&render=180`);
  const hosting = await until(host.page, () => sockets.some(s => s.role === 'host' && s.room === 'club-' + CODE) || host.problems.length, roomTimeout);
  const hostUrl = new URL(host.page.url());
  if (host.problems.length) record('host', host.problems.join('\n'));
  else if (registered.length !== 1) record('host', `registered ${registered.length} rooms, expected 1`);
  else if (hostUrl.searchParams.get('r') !== CODE) record('host', `address bar ${host.page.url()} does not carry ?r=${CODE}`);
  else if (!hosting) record('host', `never opened room ${CODE} to other couches while in the lobby (no host signaling socket within ${roomTimeout / 1000} s)`);
  else record('host', null, `room ${CODE} registered once, in the address bar, open to followers from the lobby`);

  // 2. Play on this screen, from a stale link with another code in it.
  const follower = await open('follower', `${origin}/?follow=${CODE}&r=${STALE}&players=1&render=180`);
  const joined = await until(follower.page, () => sockets.some(s => s.role === 'follow' && s.room === 'club-' + CODE) || follower.problems.length, roomTimeout);
  // Long enough for the room to come up and for a bounce to happen.
  await until(follower.page, () => roomUp('follower') || follower.problems.length, roomTimeout);
  await follower.page.waitForTimeout(8000);
  const at = new URL(follower.page.url());
  const codes = [...at.searchParams.entries()].filter(([k]) => k === 'r' || k === 'follow' || k === 'room').map(([k, v]) => `${k}=${v}`);
  const bounced = follower.visited.find(u => new URL(u).pathname.startsWith('/app'));
  if (follower.problems.length) record('follow', follower.problems.join('\n'));
  else if (registered.length !== 1) record('follow', `the follower registered a room of its own (${registered.length} registrations)`);
  else if (bounced) record('follow', `the follower was sent away to ${bounced}`);
  else if (codes.length !== 1 || codes[0] !== `follow=${CODE}`) record('follow', `address bar ${follower.page.url()} carries ${codes.join(', ') || 'no code'}, expected only follow=${CODE}`);
  else if (!joined) record('follow', `never joined room ${CODE}'s signaling`);
  else record('follow', null, `followed room ${CODE}: no room of its own, one code in the address bar, still on the page`);

  if (report) fs.writeFileSync(report, JSON.stringify({ origin, results, log: log.slice(-200) }, null, 1));
  await browser.close();
  server.close();
  process.exit(results.every(r => r.ok) ? 0 : 1);
})().catch(e => { console.error(e); process.exit(2); });
