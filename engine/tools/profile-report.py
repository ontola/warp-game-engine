#!/usr/bin/env python3
"""Render app::profile::bench reports as an offline, interactive HTML comparison.

No dependencies. Keeps CPU and GPU tracks separate; does not infer FPS or
hardware utilization from pass sums. Open the output in any browser.
"""
import argparse
import json
from pathlib import Path


def load(path):
    report = json.loads(path.read_text(encoding="utf-8-sig"))
    profile = report.get("profile", report)
    if profile.get("schema") != "warp.native-profile.v1":
        raise ValueError(f"{path}: expected warp.native-profile.v1, not a legacy readback benchmark")
    if not profile.get("cpu_frames"):
        raise ValueError(f"{path}: no CPU samples")
    return {"name": path.stem, "profile": profile,
            "scenario": report.get("scenario"), "gpu": report.get("gpu"),
            "diagnostics": report.get("diagnostics")}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reports", type=Path, nargs="+")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    data = [load(p) for p in args.reports]
    # Escape script terminators even if a report contains untrusted labels.
    payload = json.dumps(data, ensure_ascii=True).replace("<", "\\u003c").replace(">", "\\u003e").replace("&", "\\u0026")
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(HTML.replace("__DATA__", payload), encoding="utf-8")
    print(args.out)


