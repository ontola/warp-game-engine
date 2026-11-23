// Installed by cache_web_shelf.py on an opted-in shelf. Other requests keep native fetch semantics.
(() => {
  const config = JSON.parse(document.getElementById('warp-shelf-assets').textContent);
  const root = new URL('./', document.baseURI);
  const release = document.documentElement.dataset.version;
  const nativeFetch = window.fetch.bind(window);
  function lookup(input) {
    let url;
    try { url = new URL(input, document.baseURI); } catch { return; }
    if (url.origin !== root.origin || !url.pathname.startsWith(root.pathname)) return;
    const path = url.pathname.slice(root.pathname.length);
    if (!Object.hasOwn(config.keys, path)) return;
    const key = config.keys[path];
    if (url.search && url.search !== '?v=' + release && url.search !== '?v=' + key) return;
    url.search = key ? '?v=' + key : '';
    return {url, path};
  }
  window.warpShelfAsset = input => lookup(input)?.url ?? new URL(input, document.baseURI);
  async function download(response, report, total) {
    if (!report || !response.body) return response.blob();
    const chunks = [];
    let received = 0;
    for (const reader = response.body.getReader();;) {
      const {done, value} = await reader.read();
      if (done) break;
      chunks.push(value);
      received += value.byteLength;
      report(Math.min(0.29, 0.3 * received / total), `Downloading the club (${(received / 1e6).toFixed(1)} MB)`);
    }
    return new Blob(chunks);
  }
  async function load(asset, report) {
    const {url, path} = asset, packed = config.compressed[path];
    if (!packed || typeof DecompressionStream !== 'function') return nativeFetch(url, {cache:'force-cache'});
    // An existing player may already have the uncompressed, versioned response.
    // An only-if-cached miss never sends that large file over the network.
    try {
      const cached = await nativeFetch(url, {cache:'only-if-cached', mode:'same-origin'});
      if (cached.ok) return cached;
    } catch {}
    try {
      const [digest, size, wireSize, mime] = packed;
      const response = await nativeFetch(new URL('pkg/compressed/' + digest + '.gz', root), {cache:'force-cache'});
      if (!response.ok) throw Error('HTTP ' + response.status);
      const blob = await download(response, report, wireSize);
      const prefix = new Uint8Array(await blob.slice(0, 2).arrayBuffer());
      // HTTP may already have decoded Content-Encoding. Check the actual payload.
      const stream = prefix[0] === 31 && prefix[1] === 139
        ? blob.stream().pipeThrough(new DecompressionStream('gzip')) : blob.stream();
      const bytes = await new Response(stream).arrayBuffer();
      if (bytes.byteLength !== size) throw Error('Invalid asset length');
      const hash = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
      const hex = Array.from(hash, byte => byte.toString(16).padStart(2,'0')).join('');
      if (hex !== digest) throw Error('Invalid asset digest');
      return new Response(bytes, {headers:{'Content-Type':mime,'Content-Length':String(size)}});
    } catch {
      // Corrupt transport, server without sidecars, or missing platform capability.
      return nativeFetch(url, {cache:'force-cache'});
    }
  }
  window.fetch = (input, options) => {
    // The Rust loader and editor request local files by URL without Request options.
    // API requests, explicit cache policies, ranges, signals and unknown queries pass through.
    if (options !== undefined || !(typeof input === 'string' || input instanceof URL)) return nativeFetch(input, options);
    const asset = lookup(input);
    return asset ? load(asset) : nativeFetch(input);
  };
  window.warpShelfWasm = async report => {
    const asset = lookup(config.wasm), response = await load(asset, report);
    if (!response.ok) throw Error(config.wasm + ': HTTP ' + response.status);
    return response.arrayBuffer();
  };
})();
