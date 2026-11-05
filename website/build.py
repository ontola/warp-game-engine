#!/usr/bin/env python3
"""Build the Warp website from the repository's markdown.

    python3 website/build.py                 # writes website/_site
    python3 website/build.py --out DIR --ref main --strict

Only the Python standard library is used. The markdown files in the
repository are the single source: this script converts them to HTML pages,
rewrites links between them, and points links to code at GitHub. The
navigation comes from engine/docs/README.md (its Guides table) and
engine/docs/log/README.md (its sections of dated notes), so adding a guide
or a log entry there adds it to the site.

The converter handles the markdown the docs use: headings, paragraphs,
lists (nested), fenced code, tables, block quotes, rules, emphasis, inline
code and links. Raw HTML in markdown is escaped, not passed through.
"""
from __future__ import annotations

import argparse
import html
import posixpath
import re
import shutil
import sys
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SITE_SRC = ROOT / "website"
REPO_URL = "https://github.com/ontola/warp-game-engine"
INDEX = "engine/docs/README.md"
LOG_INDEX = "engine/docs/log/README.md"

# Markdown outside engine/docs that also becomes a page: repo path -> page.
EXTRA_PAGES = {
    "engine/README.md": "docs/engine.html",
    "engine/tools/README.md": "docs/tools.html",
    "engine/tools/frame-budget.md": "docs/frame-budget.html",
    "games/sideways/README.md": "docs/sideways.html",
    "games/growing-guns/README.md": "docs/growing-guns.html",
    "games/growing-guns/mods/API.md": "docs/growing-guns-mod-api.html",
}
GAMES_NAV = [
    ("Sideways", "games/sideways/README.md"),
    ("Growing Guns", "games/growing-guns/README.md"),
    ("Growing Guns mod API", "games/growing-guns/mods/API.md"),
]


# ---------------------------------------------------------------- markdown

def slugify(text: str) -> str:
    """GitHub's heading anchors: lowercase, drop punctuation, spaces to -."""
    text = re.sub(r"<[^>]+>", "", text).strip().lower()
    text = re.sub(r"[^\w\- ]", "", text, flags=re.UNICODE)
    return text.replace(" ", "-")


class Inline:
    """Inline markdown to HTML. `link` maps a markdown URL to (href, is_code_path)."""

    def __init__(self, link):
        self.link = link

    def __call__(self, text: str) -> str:
        stash: list[str] = []

        def keep(s: str) -> str:
            stash.append(s)
            return f"\x00{len(stash) - 1}\x00"

        # Code spans first: nothing inside them is markdown.
        def code(m):
            return keep(f"<code>{html.escape(m.group(2).strip() if m.group(2).strip() else m.group(2))}</code>")
        text = re.sub(r"(`+)(.+?)\1", code, text)

        # Links (images become links to the image).
        def link(m):
            label, url = m.group(2), m.group(3).strip()
            url = url.split(' "')[0].strip("<>")
            href = self.link(url)
            inner = self.emphasis(html.escape(label))
            inner = re.sub(r"\x00(\d+)\x00", lambda k: stash[int(k.group(1))], inner)
            if href is None:  # a local path on someone's machine: show, do not link
                return keep(f'<span class="localpath">{inner}</span>')
            ext = ' rel="noopener"' if href.startswith("http") else ""
            return keep(f'<a href="{html.escape(href, quote=True)}"{ext}>{inner}</a>')
        text = re.sub(r"(!?)\[((?:[^\[\]]|\[[^\]]*\])*)\]\(([^)\s]*(?:\s+\"[^\"]*\")?)\)", link, text)

        # Autolinks and bare URLs.
        text = re.sub(r"<(https?://[^>\s]+)>", lambda m: keep(f'<a href="{html.escape(m.group(1), quote=True)}" rel="noopener">{html.escape(m.group(1))}</a>'), text)
        text = re.sub(r"(?<![\w/\"'=])(https?://[^\s<>()]+[^\s<>().,;:!?'\"])",
                      lambda m: keep(f'<a href="{html.escape(m.group(1), quote=True)}" rel="noopener">{html.escape(m.group(1))}</a>'), text)

        text = self.emphasis(html.escape(text, quote=False))
        while "\x00" in text:
            text = re.sub(r"\x00(\d+)\x00", lambda k: stash[int(k.group(1))], text)
        return text

    @staticmethod
    def emphasis(s: str) -> str:
        s = re.sub(r"\*\*(?=\S)(.+?)(?<=\S)\*\*", r"<strong>\1</strong>", s)
        s = re.sub(r"(?<![\w])__(?=\S)(.+?)(?<=\S)__(?![\w])", r"<strong>\1</strong>", s)
        s = re.sub(r"(?<![\w*])\*(?=[^\s*])(.+?)(?<=[^\s*])\*(?![\w*])", r"<em>\1</em>", s)
        s = re.sub(r"(?<![\w])_(?=[^\s_])(.+?)(?<=[^\s_])_(?![\w])", r"<em>\1</em>", s)
        return s