HTML = r'''<!doctype html><html lang="en"><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Warp frame cost</title><style>
body{background:#10151e;color:#e4ecf5;font:15px system-ui;margin:0;padding:28px;max-width:1300px;margin:auto}
h1{font-size:30px}p{color:#aebed0;line-height:1.6}select{background:#202c3e;color:inherit;padding:10px;border:1px solid #596e8b;border-radius:6px;max-width:100%}
canvas{width:100%;height:280px;background:#151e2a;border-radius:8px;margin:16px 0}table{border-collapse:collapse;width:100%;font-variant-numeric:tabular-nums}
td,th{padding:9px;text-align:right;border-bottom:1px solid #29384c}td:first-child,th:first-child{text-align:left}th{color:#9ab0c9}
.scroll{overflow:auto}pre{white-space:pre-wrap;overflow-wrap:anywhere;color:#aebed0}.warning{color:#ffcc88}
#frame{min-height:50px}#passes div{display:flex;gap:14px;margin:4px 0;font-size:13px}#passes span{width:190px;flex-shrink:0}#passes i{background:#55d8cc;display:block;min-width:1px;height:16px}
</style><h1>Warp frame cost</h1>
<p>Native-resolution headless rendering. Warmup excluded; no pixel readback. CPU wall time and GPU timestamps overlap: do not add them or call their inverse gameplay FPS. GPU scopes exclude HUD and presentation; completion wait is shown separately in the table.</p>
<div id="warnings" class="warning"></div><label>Inspect run <select id="run"></select></label>
<p id="meta"></p><canvas id="chart" aria-label="CPU submission and GPU interval by frame"></canvas>
<p id="frame">Hover over the graph to inspect a frame.</p><div id="passes"></div>
<h2>Frame costs in milliseconds</h2><p>CPU submission: blue. Measured GPU interval: green. Dashed line: 8.33 ms. Percentiles are nearest-rank; worst frames remain in the data.</p>
<div class="scroll"><table id="summary"></table></div><details><summary>Scenario and diagnostics</summary><pre id="details"></pre></details>
<script>
const runs=__DATA__, $=id=>document.getElementById(id), select=$('run');
const text=(parent,tag,s)=>{const el=document.createElement(tag);el.textContent=s;parent.append(el);return el};
runs.forEach((r,i)=>{const o=text(select,'option',r.name);o.value=i});
const cases=runs.map(r=>JSON.stringify(r.scenario));
if(runs.length>1 && (cases.some(s=>s==='null') || new Set(cases).size>1)) $('warnings').textContent='Scenario metadata is absent or differs; check capture commands before interpreting a comparison.';
const table=$('summary'), head=text(table,'tr','');text(head,'th','Phase');
runs.forEach(r=>text(head,'th',r.name+' · p50 / p95'));
const keys=[...new Set(runs.flatMap(r=>Object.keys(r.profile.summary)))];
keys.sort().forEach(k=>{const tr=text(table,'tr','');text(tr,'td',k);runs.forEach(r=>{const d=r.profile.summary[k];text(tr,'td',d?d.p50.toFixed(3)+' / '+d.p95.toFixed(3):'—')})});
let active, samples, gpu, ymax, ctx, width;
function draw(){
active=runs[Number(select.value)];samples=active.profile.cpu_frames;gpu=new Map(active.profile.gpu_frames.map(g=>[g.frame,g]));
const p=active.profile;
$('meta').textContent=`${p.size.join(' × ')} · ${p.frames} measured / ${p.warmup} warmup · GPU samples ${p.gpu_samples} · dropped ${p.dropped} · errors ${p.errors} · pending ${p.pending}`;
$('details').textContent=JSON.stringify({scenario:active.scenario,gpu:active.gpu,diagnostics:active.diagnostics},null,2);
const c=$('chart'),dpr=window.devicePixelRatio||1;width=c.clientWidth;c.width=width*dpr;c.height=280*dpr;ctx=c.getContext('2d');ctx.scale(dpr,dpr);
const interval=g=>g?.intervals.length?Math.max(...g.intervals.map(x=>x.end_ms))-Math.min(...g.intervals.map(x=>x.start_ms)):null;
ymax=Math.max(10,...samples.map(s=>s.submit_total_ms),...p.gpu_frames.map(interval))*1.1;
const y=v=>250-v/ymax*225;
ctx.font='12px system-ui';ctx.fillStyle='#9ab0c9';ctx.fillText(ymax.toFixed(1)+' ms',10,18);ctx.fillText('frame '+samples[0].frame,10,272);ctx.fillText('frame '+samples.at(-1).frame,width-90,272);
ctx.strokeStyle='#76859a';ctx.setLineDash([4,4]);ctx.beginPath();ctx.moveTo(0,y(8.333));ctx.lineTo(width,y(8.333));ctx.stroke();ctx.setLineDash([]);
for(const [color,value] of [['#79aaff',s=>s.submit_total_ms],['#55d8cc',s=>interval(gpu.get(s.frame))]]){ctx.strokeStyle=color;ctx.lineWidth=1.5;ctx.beginPath();let started=false;samples.forEach((s,i)=>{const v=value(s);if(v==null){started=false;return}const x=i/Math.max(1,samples.length-1)*width;started?ctx.lineTo(x,y(v)):ctx.moveTo(x,y(v));started=true});ctx.stroke()}
}
$('chart').addEventListener('pointermove',e=>{
const i=Math.max(0,Math.min(samples.length-1,Math.round(e.offsetX/width*(samples.length-1)))),s=samples[i],g=gpu.get(s.frame);
$('frame').textContent=`Frame ${s.frame}: CPU submit ${s.submit_total_ms.toFixed(2)} ms · simulation ${s.tick_ms.toFixed(2)} · game draw ${s.draw.game_ms.toFixed(2)} · encode ${s.draw.render_ms.toFixed(2)} · HUD ${s.draw.hud_ms.toFixed(2)} · completion wait ${s.completion_wait_ms.toFixed(2)} ms`;
$('passes').replaceChildren();if(!g)return;
g.intervals.filter(x=>x.parent==null).forEach(p=>{const row=text($('passes'),'div','');text(row,'span',p.name+' ['+(p.view??'shared')+']');const bar=text(row,'i','');bar.style.width=Math.min(65,(p.end_ms-p.start_ms)/ymax*65)+'%';text(row,'b',(p.end_ms-p.start_ms).toFixed(3)+' ms')});
});select.addEventListener('change',draw);window.addEventListener('resize',draw);draw();
</script></html>'''

if __name__ == "__main__":
    main()
