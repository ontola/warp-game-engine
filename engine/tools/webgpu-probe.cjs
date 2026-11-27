#!/usr/bin/env node
// Does this machine's browser give a page a WebGPU device? Prints one JSON
// line per browser. Used by .github/workflows/webgpu-probe.yml to see
// whether a CI runner can run the /play/ smoke test in WebKit or Safari.
//   node engine/tools/webgpu-probe.cjs [webkit|chromium|firefox|safari ...]
'use strict';
const http = require('http');
const PAGE = `<!doctype html><title>probe</title><canvas id=c width=64 height=64></canvas><script>
window.probe = (async () => {
  if (!navigator.gpu) return { gpu: false };
  const a = await navigator.gpu.requestAdapter();
  if (!a) return { gpu: true, adapter: false };
  const info = a.info || {};
  const d = await a.requestDevice();
  const m = d.createShaderModule({ code: '@compute @workgroup_size(1) fn main() {}' });
  const ci = await m.getCompilationInfo();
  const ctx = document.getElementById('c').getContext('webgpu');
  ctx.configure({ device: d, format: navigator.gpu.getPreferredCanvasFormat() });
  return { gpu: true, adapter: true, vendor: info.vendor, architecture: info.architecture, description: info.description,
    sampledTextures: d.limits.maxSampledTexturesPerShaderStage, storageBuffers: d.limits.maxStorageBuffersPerShaderStage,
    shaderMessages: ci.messages.length, canvas: true, ua: navigator.userAgent };
})().catch(e => ({ error: String(e) }));
</script>`;
(async () => {
  const server = http.createServer((q, r) => { r.writeHead(200, { 'Content-Type': 'text/html' }); r.end(PAGE); });
  await new Promise(ok => server.listen(0, '127.0.0.1', ok));
  const url = `http://127.0.0.1:${server.address().port}/`;
  for (const name of process.argv.slice(2)) {
    let out;
    try {
      if (name === 'safari') {
        const { Builder } = require('selenium-webdriver');
        const d = await new Builder().forBrowser('safari').build();
        try {
          await d.get(url);
          out = await d.executeAsyncScript('const done = arguments[arguments.length - 1]; window.probe.then(done);');
        } finally { await d.quit(); }
      } else {
        const b = await require('playwright')[name].launch({ headless: false,
          firefoxUserPrefs: name === 'firefox' ? { 'dom.webgpu.enabled': true, 'gfx.webgpu.ignore-blocklist': true } : undefined });
        try { const p = await b.newPage(); await p.goto(url); out = await p.evaluate(() => window.probe); out.version = b.version(); }
        finally { await b.close(); }
      }
    } catch (e) { out = { error: String(e).slice(0, 500) }; }
    console.log(JSON.stringify({ browser: name, ...out }));
  }
  server.close();
})();