LIST_RE = re.compile(r"^(\s*)([-*+]|\d+[.)])\s+(.*)$")
FENCE_RE = re.compile(r"^(\s*)(```+|~~~+)\s*([\w+-]*)")


@dataclass
class Doc:
    html: str
    title: str
    headings: list[tuple[int, str, str]] = field(default_factory=list)


class Markdown:
    def __init__(self, inline: Inline):
        self.inline = inline
        self.headings: list[tuple[int, str, str]] = []
        self.ids: dict[str, int] = {}

    def convert(self, text: str) -> Doc:
        lines = text.replace("\r\n", "\n").replace("\t", "    ").split("\n")
        body = self.blocks(lines)
        title = next((t for lvl, t, _ in self.headings if lvl == 1), "")
        return Doc(body, re.sub(r"<[^>]+>", "", title), self.headings)

    def heading_id(self, text: str) -> str:
        base = slugify(text) or "section"
        n = self.ids.get(base, 0)
        self.ids[base] = n + 1
        return base if n == 0 else f"{base}-{n}"

    def blocks(self, lines: list[str]) -> str:
        out: list[str] = []
        i = 0
        while i < len(lines):
            line = lines[i]
            if not line.strip():
                i += 1
                continue
            m = FENCE_RE.match(line)
            if m:
                fence, lang = m.group(2), m.group(3)
                indent = len(m.group(1))
                body = []
                i += 1
                while i < len(lines) and not lines[i].strip().startswith(fence):
                    body.append(lines[i][indent:] if lines[i][:indent].strip() == "" else lines[i])
                    i += 1
                i += 1
                cls = f' class="language-{html.escape(lang)}"' if lang else ""
                label = f'<span class="lang">{html.escape(lang)}</span>' if lang else ""
                out.append(f"<pre{cls}>{label}<code>{html.escape(chr(10).join(body))}</code></pre>")
                continue
            m = re.match(r"^(#{1,6})\s+(.*?)\s*#*\s*$", line)
            if m:
                level, raw = len(m.group(1)), m.group(2)
                inner = self.inline(raw)
                hid = self.heading_id(re.sub(r"<[^>]+>", "", inner))
                self.headings.append((level, inner, hid))
                anchor = f'<a class="anchor" href="#{hid}" aria-label="Link to this section">#</a>' if level > 1 else ""
                out.append(f'<h{level} id="{hid}">{inner}{anchor}</h{level}>')
                i += 1
                continue
            if re.match(r"^\s*([-*_])(\s*\1){2,}\s*$", line):
                out.append("<hr>")
                i += 1
                continue
            if line.lstrip().startswith("|") and i + 1 < len(lines) and re.match(r"^\s*\|?\s*:?-{2,}", lines[i + 1]):
                i = self.table(lines, i, out)
                continue
            if line.lstrip().startswith(">"):
                quote = []
                while i < len(lines) and lines[i].lstrip().startswith(">"):
                    quote.append(re.sub(r"^\s*>\s?", "", lines[i]))
                    i += 1
                out.append(f"<blockquote>{self.blocks(quote)}</blockquote>")
                continue
            if LIST_RE.match(line):
                i = self.list(lines, i, out)
                continue
            para = []
            while i < len(lines) and lines[i].strip() and not self.starts_block(lines, i):
                para.append(lines[i].strip())
                i += 1
            out.append(f"<p>{self.inline(' '.join(para))}</p>")
        return "\n".join(out)

    def starts_block(self, lines, i) -> bool:
        line = lines[i]
        return bool(FENCE_RE.match(line) or re.match(r"^#{1,6}\s", line) or LIST_RE.match(line)
                    or line.lstrip().startswith(">")
                    or (line.lstrip().startswith("|") and i + 1 < len(lines) and re.match(r"^\s*\|?\s*:?-{2,}", lines[i + 1])))

    def table(self, lines, i, out) -> int:
        def cells(row: str) -> list[str]:
            row = row.strip()
            if row.startswith("|"):
                row = row[1:]
            if row.endswith("|") and not row.endswith("\\|"):
                row = row[:-1]
            parts, cur, tick = [], "", False
            for ch in row:  # split on | outside code spans
                if ch == "`":
                    tick = not tick
                if ch == "|" and not tick:
                    parts.append(cur)
                    cur = ""
                else:
                    cur += ch
            parts.append(cur)
            return [p.strip().replace("\\|", "|") for p in parts]
        head = cells(lines[i])
        aligns = []
        for spec in cells(lines[i + 1]):
            left, right = spec.startswith(":"), spec.endswith(":")
            aligns.append("center" if left and right else "right" if right else "")
        i += 2
        rows = []
        while i < len(lines) and lines[i].strip().startswith("|"):
            rows.append(cells(lines[i]))
            i += 1

        def td(tag, k, v):
            a = aligns[k] if k < len(aligns) else ""
            style = f' class="{a}"' if a else ""
            return f"<{tag}{style}>{self.inline(v)}</{tag}>"
        thead = "".join(td("th", k, v) for k, v in enumerate(head))
        tbody = "".join("<tr>" + "".join(td("td", k, v) for k, v in enumerate(r)) + "</tr>" for r in rows)
        out.append(f'<div class="table"><table><thead><tr>{thead}</tr></thead><tbody>{tbody}</tbody></table></div>')
        return i

    def list(self, lines, i, out) -> int:
        first = LIST_RE.match(lines[i])
        base = len(first.group(1))
        ordered = first.group(2)[0].isdigit()
        start = int(re.match(r"\d+", first.group(2)).group(0)) if ordered else 1
        items: list[list[str]] = []
        loose = False
        while i < len(lines):
            m = LIST_RE.match(lines[i])
            if m and len(m.group(1)) == base and (m.group(2)[0].isdigit()) == ordered:
                content_indent = len(m.group(0)) - len(m.group(3))
                item = [m.group(3)]
                i += 1
                while i < len(lines):
                    nxt = lines[i]
                    if not nxt.strip():
                        # A blank line ends the list unless the next text is indented into the item
                        # or is another item of this list.
                        j = i + 1
                        while j < len(lines) and not lines[j].strip():
                            j += 1
                        if j < len(lines):
                            lm = LIST_RE.match(lines[j])
                            ind = len(lines[j]) - len(lines[j].lstrip())
                            if ind >= content_indent or (lm and len(lm.group(1)) == base):
                                loose = True
                                item.append("")
                                i += 1
                                continue
                        break
                    ind = len(nxt) - len(nxt.lstrip())
                    lm = LIST_RE.match(nxt)
                    if lm and len(lm.group(1)) <= base:
                        break
                    if ind < content_indent and not lm and self.starts_block(lines, i):
                        break
                    item.append(nxt[content_indent:] if ind >= content_indent else nxt.strip())
                    i += 1
                items.append(item)
            else:
                break
        tag = "ol" if ordered else "ul"
        attr = f' start="{start}"' if ordered and start != 1 else ""
        parts = []
        for item in items:
            while item and not item[-1].strip():
                item.pop()
            has_blocks = any(LIST_RE.match(x) or FENCE_RE.match(x) or x.lstrip().startswith("|") for x in item[1:])
            if loose or has_blocks:
                # Split the leading paragraph from nested blocks.
                inner = self.blocks(item)
                if not loose:
                    inner = re.sub(r"^<p>(.*?)</p>", r"\1", inner, count=1, flags=re.S)
            else:
                inner = self.inline(" ".join(x.strip() for x in item))
            parts.append(f"<li>{inner}</li>")
        out.append(f"<{tag}{attr}>{''.join(parts)}</{tag}>")
        return i


