// The production game exports live outside each replaceable /play/ ZIP.
(() => {
  let pending;
  window.clubGameLibrary = () => pending ??= readLibrary();

  async function readLibrary() {
    const version = document.documentElement.dataset.version || '';
    const response = await fetch('games/library.json?v=' + encodeURIComponent(version));
    if (!response.ok) throw Error('games/library.json: HTTP ' + response.status);
    const games = await response.json();
    if (location.origin !== 'https://splitscreen.club' ||
        !location.pathname.startsWith('/play/') ||
        document.documentElement.hasAttribute('data-custom-shelf')) return games;

    try {
      const overlay = await fetch('/play/godot/library.json', {
        cache: 'no-cache', signal: AbortSignal.timeout(2500)
      });
      if (!overlay.ok) return games;
      const extra = await overlay.json();
      if (!Array.isArray(extra)) return games;
      for (const row of extra) {
        const id = row?.id;
        if (!['ballkickers', 'frog-fighter'].includes(id) || row.folder !== id ||
            row.web_entry !== 'godot/' + id + '/index.html' || typeof row.title !== 'string' ||
            !Number.isInteger(row.min_players) || !Number.isInteger(row.max_players) ||
            row.min_players < 1 || row.max_players < row.min_players || row.max_players > 8 ||
            games.some(g => g.id === id || g.folder === id)) continue;
        games.push({...row, web_cover: 'godot/' + id + '/cover.png'});
      }
    } catch { /* The games already shipped in this ZIP remain playable. */ }
    return games;
  }
})();
