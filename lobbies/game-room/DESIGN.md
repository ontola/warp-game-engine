# Splitscreen Club UI design guide

How Splitscreen Club's own screens look: the browser host (`/play/`), the
Warp Game Room (room view, catalog view, menus, badges, toasts) and the
HTML around it (loading screen, notices, hints, crash screen). The website
(`gamepad-party/web`) and the Flutter app (`gamepad-party/mobile`) already
share most of these colours; they follow this guide where it says so.

Read this before adding or changing anything drawn over the room or the
games. The last section lists where the current UI does not follow it yet.

Two layers, two rules:

- **Club chrome**: everything Splitscreen Club draws (lobby HUD, catalog,
  page overlays, Back panel). One look,
  defined here.
- **Game UI**: what a game draws inside its own world (Spellstorm's
  parchment menus, Sideways' rally HUD). Games keep their own art style, but
  follow the *structure* rules (start menu contents, controller glyphs,
  telegraphing) in [Rules for games](#rules-for-games).

Sizes are given at the **1080p reference**. The HUD multiplies by
`scale = height / 1080` (as `hud.rs` and `catalog.rs` do); HTML uses the
same numbers in px with `clamp()` where the text must grow with the
screen.

## Colour tokens

One palette for HTML, HUD and app. The HUD names are the Rust constants
in `src/room.rs` / `src/lobby.rs`; the CSS names go in `:root` of
`examples/browser/index.html` (and `gamepad-party/web/site.css`).

| Token | Value | CSS | HUD | Use |
|---|---|---|---|---|
| bg | `#0d1020` | `--bg` | `BG` (now `#0b0b12`, see below) | Page and full-screen backgrounds, ink on bright chips |
| panel | `#0d1020` at 88 % | `--panel` | `PANEL` | Every floating panel, card and banner |
| panel-solid | `#0d1020` opaque (HUD), 95 % in CSS | `--panel-solid` | `PANEL_SOLID` | Menus (must hide what is under them) |
| line | white at 10 % | `--line` | `LINE` | 1 px panel border |
| text | `#eef0fc` | `--text` | `TEXT` | Body text, selected menu item |
| muted | `#a8adc4` | `--muted` | `DIM` | Secondary text, unselected items, counts |
| amber | `#ffb454` | `--amber` | `AMBER` | Brand ("Splitscreen"), kickers/eyebrows, warnings, loading |
| lilac | `#a78bfa` | `--lilac` | `NIGHT_LILAC` | Brand (".Club"), links, focus ring |
| violet | `#7c5cff` | `--violet` | `VIOLET` | Selection fill (at 35 %), progress end |
| violet-deep | `#5143ae` | `--violet-deep` | `VIOLET_DEEP` | Filled primary buttons with white text (app, HTML) |
| green | `#3ddc97` | `--green` | `GREEN` | Ready, joined, queued ("Up next"), success |
| red | `#e2574c` | `--red` | `RED` | Errors, leaving, destructive confirm |

Rules:

- Ink on a bright chip, disc or key is `bg` (`#0d1020`), never pure black
  and never one of the near-blacks (`#0b0b12`, `#0c0a16`) scattered now.
- Amber is the club's *voice* (kickers, "loading", warnings). Violet is
  *interaction* (what is selected, what you press). Green is *state*
  (ready, queued). Don't use them for other things.
- No other greys or near-blacks in chrome. A new colour needs a row here.
- The website and app keep their `--accent #7565ee` and button `#5143ae`
  for now; they converge on `violet` when they are next restyled.

## Controller and key glyphs

A button on screen is always drawn as a **glyph**, never as a word in
the sentence ("Press A", "press Enter"). One look everywhere: HTML hint,
HUD badge, catalog button, Back panel.

| Button | Glyph | Fill | Ink |
|---|---|---|---|
| A | disc | green `#3ddc97` | bg |
| B | disc | red `#e2574c` | bg |
| X | disc | blue `#4a8fe7` | bg |
| Y | disc | yellow `#f2c14e` | bg |
| LB / RB / LT / RT | rounded bumper (`paint::bumper`) | `#30303f`, 1.5 px `#b4b2c8` edge | text |
| Start / Back | pill | `#30303f`, 1.5 px `#b4b2c8` edge | text |
| Keyboard key | keycap (`paint::keycap`) | `#f2f1f8` key on a `bg` shadow | bg |
| Left / right stick | stick (see below) | `#30303f`, 1.5 px `#b4b2c8` edge, inner cap ring | text |
| D-pad | plus shape, the used arm lit in `text` | `#30303f` | |

- **Sticks** are the hardest to read, so they follow one drawing: a top
  view of the stick, a `#30303f` disc with a 1.5 px `#b4b2c8` edge and a
  second ring at 62 % of its radius (the cap), the letter **L** or **R** in
  the middle. Moving it: a small triangle on the side it moves to (all
  four for "any direction"). Clicking it (L3/R3): a short bar under the
  disc with a down chevron, and the label says "Press". Never write "L3",
  "R3" or "LS" as text on screen.
- Size: the glyph is as tall as the line it sits in (1.4 × the text size);
  the label next to it is Outfit 800.
- Keyboard keys are **keycaps** (light key, dark text), in HTML too: not
  the violet `<kbd>` chip. CSS for it is in [Components](#components).
- A disc keeps its colour when disabled; only its alpha drops to 35 %.
- Show the glyph of the device the player uses (`lobby.button_name`,
  `on_keyboard`): a keyboard player sees keycaps, a pad player discs.
  Nintendo/PlayStation glyphs: later, through the same function.

## Type

Outfit for all UI text, Bungee only for the wordmark
(same rule on the website and in the app). Only two weights are shipped: **600** (regular) and
**800** (bold). Never ask for 700: browsers then fake or snap it.

| Role | Size @1080p | Weight | Case | Examples |
|---|---|---|---|---|
| Display | 44 | 800 | Title Case (game titles as written) | Catalog game title, crash title |
| Screen title | 34 | 800 | UPPERCASE | "ALL GAMES" |
| Panel title | 22 | 800 | Sentence case | "Connect a controller and press A" |
| Body | 19–20 | 600 | Sentence case | Menu items, toasts, notice text, queue rows |
| Small | 15–16 | 600 | Sentence case | Hints under a title, footers, "+6 more" |
| Kicker | 0.7 × the text below it, +0.04 em tracking | 800 | UPPERCASE | "UP NEXT", "KEYBOARD & MOUSE", "BACK · JOEP" |

- Kickers are amber. The one exception is "UP NEXT" and "ON PHONES",
  which are green because they name a state.
- Text over the 3D room (names over heads) gets a `bg` outline (5 px at
  1080p, `text_outline`); text on a panel never does.
- Wordmark: "Splitscreen" amber, ".Club" lilac, Bungee, nothing else in
  Bungee.
- Copy: sentence case, no full stop at the end of a one-line hint, a
  middle dot ` · ` between two short phrases. No "please".

## Spacing, radii, elevation

Spacing steps (px @1080p): **4, 8, 12, 16, 24, 32, 48**. Nothing in
between.

- Screen edge margin: **32** (corners: catalog title/count, Up next,
  bench, update banner). Top notices sit **28** from the top.
- Panel padding: **16 × 24** (vertical × horizontal); between a glyph and
  its label: **12**; between stacked panels: **12**.

Radii:

| Thing | Radius |
|---|---|
| Key cap, small chip | 8 |
| Cover card, catalog selection frame | 12 |
| Panel, notice, toast, menu, Back panel, Up next | 18 |
| Button (HTML, app) | 12 |
| Pill, hover pill, hint row | full (height / 2) |

Elevation: panels get one shadow, `0 12px 36px rgba(0,0,0,.45)` (HUD: a
`style_box` 8 px lower at 45 % black, as the catalog cards already do).
Backdrop blur (8 px) where the platform has it (HTML); the HUD has none,
which is why `panel` is 88 % opaque.

## Components

### Panel

The one container. Notices, toasts, menus, Back panel, Up next, bench,
banners, the loading card: all of them are a panel.

```css
.panel{background:rgba(13,16,32,.88);border:1px solid rgba(255,255,255,.10);border-radius:18px;
  box-shadow:0 12px 36px rgba(0,0,0,.45);backdrop-filter:blur(8px);padding:16px 24px}
```

HUD: `c.style_box(r, [18.0 * scale; 4], PANEL, Some((1.0, LINE)))`, a
helper `panel(c, r, scale)` in `paint.rs` so nobody draws their own.

- An accent goes in the **kicker**, not in the frame: no coloured borders,
  no left accent bars (toasts, bench, error banner do that now).
  Exception: the error banner may use a 1 px red border.
- Menus use `panel-solid` so the room, the TV and the Up next cabinet do
  not show through.

### Menu (Start menu, mouse menu, Back panel)

- Panel-solid, radius 18, kicker on top (amber, uppercase: the player's
  name, "KEYBOARD & MOUSE", "BACK · NAME").
- Rows: Body size, 12 px vertical padding. Selected row: violet at 35 %,
  radius 12, text `text`; other rows `muted`.
- Confirm state ("Press A again to quit"): the row's label changes and the
  glyph of the button is shown in it; text amber.
- No hint footer in a Start menu (Joep's rule for game menus applies to
  the lobby's too). "Esc closes" goes away: Esc closing a menu is assumed.

### Hint (notice)

What a player must do next. Top centre, one at a time.

- A panel with: optional icon (44 px) or glyph, a panel title, and
  optionally one muted line.
- Glyph first, then the verb: `[A] Join this game`, `[Enter] Play with
  the keyboard`, `[Back] Back to the room`.
- The empty-room notice ("Connect a controller") and the key/join/back
  hints are the **same component** at the same place and size. If two
  are due, the newest replaces the old (crossfade), they never stack.

### Toast

Starting a game gives no toast: its progress bar already shows on its card,
the TV and Up next.

Something happened ("Joep joined", "Update ready"). Bottom centre in the
room, top centre in the catalog view (so it doesn't cover the buttons),
stacked 12 apart, newest nearest the edge. Panel, Body text, an optional
coloured dot (8 px) for its colour: no left bar.

### Controller action (catalog, TV screens)

An action a pad triggers is **the glyph and its label, nothing around
them** (Joep, 2026-11-25: the coloured pills were too loud). Only the
glyph is coloured; the label is Outfit 800 in `text`, the pair centred in
its slot, 54 px tall @1080p (5 % of the screen), gap 0.25 × the height.
Disabled: glyph at 35 %, label at 40 %. A mouse over it gets a faint
pill (white 8 %, radius full) so it reads as clickable; no border.

### Button (HTML, app)

Things you click or tap are buttons: radius 12, `violet-deep` fill and
white text for the primary action, white at 10 % for the others, no
border. The crash screen, the loading page links and the app use these.

### Cover card

3:4, radius 12. **Cover only in every store grid** (app, website Artwork
view, catalog): the art carries the title; tagline, player count and
price live on the game page. Without a cover: the game's colour with its
title. The website's Gameplay view shows screenshots, so it keeps a title.
 Unselected 72 % brightness; hover 90 %; selected 100 % and
a 5 px `text` frame at radius 12 (catalog). Tag on a cover, top right: a
quiet dark pill (solid panel 78 %, radius 8) with the queue place as in the
playlist ("02") in `text`, or an amber play/pause sign for the game on now.
No loading spinner: the progress bar along the bottom of the cover says it.

### Playlist (Up next)

The queue looks the same in the room (TV cabinet and the list under
it), the catalog view and the app (Playlist card), only bigger or
smaller (Joep, 2026-11-25: "the playlist looks consistent throughout").

- Kicker **UP NEXT** in green. When the game on now is shown with it, it
  comes first under an amber **ON NOW** kicker.
- One row per game: two-digit number in `muted` (01 is the next game,
  the same number as the green badge on its cover in the catalog), a
  small 3:4 cover, the title in `text`.
- The first row may be larger (the TV cabinet's big cover); the numbers
  and order do not change with it.
- Loading: the percentage after the title in amber, an amber bar under
  the first row. Problems (cannot start): the status line in red.
- Where the queue came from (Queued, Autoplay) is a small `muted` line
  under the title, never a colour.
- Too many rows: **+N more** in `muted`, one row's height.
- Rows of the same size in all three; the panel is the standard panel.

### Progress

One bar everywhere (HTML loader, cards, TV): 10 px track (HUD: about a
quarter of the label's height), white at 8 %, fill solid amber. Radius
full. A step line in `muted` under it (Joep, 2026-11-25: the card's bar
and the loader's differed). HUD: `paint::progress`.

## Layout zones

The screen has fixed places so overlays never collide. Every overlay
belongs to one zone; a zone holds one thing at a time (or a stack of the
same kind).

| Zone | Room view | Catalog view |
|---|---|---|
| Top centre | Hints / empty-room notice | Toasts |
| Top left | (free: the room) | Screen title "ALL GAMES" |
| Top right | Update banner | "n of N", update banner under it |
| Middle | The room; menus over heads; mouse menu at 62 % height | Cover grid |
| Bottom centre | Toasts | Title, buttons |
| Bottom left | Bench (no controller / on phones) | Player menus (corner order) |
| Bottom right | Fullscreen button (windowed only) | Up next |

- Bottom centre in the catalog is **reserved** for the title and buttons
  over the full width of the buttons plus 32 px each side. Up next moves
  up (above the buttons' top) or shrinks to a one-line "Up next: Title
  +6" when the screen is narrower than 16:9.
- Anything in a zone is laid out from the zone's edge inward with the
  margin above; nothing is placed with a magic `h * 0.66`.

## Motion

| What | How |
|---|---|
| Panel in | opacity 0 → 1 and 12 px slide from its edge, 350 ms ease-out |
| Panel out | opacity to 0, 250 ms ease-in, no slide |
| Hint on screen | 4 s (7 s the very first time), then out |
| View change (room ↔ catalog) | 300 ms crossfade (`catalog_fade`) |
| Pressed glyph | squash 18 %, 200 ms back (`badge`) |
| Attention ("you pressed it, look here") | the `nod`: 0.9 s, ±4 px, text to lilac |

Nothing appears or disappears in one frame, except the loading screen's
first paint. No bouncing panels; the only idle animation in chrome is the
loading icon's `bob`.

## Rules for games

From Joep's decisions, for every game's own UI:

- Start opens the game's menu: game title visible, **no button hints**,
  Resume first, then Restart/New round/Apply, then "Back to lobby" (Back
  alone also goes to the lobby). Controller mapping is in the start menu
  or one item away.
- A game's own HUD keeps its own style; it does not need the club look
  (Joep, 2026-11-25). Glyph colours from
  [Controller and key glyphs](#controller-and-key-glyphs) are a
  suggestion there, not a rule.
- Enemy attacks are always telegraphed.
- Never freeze on load or on a new player: load in the background.
- A game's start menu is the game's own, including the engine's built-in
  one for Luau games (`warp/src/game.rs`): it fits the game's brand, not
  the club's (Joep, 2026-11-25). Only the structure rules above apply.

## Where the current UI differs

Found on 2026-11-25 from the code and Joep's screenshots of the browser
host. Numbers are referenced from the thread.

**Status (2026-11-25, same day):** fixed 1–9, 11–13, 15–19, 21 (store grids
are cover only), 22 (room TV, list, catalog and app share the Playlist
component; the TV keeps its big first cover, numbered 01) and 23. Left:
14 (dev tool). 10 and 20 are out of scope: games' own start menus and
HUDs keep their own style (Joep, 2026-11-25).

1. **Panels have six looks.** HTML notices: rounded 18–20, 1 px border,
   blur. Mouse menu: square, 3 px violet border. Toasts, bench, error
   banner: square with a 4–5 px left bar. Back panel: radius 0.6 em, 2 px
   grey border. Up next: radius 0.5 em, no border. Badges: pill. →
   one `panel()` helper in `paint.rs`, used by all (`hud.rs` `menu`,
   toasts, `bench`, `back_panel`, error banner; `catalog.rs` queue).
2. **Keyboard keys have two looks.** HTML hints use a violet `<kbd>` chip
   (`#back-hint kbd`, "Enter"), the HUD draws a white keycap
   (`paint::keycap`), the fullscreen button a grey chip. → keycap
   everywhere.
3. **Gamepad buttons are written as words** in the empty-room notice
   ("Connect a controller and press A", "press Enter on the keyboard") and
   the join hint, while the catalog and badges show discs. → glyphs.
4. **Two near-identical notices at the top** (empty-room notice and key
   hint) with different sizes and layouts; the key hint is hidden while
   the notice shows. → one hint component (above).
5. **Up next overlaps** the catalog buttons on screens narrower than 16:9
   (its place is `h - qh - 72`, the buttons are centred, nothing reserves
   the space). → layout zones.
6. **The room shows through the mouse menu**: the TV's "UP NEXT / WARP
   THEFT AUTO" is readable through "Show all games" in Joep's screenshot.
   Cause (checked 2026-11-25): the HUD blends in linear light, so a 95 %
   panel lets 5 % *linear* through, which a white TV text turns into
   about 25 % *sRGB* grey. Fixed: `PANEL_SOLID` (and the engine start
   menu's panel) is opaque in the HUD. Translucent HUD panels (`panel`,
   88 %) are fine over the dim room, never over the TV.
7. **Mouse menu has a hint footer** ("Esc closes · Controllers use their
   own Start menu"), against the no-hints rule.
8. **Three violets.** Lobby `VIOLET #7c5cff`, HTML accent `#a78bfa`
   (used as the kbd fill and button fill), web/app `#7565ee` / `#5143ae`.
   → violet for interaction, lilac for brand/links (tokens above).
9. **Three backgrounds and two text whites.** `#0d1020` (HTML, web, app),
   `#0b0b12` (lobby `BG`), `rgba(.03,.03,.06)` (HUD panels, ≈ `#08080f`),
   plus `#0c0a16`/`#0b0b12` as ink; text `#eef0fc` vs `#f2f1f8`, muted
   `#a8adc4` vs `#9a98ad`. → tokens above.
10. **Engine start menu (Luau games) uses Barlow Condensed**, a white 16 %
    highlight and yellow `#ffd65a` selected text: a different menu from
    the lobby's. → Outfit and the menu component.
11. **Kicker colours vary**: menu headings amber, "UP NEXT" green, Back
    panel title white, "ALL GAMES" white and big. → kicker rule above.
12. **Weight 700 is used in HTML** (`#web-game-links a`, `#fullscreen`)
    but only 600/800 are loaded.
13. **Selection looks differ**: menu row violet 35 % square, catalog card
    white frame, engine menu white 16 % with yellow text, website buttons
    `#5143ae`. → violet fill for rows, white frame for cards.
14. **The story editor** (`stories/editor.css`, F2) has its own teal
    palette and 4 px radii. It is a dev tool; leave it, or move it to the
    tokens when it ships to players.

### The app (Flutter, `gamepad-party/mobile`)

Checked 2026-11-25 on the live web build (splitscreen.club/app, phone
390 px and desktop 1280 px: Room, Games, game page, You, Host a room,
More). The app is the most consistent surface: same `bg`, Outfit, the
wordmark, one card style. Where it differs from this guide:

15. **Primary violet is darker.** Buttons and the selected tab use
    `#5143ae`, `accent` is `#7565ee`; the TV side selects with `#7c5cff`.
    → `accent` to `violet`. Filled buttons with white text keep the
    darker `#5143ae` (white on `#7c5cff` is only 4.4 : 1); name it
    `violet-deep` here and use it for filled buttons in HTML too.
16. **The More sheet is Material grey** (the default bottom sheet colour
    from the seed), not the navy card colour: it is the one screen that
    looks like another app. → `bottomSheetTheme` with `GnColors.card`,
    top radius 18, the `line` edge.
17. **Nine radii** (4, 6, 8, 9, 10, 12, 14, 16, 18, 20). → 8 for chips,
    12 for buttons, inputs and cover cards, 18 for cards and sheets.
18. **One card has a coloured border** ("Save your player" on You is
    framed in violet, every other card in `#343b53`). → same edge as the
    others; the emphasis goes in its title or its button.
19. **Card edges are `#343b53`**, the TV side uses white at 10 %. Both
    read the same on `bg`; the app keeps its opaque colour (no blur on
    phones), CSS and HUD use `line`.
20. **Game text in store screenshots spells buttons as words**
    ("A / Space · Plant bomb", Bombing Bugs). That is the game's own HUD:
    see [Rules for games](#rules-for-games), glyphs.
21. **Game card blurbs cut mid-word** ("dash tackl…") on the phone grid.
    → two lines, cut at a word, or the catalog's short tagline.
22. **The playlist has three looks.** Room TV: big cover for the first
    game, rows numbered from 02, "+N more in queue" in green. Catalog: rows
    numbered from 01, no covers, "+6 more" in muted. App: covers, no
    numbers, "UP NEXT" in muted, status words as subtitles. → the
    Playlist component above.
23. **Sticks are drawn as text** ("R3 All games" at the shelf). → the
    stick glyph.
