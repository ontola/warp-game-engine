#!/usr/bin/env python3
"""A web page from a look-report.py report dir: before/after renders, clips
and GPU ms per pass group, per scenario and angle.

    look-page.py REPORT_DIR [--title NAME]

Writes REPORT_DIR/index.html next to report.json, images/ and clips/, all
referenced relatively, so the dir can be published or opened as is.
"""
import argparse
import html
import json
from pathlib import Path

GROUPS = ["selection", "voxel walk", "splat raster", "occluder", "remaining", "sun", "terrain", "shade", "other"]

STYLE = """
/* Lab bench: dark viewing ground for renders, one signal hue for changes. */
:root{--bg:#f3f4f1;--panel:#ffffff;--fg:#1d2320;--muted:#5d6762;--line:#d6dbd6;--accent:#2f7d4f;--worse:#b4462d;--better:#2f7d4f;
--display:"Archivo",system-ui,sans-serif;--body:"IBM Plex Sans",system-ui,sans-serif;--mono:"IBM Plex Mono",ui-monospace,monospace;color-scheme:light}
@media (prefers-color-scheme:dark){:root:not([data-theme="light"]){--bg:#121513;--panel:#1a1e1b;--fg:#e4e8e3;--muted:#98a29b;--line:#2c332e;--accent:#6fc28e;--worse:#e58a6f;--better:#6fc28e;color-scheme:dark}}
:root[data-theme="dark"]{--bg:#121513;--panel:#1a1e1b;--fg:#e4e8e3;--muted:#98a29b;--line:#2c332e;--accent:#6fc28e;--worse:#e58a6f;--better:#6fc28e;color-scheme:dark}
body{background:var(--bg);color:var(--fg);font:15px/1.5 var(--body);padding-left:max(16px,env(safe-area-inset-left,0px));padding-right:max(16px,env(safe-area-inset-right,0px));padding-block:24px 48px}
main{max-width:1400px;margin:0 auto;display:grid;gap:40px}
h1{font:700 clamp(26px,4vw,38px)/1.1 var(--display);margin:0;text-wrap:balance}
h2{font:700 22px/1.2 var(--display);margin:0;text-wrap:balance}
h3{font:600 15px/1.3 var(--body);margin:0}
.meta{color:var(--muted);font:13px/1.5 var(--mono);display:flex;flex-wrap:wrap;gap:4px 18px;margin-top:8px}
.eyebrow{font:600 11px/1 var(--mono);letter-spacing:.08em;text-transform:uppercase;color:var(--muted)}
.scroll{overflow-x:auto}
table{border-collapse:collapse;font:13px/1.4 var(--mono);font-variant-numeric:tabular-nums;width:100%}
th,td{padding:6px 10px;border-bottom:1px solid var(--line);text-align:right;white-space:nowrap}
th:first-child,td:first-child{text-align:left}
th{color:var(--muted);font-weight:500}
.better{color:var(--better)}.worse{color:var(--worse)}
section{display:grid;gap:16px}
.view{display:grid;gap:10px;padding-block:16px;border-top:1px solid var(--line)}
.pair{display:grid;grid-template-columns:repeat(auto-fit,minmax(min(100%,420px),1fr));gap:10px}
figure{margin:0;display:grid;gap:6px;min-width:0}
figure img,figure video{width:100%;max-width:100%;display:block;border-radius:4px;background:#000}
figcaption{font:12px/1.4 var(--mono);color:var(--muted)}
.about{color:var(--muted);max-width:70ch}
.bars{display:grid;gap:3px;font:12px/1.3 var(--mono);font-variant-numeric:tabular-nums}
.bar{display:grid;grid-template-columns:9em 1fr 5.5em;gap:8px;align-items:center}
.track{position:relative;height:10px}
.track span{position:absolute;left:0;height:4px;border-radius:2px}
.track .b{top:0;background:var(--muted);opacity:.55}.track .a{top:5px;background:var(--accent)}
.legend{display:flex;gap:16px;font:12px var(--mono);color:var(--muted)}
.legend i{display:inline-block;width:14px;height:4px;border-radius:2px;margin-right:6px;vertical-align:middle}
nav{display:flex;flex-wrap:wrap;gap:6px 14px;font:13px var(--mono)}
a{color:var(--accent)}a:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
"""


def fmt(v, digits=2):
    return "–" if v is None else f"{v:.{digits}f}"


def delta(after, before):
    if after is None or before is None:
        return ""
    d = after - before
    cls = "better" if d < -0.05 else "worse" if d > 0.05 else ""
    return f'<span class="{cls}">{d:+.2f}</span>'


def bars(view):
    after = view.get("groups") or {}
    before = (view.get("base") or {}).get("groups") or {}
    top = max([*after.values(), *before.values(), 0.01])
    rows = []
    for g in GROUPS:
        a, b = after.get(g), before.get(g)
        if (a or 0) < 0.05 and (b or 0) < 0.05:
            continue
        rows.append(f'<div class="bar"><span>{g}</span><div class="track">'
                    + (f'<span class="b" style="width:{(b or 0) / top * 100:.1f}%"></span>' if before else "")
                    + f'<span class="a" style="width:{(a or 0) / top * 100:.1f}%"></span></div>'
                    f'<span>{fmt(a)}{" " + delta(a, b) if before else ""}</span></div>')
    return '<div class="bars">' + "".join(rows) + "</div>"


