self.onmessage = async ({data}) => {
  try {
    const glue = new URL('../pkg/browser.js', import.meta.url);
    glue.searchParams.set('v',data.version);
    const {default:init, evaluate_story} = await import(glue.href);
    await init({module_or_path: data.module});
    self.postMessage({definition: evaluate_story(data.source)});
  } catch (error) {
    self.postMessage({error: error?.message === 'unreachable'
      ? 'Luau stopped while evaluating this script. Check its syntax and runtime expressions.'
      : String(error?.message || error)});
  }
};
