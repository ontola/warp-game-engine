// Background jobs for the page's games (app/src/web_jobs.rs): CPU work that
// natively has its own thread, such as Growing Guns' stadium lighting bake.
// Each message: {id, name, input (bytes), module, version}; the answer is
// {id, output} or {id, error}. One module instance serves every job.
let ready = null;
self.onmessage = async ({data}) => {
  try {
    if (!ready) ready = (async () => {
      const glue = new URL('./pkg/browser.js', import.meta.url);
      glue.searchParams.set('v', data.version);
      const api = await import(glue.href);
      await api.default({module_or_path: data.module});
      return api;
    })();
    const api = await ready;
    const output = api.worker_job(data.name, data.input);
    self.postMessage({id: data.id, output}, [output.buffer]);
  } catch (error) {
    self.postMessage({id: data.id, error: String(error?.message || error)});
  }
};