# ---------------------------------------------------------------- site

@dataclass
class Page:
    src: str          # repo path, posix
    out: str          # site path, posix
    title: str = ""
    body: str = ""
    headings: list = field(default_factory=list)


def page_map() -> dict[str, str]:
    pages = {}
    for p in sorted((ROOT / "engine/docs").glob("*.md")):
        rel = p.relative_to(ROOT).as_posix()
        pages[rel] = "docs/index.html" if p.name == "README.md" else f"docs/{p.stem}.html"
    for p in sorted((ROOT / "engine/docs/log").glob("*.md")):
        pages[p.relative_to(ROOT).as_posix()] = f"docs/log/{'index' if p.name == 'README.md' else p.stem}.html"
    for src, out in EXTRA_PAGES.items():
        if (ROOT / src).exists():
            pages[src] = out
    return pages


class Linker:
    def __init__(self, pages: dict[str, str], ref: str):
        self.pages = pages
        self.ref = ref
        self.broken: list[tuple[str, str]] = []

    def for_page(self, src: str, out: str):
        def link(url: str):
            if not url:
                return url
            if re.match(r"^[a-z]+:", url) and not re.match(r"^[A-Za-z]:[/\\]", url):
                return url  # http:, https:, mailto:
            if url.startswith("#"):
                return url
            if re.match(r"^[A-Za-z]:[/\\]", url) or url.startswith(("/", "\\")):
                return None  # an absolute path on a developer machine
            path, _, frag = url.partition("#")
            target = posixpath.normpath(posixpath.join(posixpath.dirname(src), path))
            if target in self.pages:
                href = posixpath.relpath(self.pages[target], posixpath.dirname(out))
                return href + (f"#{frag}" if frag else "")
            if target.startswith("..") or not (ROOT / target).exists():
                self.broken.append((src, url))
                return None  # not in the repository (a build output, a moved file): show the path
            kind = "tree" if (ROOT / target).is_dir() else "blob"
            return f"{REPO_URL}/{kind}/{self.ref}/{target}" + (f"#{frag}" if frag else "")
        return link


