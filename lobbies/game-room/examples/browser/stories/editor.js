const editor = document.querySelector('#story-editor');
const trigger = document.querySelector('#open-stories');
const templates = editor.querySelector('select');
const code = editor.querySelector('textarea');
const feedback = editor.querySelector('output');
const run = editor.querySelector('[data-run]');
const restore = editor.querySelector('[data-reset]');
const layoutSelected = () => templates.selectedOptions[0]?.dataset.kind === 'layout';
let game, sample, module, runtime, activeFile, generation = 0, busy = false;
let draft = new Map();
const key = file => 'warp-story:' + game + ':' + file;
function save() {
  if (!activeFile) return;
  draft.set(key(activeFile), code.value);
  try { localStorage.setItem(key(activeFile), code.value); } catch {}
}
async function loadTemplate() {
  const file = templates.value, token = ++generation;
  activeFile = null;
  run.disabled = restore.disabled = true;
  feedback.textContent = '';
  run.textContent = layoutSelected() ? 'Apply layout' : 'Run story';
  if (!file) return;
  try {
    const path = './games/' + encodeURIComponent(game) + '/' + encodeURIComponent(file);
    const res = await fetch(path);
    if (!res.ok) throw Error('Could not load this template');
    const text = await res.text();
    if (generation !== token) return;
    sample = text; activeFile = file;
    let saved = draft.get(key(file));
    try { saved ??= localStorage.getItem(key(file)); } catch {}
    code.value = saved ?? text;
    run.disabled = restore.disabled = false;
  } catch (e) { if (generation === token) feedback.textContent = e.message; }
}
async function session() {
  save();
  game = document.documentElement.dataset.playing;
  activeFile = null; trigger.hidden = true;
  editor.close(); templates.replaceChildren();
  const token = ++generation;
  if (!game || !/^[a-z0-9-]+$/.test(game)) return;
  try {
    const library = await window.clubGameLibrary();
    if (generation !== token || library.some(g => g.web_entry && g.id === game)) return;
    const base = './games/' + game + '/';
    const files = await (await fetch(base + 'files.json')).json();
    if (!files.includes('stories.json')) return;
    const entries = await (await fetch(base + 'stories.json')).json();
    if (generation !== token) return;
    for (const item of entries) {
      if (files.includes(item.file) && /^[a-z0-9_]+\.module\.luau$/.test(item.file)) {
        const option = new Option(item.title, item.file);
        if (item.kind === 'layout') option.dataset.kind = 'layout';
        templates.add(option);
      }
    }
    const hasLayout = Boolean(templates.querySelector('option[data-kind="layout"]'));
    const name = hasLayout ? 'Luau stories & map' : 'Luau stories';
    trigger.textContent = name + ' · F2';
    editor.querySelector('h2').textContent = name;
    trigger.hidden = !templates.length;
  } catch {}
}
function open() {
  if (trigger.hidden || editor.open) return;
  document.exitPointerLock?.();
  editor.showModal();
  if (!activeFile) loadTemplate();
  templates.focus();
}
function close() { save(); editor.close(); document.querySelector('#warp').focus(); }
function evaluate(source) {
  return new Promise((resolve,reject) => {
    const worker = new Worker(new URL('./worker.js',import.meta.url),{type:'module'});
    const finish = (ok,value) => { clearTimeout(timeout); worker.terminate(); (ok ? resolve : reject)(value); };
    const timeout = setTimeout(() => finish(false, Error('Story took too long to generate')), 7000);
    worker.onmessage = ({data}) => data.error ? finish(false,Error(data.error)) : finish(true,data.definition);
    worker.onerror = e => finish(false,Error(e.message));
    worker.postMessage({module,source,version:document.documentElement.dataset.version});
  });
}
async function play() {
  if (busy || run.disabled || !activeFile) return;
  const token = generation, owner = game;
  busy = run.disabled = templates.disabled = restore.disabled = true;
  const isLayout = layoutSelected();
  feedback.textContent = isLayout ? 'Applying layout…' : 'Starting story…'; save();
  try {
    const definition = await evaluate(code.value);
    if (token !== generation) return;
    runtime.start_story(owner, definition);
    const deadline = performance.now() + 12000;
    while (token === generation && performance.now() < deadline) {
      const raw = runtime.story_reply();
      if (raw) {
        const reply = JSON.parse(raw);
        if (reply.error) throw Error(reply.error);
        feedback.textContent = isLayout ? 'Applied ' + reply.title : 'Playing ' + reply.title;
        close();
        return;
      }
      await new Promise(resolve => setTimeout(resolve,45));
    }
    if (token === generation) throw Error('The game did not respond. Resume the city and try again.');
  } catch (e) {
    if (generation === token) feedback.textContent = String(e?.message || e);
  } finally {
    busy = templates.disabled = false;
    run.disabled = restore.disabled = !activeFile;
  }
}
export function connectStories(warp, compiledModule) {
  runtime = warp; module = compiledModule;
  new MutationObserver(session).observe(document.documentElement, {attributes:true,attributeFilter:['data-playing']});
  session();
}
trigger.onclick = open; editor.querySelector('[data-close]').onclick = close;
run.onclick = play;
restore.onclick = () => { code.value = sample; save(); feedback.textContent = 'Restored template'; };
const fileInput = editor.querySelector('input[type=file]');
editor.querySelector('[data-open]').onclick = () => fileInput.click();
fileInput.onchange = async () => {
  const [file] = fileInput.files; fileInput.value='';
  if (!file || !activeFile || busy) return;
  if (file.size > 64 * 1024) { feedback.textContent='Story files can be up to 64 KiB'; return; }
  const token = generation;
  try {
    const source = await file.text();
    if (token === generation) { code.value=source; save(); feedback.textContent='Opened ' + file.name; }
  } catch { feedback.textContent='Could not read this file'; }
};
editor.querySelector('[data-save]').onclick = () => {
  if (!activeFile) return;
  const link=document.createElement('a');
  const url=URL.createObjectURL(new Blob([code.value],{type:'text/plain;charset=utf-8'}));
  link.href=url; link.download=activeFile.replace('.module.luau','-custom.luau');
  link.click(); setTimeout(() => URL.revokeObjectURL(url),1000);
};
templates.onchange = () => { save(); loadTemplate(); };
code.oninput = () => { save(); feedback.textContent = ''; };
editor.oncancel = e => { e.preventDefault(); close(); };
window.addEventListener('keydown', e => {
  if (e.code === 'F2' && !trigger.hidden) {
    e.preventDefault(); e.stopImmediatePropagation();
    if (!e.repeat) editor.open ? close() : open();
  }
}, true);
for (const type of ['keydown','keyup','pointerdown','pointerup','click','wheel']) {
  editor.addEventListener(type, e => {
    e.stopPropagation();
    if (type === 'keydown' && e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); play(); }
  });
}
