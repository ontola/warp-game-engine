# Warp website

A static site: a landing page plus the repository's markdown docs rendered
as pages. The markdown stays the single source; this folder only holds the
build script, two templates, a stylesheet and one picture.

```sh
python3 website/build.py                      # writes website/_site
python3 -m http.server -d website/_site 8000  # preview at http://localhost:8000
```

Python 3.9 or newer, standard library only: nothing to install.

## What the build does

`build.py` reads every `engine/docs/*.md` plus a few other READMEs and notes
(listed in `EXTRA_PAGES`), converts them with a small built-in markdown
converter, and writes `_site/docs/*.html`.

- Navigation comes from `engine/docs/README.md`: the Guides table and the
  grouped lists under Engineering notes. Add a guide there and it appears
  in the sidebar.
- Links between rendered files become links between pages, anchors
  included. Links to code and other repository files point at GitHub;
  `--ref` picks the branch or tag (default `main`).
- Links to paths that are not in the repository (absolute paths on a
  developer machine, build outputs) are shown as paths, not links, and
  reported as warnings. `--strict` turns warnings into a failure.
- Raw HTML in markdown is escaped, not passed through.

Other inputs, copied as they are:

- `templates/index.html`: the landing page; `templates/page.html`: every
  docs page. Placeholders look like `{{root}}`.
- `static/`: `style.css` and the favicon.
- `assets/hero/`: the four frames of the landing page's split-screen
  picture, cropped to 16:9 from `sideways-stage.jpg`, two Growing Guns
  golden images and `engine/docs/images/volumetric-night-arena.jpg`.
  To be replaced by a real four-view capture.
- `static/logo.svg`: the logo. A big white W in front of a coarse grid
  whose horizontal lines bend around a warp bubble: a rise behind and a
  dip ahead, each peak exactly on a vertical grid line. The yellow grid
  line runs through the middle of the W. Grid lines are one solid grey
  (#5e6772), not translucent white, so crossings do not add up. Deep-space blue.
  `static/favicon.svg` is the same file (also used next to the wordmark);
  the grid is coarse enough to read at 16 pixels.
- `assets/sideways-stage.jpg`: a Sideways frame rendered headless on a CPU
  rasteriser (`KERN_ADAPTER=software sideways shot out.png --size 960x540
  --frames 900 --taa 4`, llvmpipe, 28 October 2026), converted to JPEG.
- From the repository: Growing Guns' golden images
  (`games/growing-guns/scripts/golden/*.png`) and the Barlow Condensed
  fonts with their license (`engine/assets/fonts`).

## Why a script and not a site generator

The docs use a small part of markdown (headings, lists, tables, fenced
code, links), and the site needs one thing a generic generator does not
do by default: rewrite links between files across the repository and to
code on GitHub. A few hundred lines of standard-library Python do that
with nothing to pin or install, in CI or on a laptop.

## Publishing

`.github/workflows/pages.yml` builds the site on pull requests that touch
docs or the website (a check only) and on pushes to `main`. Deploying to
GitHub Pages runs only when the repository variable `PAGES_ENABLED` is
`true` and Pages is set to "GitHub Actions" in the repository settings.
Neither is switched on by this folder.