def read_nav(index_md: str, log_md: str) -> tuple[list[tuple[str, str]], list[tuple[str, list[tuple[str, str]]]]]:
    """Guides (label, repo path) from the docs index, note groups from the log index."""
    guides, groups = [], []
    section = ""
    for line in index_md.splitlines():
        if line.startswith("## "):
            section = line[3:].strip().lower()
            continue
        if section == "guides" and line.startswith("|"):
            m = re.search(r"\[([^\]]+)\]\(([^)]+)\)", line.split("|")[1])
            if m:
                guides.append((m.group(1), posixpath.normpath(posixpath.join("engine/docs", m.group(2)))))
    for line in log_md.splitlines():
        if line.startswith("## "):
            groups.append((line[3:].strip(), []))
            continue
        m = re.match(r"^- \[([^\]]+)\]\(([^)#]+)", line)
        if m and groups:
            target = posixpath.normpath(posixpath.join("engine/docs/log", m.group(2)))
            if target.startswith("engine/docs/log/"):
                groups[-1][1].append((m.group(1), target))
    return guides, [g for g in groups if g[1]]


def render_nav(current: Page, pages, guides, groups) -> str:
    def a(label, src):
        if src not in pages:
            return ""
        href = posixpath.relpath(pages[src], posixpath.dirname(current.out))
        cur = ' aria-current="page"' if src == current.src else ""
        return f'<li><a href="{href}"{cur}>{html.escape(label)}</a></li>'
    home = posixpath.relpath("docs/index.html", posixpath.dirname(current.out))
    parts = [f'<p class="nav-title"><a href="{home}"{" aria-current=page" if current.src == INDEX else ""}>Documentation</a></p>',
             '<h2>Guides</h2><ul>' + "".join(a(l, s) for l, s in guides) + "</ul>",
             '<h2>Games</h2><ul>' + "".join(a(l, s) for l, s in GAMES_NAV) + "</ul>"]
    in_notes = any(current.src == s for _, items in groups for _, s in items)
    notes = []
    for name, items in groups:
        open_ = " open" if any(current.src == s for _, s in items) else ""
        notes.append(f"<details{open_}><summary>{html.escape(name)}</summary><ul>" + "".join(a(l, s) for l, s in items) + "</ul></details>")
    log = posixpath.relpath(pages[LOG_INDEX], posixpath.dirname(current.out)) if LOG_INDEX in pages else "#"
    parts.append(f'<h2><a href="{log}">Development log</a></h2><div class="notes{" here" if in_notes else ""}">' + "".join(notes) + "</div>")
    return "\n".join(parts)