def metrics(view):
    parts = []
    if view.get("crown_rgb"):
        parts.append(f"crown RGB {view['crown_rgb']}" + (f" (x{view['crown_ratio']} of nearest)" if view.get("crown_ratio") else ""))
    if view.get("grid"):
        g = view["grid"]
        parts.append(f"grass grid c={g['c']}" + (f" (was {view['base']['grid']['c']})" if (view.get("base") or {}).get("grid") else ""))
    if view.get("largest_cell"):
        parts.append(f"largest cell {view['largest_cell']['px']} px")
    if view.get("selected") is not None:
        parts.append(f"selected {view['selected']:,}")
    return " · ".join(parts)


def page(report, title):
    e = html.escape
    views = report["views"]
    base = report.get("base")
    scenarios = {}
    for name, v in views.items():
        scenarios.setdefault(v["scenario"], []).append((name, v))
    head = (f"<title>{e(title)}</title><meta name=\"claude:viewport-fit-sides\" content=\"cover\">"
            '<link rel="preconnect" href="https://fonts.googleapis.com"><link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>'
            '<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Archivo:wght@700&family=IBM+Plex+Mono:wght@400;500;600&family=IBM+Plex+Sans:wght@400;600&display=swap">'
            f"<style>{STYLE}</style>")
    meta = [f"after {e(report['label'])} @ {e(str(report['commit']))}"]
    if base:
        meta.append(f"before {e(str(base['label']))} @ {e(str(base['commit']))}")
    meta += [e(str(report.get("adapter") or "")), e(str(report.get("size") or "")), e(str(report.get("at") or ""))]
    out = [head, "<main><header>", '<div class="eyebrow">Warp look suite</div>', f"<h1>{e(title)}</h1>",
           '<div class="meta">' + "".join(f"<span>{m}</span>" for m in meta if m) + "</div></header>"]
    # Summary: frame GPU ms per view, before and after.
    rows = "".join(f'<tr><td><a href="#{e(n.replace(".", "-"))}">{e(n)}</a></td><td>{fmt((v.get("base") or {}).get("gpu_ms"))}</td>'
                   f'<td>{fmt(v["gpu_ms"])}</td><td>{delta(v["gpu_ms"], (v.get("base") or {}).get("gpu_ms"))}</td></tr>' for n, v in views.items())
    out.append('<section><h2>Frame GPU time</h2><div class="scroll"><table><thead><tr><th>view</th><th>before ms</th><th>after ms</th><th>change</th></tr></thead>'
               f"<tbody>{rows}</tbody></table></div></section>")
    if report.get("clips"):
        clips = []
        for name, pair in report["clips"].items():
            figs = "".join(f'<figure><video src="{e(pair[k])}" controls muted loop playsinline preload="metadata"></video><figcaption>{k}</figcaption></figure>'
                           for k in ("before", "after") if k in pair)
            clips.append(f'<div class="view"><h3>{e(name)}</h3><div class="pair">{figs}</div></div>')
        out.append("<section><h2>Approach clips</h2>" + "".join(clips) + "</section>")
    out.append("<nav>" + "".join(f'<a href="#{e(s)}">{e(s)}</a>' for s in scenarios) + "</nav>")
    legend = '<div class="legend"><span><i style="background:var(--muted);opacity:.55"></i>before</span><span><i style="background:var(--accent)"></i>after</span></div>' if base else ""
    for scenario, items in scenarios.items():
        out.append(f'<section id="{e(scenario)}"><h2>{e(scenario)}</h2>{legend}')
        for name, v in items:
            figs = ""
            if v.get("base"):
                figs += f'<figure><img loading="lazy" src="{e(v["base"]["image"])}" alt="{e(name)} before"><figcaption>before · {fmt(v["base"]["gpu_ms"])} ms</figcaption></figure>'
            figs += f'<figure><img loading="lazy" src="{e(v["image"])}" alt="{e(name)} after"><figcaption>after · {fmt(v["gpu_ms"])} ms</figcaption></figure>'
            about = f'<p class="about">{e(v["about"])}</p>' if v.get("about") else ""
            m = metrics(v)
            out.append(f'<div class="view" id="{e(name.replace(".", "-"))}"><h3>{e(v.get("angle") or name)}</h3>{about}'
                       f'<div class="pair">{figs}</div>{bars(v)}' + (f'<div class="meta">{e(m)}</div>' if m else "") + "</div>")
        out.append("</section>")
    out.append("</main>")
    return "\n".join(out)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("dir")
    parser.add_argument("--title", default="Look suite")
    args = parser.parse_args()
    folder = Path(args.dir)
    report = json.loads((folder / "report.json").read_text(encoding="utf-8"))
    (folder / "index.html").write_text(page(report, args.title), encoding="utf-8")
    print(folder / "index.html")


if __name__ == "__main__":
    main()
