// Errors of the browser host go to Sentry: an engine panic (warpCrash), a
// partial GPU failure (warpGpuProblem), a failed start and uncaught page
// errors. No Sentry SDK (the wasm is big enough): one envelope POST per
// error, at most five per page load, each message once.
//
// What a report holds: the error, the build (data-version), the browser's
// user agent, the GPU and limits lines the engine pinned in #warp-log, the
// canvas and window size and the game on screen. Nothing personal: no
// device id, room code, address query or cookies, and Sentry is asked not
// to keep the sender's IP address.
(() => {
  // The project's DSN. Empty: nothing is sent.
  const DSN = 'https://d2f7ad1543dc8d42a8100168640f3f0e@o1378995.ingest.us.sentry.io/4512486860718080';
  const MAX = 5, seen = new Set();
  let sent = 0;
  const dsn = /^https:\/\/([0-9a-f]+)@([^/]+)\/(\d+)$/.exec(DSN);
  const hex = n => Array.from(crypto.getRandomValues(new Uint8Array(n)), b => b.toString(16).padStart(2, '0')).join('');
  // Room codes, device ids and tokens out of free text.
  const scrub = text => String(text)
    .replace(/[0-9a-f]{24,}/gi, '[id]')
    .replace(/(https?:\/\/[^\s?#)]*)[?#][^\s:)]*/g, '$1')
    .replace(/\b(room|code)( code)?:? [A-Z0-9]{4,8}\b/g, '$1 [code]');
  const pinned = () => {
    const lines = (document.getElementById('warp-log')?.textContent || '').split('\n');
    return {
      gpu: lines.find(l => l.startsWith('GPU: ')) || null,
      limits: lines.find(l => / storage buffers, \d+ sampled textures/.test(l)) || null,
      log: scrub(lines.filter(Boolean).slice(-16).join('\n')),
    };
  };
  const report = (kind, message, stack, level = 'error') => {
    const text = scrub(message || 'unknown error').slice(0, 4000);
    const key = kind + '\n' + text;
    if (!dsn || sent >= MAX || seen.has(key) || location.protocol === 'file:') return;
    seen.add(key); sent++;
    const root = document.documentElement, canvas = document.getElementById('warp');
    const version = root.dataset.version || 'dev';
    const game = root.getAttribute('data-playing') || new URLSearchParams(location.search).get('game') || 'room';
    const { gpu, limits, log } = pinned();
    const [type, ...rest] = text.split('\n');
    const id = hex(16);
    const event = {
      event_id: id,
      timestamp: Date.now() / 1000,
      platform: 'javascript',
      level,
      logger: 'browser-host',
      release: 'warp-browser@' + version.split('-')[0],
      dist: version,
      environment: /(^|\.)splitscreen\.club$/.test(location.hostname) ? 'production' : 'local',
      // The first line names the issue (a panic's place, a GPU error's kind).
      exception: { values: [{ type: kind, value: type.slice(0, 300) + (rest.length ? '\n' + rest.join('\n') : ''), mechanism: { type: kind, handled: kind !== 'crash' } }] },
      tags: {
        kind, game,
        gpu: gpu ? gpu.replace(/^GPU: /, '').replace(/, \d+x\d+.*$/, '').slice(0, 200) : 'unknown',
        renderer: !limits ? 'unknown' : /portable/.test(limits) ? 'portable' : /no trees/.test(limits) ? 'no-trees' : 'full',
        cloud: new URLSearchParams(location.search).get('cloud') === 'off' ? 'off' : 'on',
      },
      // Relay keeps no sender IP address for this SDK.
      sdk: { name: 'splitscreen.browser', version: '1', settings: { infer_ip: 'never' } },
      request: { url: location.origin + location.pathname, headers: { 'User-Agent': navigator.userAgent } },
      contexts: {
        screen: {
          type: 'screen',
          canvas: canvas ? canvas.width + 'x' + canvas.height : null,
          window: innerWidth + 'x' + innerHeight,
          device_pixel_ratio: devicePixelRatio,
          fullscreen: !!document.fullscreenElement,
        },
        gpu_limits: { type: 'default', gpu, limits },
      },
      extra: { stack: stack ? scrub(stack).slice(0, 4000) : null, log, uptime_s: Math.round(performance.now() / 100) / 10, gamepads: [...(navigator.getGamepads?.() || [])].filter(Boolean).length },
    };
    const body = JSON.stringify({ event_id: id, sent_at: new Date().toISOString() }) + '\n' + JSON.stringify({ type: 'event' }) + '\n' + JSON.stringify(event);
    // text/plain: no CORS preflight. keepalive: also while the page closes.
    const post = url => fetch(url, { method: 'POST', body, keepalive: body.length < 60000, headers: { 'content-type': 'text/plain;charset=UTF-8' }, credentials: 'omit' });
    const direct = () => post(`https://${dsn[2]}/api/${dsn[3]}/envelope/?sentry_key=${dsn[1]}&sentry_version=7&sentry_client=splitscreen-browser/1`).catch(() => {});
    // Through splitscreen.club first: Zen (Firefox) and blockers drop
    // requests to sentry.io (gamenight-cloud browser_errors.rs).
    if (/(^|\.)splitscreen\.club$/.test(location.hostname)) post('/v1/telemetry/browser-errors').then(r => { if (!r.ok) direct(); }, direct);
    else direct();
  };
  // Uncaught errors of the page itself; not browser extensions' (another
  // origin's script shows as "Script error." without a file).
  // After an engine panic the wasm is dead and every later call fails
  // ("unreachable", "Module terminated"): those follow the panic's report.
  const echo = text => crashed && /unreachable|Module terminated/i.test(String(text));
  let crashed = false;
  const reportAny = report;
  window.splitscreenReport = (kind, ...rest) => { if (kind === 'crash') crashed = true; return reportAny(kind, ...rest); };
  addEventListener('error', e => {
    if (!e.message || e.message === 'Script error.' || (e.filename && !e.filename.startsWith(location.origin)) || echo(e.message)) return;
    report('page-error', e.message + (e.filename ? '\nat ' + e.filename.replace(location.origin, '').replace(/\?.*$/, '') + ':' + e.lineno + ':' + e.colno : ''), e.error?.stack);
  });
  addEventListener('unhandledrejection', e => {
    const r = e.reason;
    if (echo(r instanceof Error ? r.message : r)) return;
    report('unhandled-rejection', r instanceof Error ? r.message : String(r), r?.stack);
  });

  // What throws nothing: a start that never finishes (the loading card's
  // step unchanged for 60 s), the page frozen for 10 s or more, and the
  // cloud room failing (no QR code, no phones). Warnings, once each.
  let step = '', since = performance.now();
  setInterval(() => {
    const card = document.getElementById('warp-progress'), text = document.getElementById('warp-step')?.textContent || '';
    if (!card || card.hidden || card.classList.contains('direct-games')) { step = ''; since = performance.now(); return; }
    if (text !== step) { step = text; since = performance.now(); return; }
    const waited = (performance.now() - since) / 1000;
    // Waiting for a player is not stuck.
    if (waited > 60 && !/press a|join|could not/i.test(text)) report('stall', 'stuck at: ' + text.slice(0, 120) + '\nfor ' + Math.round(waited) + ' s', null, 'warning');
  }, 5000);
  let last = performance.now();
  const beat = now => {
    const gap = now - last;
    last = now;
    // Hidden tabs get no frames: only a gap while the page is shown counts.
    if (gap > 10000 && document.visibilityState === 'visible' && !document.hidden) report('freeze', 'the page drew nothing for ' + Math.round(gap / 1000) + ' s', null, 'warning');
    requestAnimationFrame(beat);
  };
  requestAnimationFrame(beat);
  document.addEventListener('visibilitychange', () => { last = performance.now(); });
  const watchLog = () => {
    const box = document.getElementById('warp-log');
    if (!box) return setTimeout(watchLog, 500);
    new MutationObserver(() => {
      const line = box.textContent.split('\n').find(l => /^cloud room: .*HTTP [45]\d\d/.test(l));
      if (line) report('cloud', line.replace(/^cloud room: /, ''), null, 'warning');
    }).observe(box, { childList: true, characterData: true, subtree: true });
  };
  watchLog();
})();