def toc(page: Page) -> str:
    items = [(lvl, t, hid) for lvl, t, hid in page.headings if lvl in (2, 3)]
    if len(items) < 3:
        return ""
    lis = "".join(f'<li class="l{lvl}"><a href="#{hid}">{re.sub(r"<a class=.anchor.*?</a>", "", t)}</a></li>' for lvl, t, hid in items)
    return f'<nav class="toc" aria-label="On this page"><p>On this page</p><ul>{lis}</ul></nav>'


def fill(template: str, **values: str) -> str:
    return re.sub(r"\{\{(\w+)\}\}", lambda m: values.get(m.group(1), m.group(0)), template)


def build(out: Path, ref: str, strict: bool) -> int:
    pages = page_map()
    linker = Linker(pages, ref)
    index_md = (ROOT / INDEX).read_text(encoding="utf-8")
    guides, groups = read_nav(index_md, (ROOT / LOG_INDEX).read_text(encoding="utf-8"))

    rendered: list[Page] = []
    for src, dst in pages.items():
        md = Markdown(Inline(linker.for_page(src, dst)))
        doc = md.convert((ROOT / src).read_text(encoding="utf-8"))
        rendered.append(Page(src, dst, doc.title or Path(src).stem, doc.html, doc.headings))

    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    page_tpl = (SITE_SRC / "templates/page.html").read_text(encoding="utf-8")
    for p in rendered:
        root = posixpath.relpath(".", posixpath.dirname(p.out))
        kind = "note" if p.src.startswith("engine/docs/log/") and p.src != LOG_INDEX else "guide"
        banner = ""
        if kind == "note":
            notes = posixpath.relpath(pages[LOG_INDEX], posixpath.dirname(p.out))
            banner = (f'<p class="note-banner">A <a href="{notes}">development log</a> entry, kept as it was written. '
                      "Numbers in it belong to the build and scene it names; paths on a developer's machine are shown, not linked.</p>")
        html_ = fill(page_tpl,
                     title=html.escape(p.title), root=root, nav=render_nav(p, pages, guides, groups),
                     body=p.body, toc=toc(p), kind=kind, banner=banner,
                     source=f"{REPO_URL}/blob/{ref}/{p.src}", source_path=html.escape(p.src))
        dest = out / p.out
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(html_, encoding="utf-8")

    # Landing page and static files.
    shutil.copytree(SITE_SRC / "static", out, dirs_exist_ok=True)
    landing = (SITE_SRC / "templates/index.html").read_text(encoding="utf-8")
    (out / "index.html").write_text(fill(landing, root=".", repo=REPO_URL), encoding="utf-8")
    shutil.copytree(SITE_SRC / "assets", out / "img", dirs_exist_ok=True)
    golden = out / "img/golden"
    golden.mkdir(parents=True, exist_ok=True)
    for png in sorted((ROOT / "games/growing-guns/scripts/golden").glob("*.png")):
        shutil.copy2(png, golden / png.name)
    fonts = out / "fonts"
    fonts.mkdir(exist_ok=True)
    for name in ["BarlowCondensed-ExtraBoldItalic.ttf", "BarlowCondensed-SemiBold.ttf", "BarlowCondensed-Medium.ttf", "OFL-BarlowCondensed.txt"]:
        shutil.copy2(ROOT / "engine/assets/fonts" / name, fonts / name)
    (out / ".nojekyll").write_text("", encoding="utf-8")

    print(f"wrote {len(rendered) + 1} pages to {out}")
    for src, url in linker.broken:
        print(f"warning: {src}: link to missing {url}", file=sys.stderr)
    return 1 if strict and linker.broken else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--out", type=Path, default=SITE_SRC / "_site", help="output directory (replaced)")
    ap.add_argument("--ref", default="main", help="git ref that links to code on GitHub point at")
    ap.add_argument("--strict", action="store_true", help="fail when a relative link points at nothing")
    a = ap.parse_args()
    return build(a.out.resolve(), a.ref, a.strict)


if __name__ == "__main__":
    sys.exit(main())
