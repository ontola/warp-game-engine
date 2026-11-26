// The gamedev panel: on a page that plays the player's own game
// (`?project=TOKEN`, a play link from gamenight-cloud), its maker can chat
// with the Assistant right here and see each change in the running game.
// Shown only to the signed-in owner (the cloud's session cookie on this
// origin); everyone else just plays. The chat is the game's own (the same
// one the app shows), the build is the cloud's game builder
// (/v1/me/games/ID/build), and a finished change reloads the game here and
// on every couch that follows (window.warp.reload_own_game).
(() => {
  const params = new URLSearchParams(location.search);
  const token = params.get('project');
  if (!token || !/^[0-9a-f]{64}$/.test(token) || params.get('cloud') === 'off') return;
  const origin = (params.get('cloud') || location.origin).replace(/\/+$/, '');
  let csrf = '', game = null, busy = false;

  const api = async (method, path, body) => {
    const headers = { 'content-type': 'application/json' };
    if (method !== 'GET') headers['x-gamenight-csrf'] = csrf;
    const r = await fetch(origin + path, { method, headers, credentials: 'include', body: body ? JSON.stringify(body) : undefined });
    if (!r.ok) throw Object.assign(new Error('HTTP ' + r.status), { status: r.status });
    return r.status === 204 ? null : r.json();
  };

  const css = document.createElement('style');
  css.textContent = `
#gamedev-toggle{position:fixed;right:16px;bottom:16px;z-index:60;display:flex;align-items:center;gap:8px;min-height:44px;padding:0 16px;border:1px solid var(--line);border-radius:999px;background:var(--panel);color:var(--text);font:800 15px Outfit,system-ui,sans-serif;box-shadow:var(--shadow);cursor:pointer;backdrop-filter:blur(8px)}
#gamedev-toggle:hover,#gamedev-toggle:focus-visible{background:var(--violet-deep)}
#gamedev-toggle b{color:var(--amber)}
#gamedev{position:fixed;right:16px;bottom:72px;z-index:60;width:min(380px,calc(100vw - 32px));max-height:min(560px,calc(100vh - 96px));display:flex;flex-direction:column;border:1px solid var(--line);border-radius:18px;background:var(--panel);box-shadow:var(--shadow);backdrop-filter:blur(10px);color:var(--text);font:500 15px/1.45 Outfit,system-ui,sans-serif}
#gamedev[hidden],#gamedev-toggle[hidden]{display:none}
#gamedev header{display:flex;align-items:center;justify-content:space-between;padding:12px 14px 8px;font-weight:800}
#gamedev header button{border:0;background:none;color:var(--muted);font-size:20px;cursor:pointer;min-width:32px;min-height:32px}
#gamedev-log{flex:1;overflow:auto;padding:4px 14px;display:flex;flex-direction:column;gap:8px}
#gamedev-log .m{padding:8px 12px;border-radius:12px;background:rgba(255,255,255,.07);white-space:pre-wrap;align-self:flex-start;max-width:90%}
#gamedev-log .m.me{align-self:flex-end;background:var(--violet-deep)}
#gamedev-log .m.bad{border:1px solid var(--red)}
#gamedev-log .steps{color:var(--muted);font-size:13px}
#gamedev-log .empty{color:var(--muted)}
#gamedev form{display:flex;gap:8px;padding:10px 12px 12px;border-top:1px solid var(--line)}
#gamedev textarea{flex:1;resize:none;min-height:44px;max-height:140px;padding:10px 12px;border:1px solid var(--line);border-radius:12px;background:rgba(0,0,0,.25);color:var(--text);font:inherit}
#gamedev form button{min-width:44px;border:0;border-radius:12px;background:var(--amber);color:#1b1300;font-weight:900;font-size:18px;cursor:pointer}
#gamedev form button:disabled{opacity:.45;cursor:default}`;
  document.head.appendChild(css);

  const toggle = document.createElement('button');
  toggle.id = 'gamedev-toggle'; toggle.type = 'button'; toggle.hidden = true;
  toggle.innerHTML = '<b>✦</b> Change this game';
  const panel = document.createElement('section');
  panel.id = 'gamedev'; panel.hidden = true;
  panel.innerHTML = '<header><span id="gamedev-title">Change this game</span><button type="button" aria-label="Close">×</button></header>'
    + '<div id="gamedev-log"></div>'
    + '<form><textarea rows="1" placeholder="Ask for a change…" maxlength="2000"></textarea><button type="submit" aria-label="Send">↑</button></form>';
  document.body.append(toggle, panel);
  const log = panel.querySelector('#gamedev-log'), form = panel.querySelector('form'), input = form.querySelector('textarea'), send = form.querySelector('button');

  // Typing here is for the panel, not for the game or the page's shortcuts.
  for (const type of ['keydown', 'keyup', 'keypress']) panel.addEventListener(type, e => e.stopPropagation());
  for (const type of ['pointerdown', 'mousedown', 'click', 'wheel']) panel.addEventListener(type, e => e.stopPropagation());
  toggle.addEventListener('pointerdown', e => e.stopPropagation());
  const open = show => {
    panel.hidden = !show;
    try { localStorage.setItem('splitscreen-club-gamedev', show ? 'open' : 'closed'); } catch (e) {}
    if (show) { if (document.pointerLockElement) document.exitPointerLock(); input.focus(); }
  };
  toggle.addEventListener('click', e => { e.stopPropagation(); open(panel.hidden); });
  panel.querySelector('header button').addEventListener('click', () => open(false));

  const bubble = (text, cls) => {
    const el = document.createElement('div');
    el.className = 'm' + (cls ? ' ' + cls : '');
    el.textContent = text;
    log.appendChild(el);
    log.scrollTop = log.scrollHeight;
    return el;
  };
  const render = entries => {
    log.textContent = '';
    if (!entries.length) { const e = document.createElement('div'); e.className = 'empty'; e.textContent = 'Say what should change. The game reloads when it is done.'; log.appendChild(e); }
    for (const e of entries) bubble(e.text, e.role === 'user' ? 'me' : (e.ok === false ? 'bad' : ''));
  };
  const chat = async () => render((await api('GET', `/v1/me/games/${game}/chat`)).entries || []);

  const follow = async (id, steps) => {
    for (;;) {
      await new Promise(done => setTimeout(done, 2000));
      const now = await api('GET', `/v1/me/games/${game}/build`).catch(() => null);
      if (!now || now.id !== id) return { ok: false, text: 'The Assistant stopped.' };
      steps.textContent = (now.steps || []).slice(-3).map(s => s.text).join(' · ') || 'Working on it…';
      if (now.state !== 'working') return { ok: now.state === 'complete', text: now.result || (now.state === 'complete' ? 'Done.' : 'That did not work. Try again.') };
    }
  };

  form.addEventListener('submit', async e => {
    e.preventDefault();
    const text = input.value.trim();
    if (!text || busy) return;
    busy = true; send.disabled = true; input.value = '';
    log.querySelector('.empty')?.remove();
    bubble(text, 'me');
    const steps = bubble('Working on it…', 'steps');
    try {
      await api('POST', `/v1/me/games/${game}/chat`, { role: 'user', text });
      const id = crypto.randomUUID();
      await api('POST', `/v1/me/games/${game}/build`, { request_id: id, text });
      const done = await follow(id, steps);
      await api('POST', `/v1/me/games/${game}/chat`, { role: 'assistant', text: done.text, ok: done.ok });
      steps.remove();
      bubble(done.text, done.ok ? '' : 'bad');
      if (done.ok) window.warp?.reload_own_game?.();
    } catch (err) {
      steps.remove();
      bubble(err.status === 409 ? 'The Assistant is still working on your last request.' : 'That did not work: ' + err.message, 'bad');
    } finally { busy = false; send.disabled = false; }
  });
  input.addEventListener('keydown', e => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); form.requestSubmit(); } });

  // Only for the game's maker: signed in here, and the game is theirs.
  (async () => {
    try {
      const session = await fetch(origin + '/auth/session', { credentials: 'include' });
      if (!session.ok) return;
      csrf = (await session.json()).csrf || '';
      const files = await fetch(origin + '/v1/game-files/' + token, { credentials: 'omit' }).then(r => r.ok ? r.json() : null);
      if (!files?.project) return;
      game = files.project;
      await chat();
      panel.querySelector('#gamedev-title').textContent = 'Change ' + (files.title || 'this game');
      toggle.hidden = false;
      let saved = null;
      try { saved = localStorage.getItem('splitscreen-club-gamedev'); } catch (e) {}
      if (saved === 'open') open(true);
    } catch (e) { /* Not the maker, or offline: the game just plays. */ }
  })();
})();
