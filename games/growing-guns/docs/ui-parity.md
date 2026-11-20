# UI parity: Godot original and the Rust port

Every UI element of the Godot Growing Guns (`/home/claude/growing-guns`, the
scenes and `game.gd`, `render_player.gd`, `start_screen.gd`,
`splitscreen_manager.gd`, `menu_helpers.gd`, `ui_style.gd`,
`gothic_menu_ui.gd`, `card_face.gd`, `crosshair.gd`, `hit_marker.gd`,
`damage_indicator.gd`, `player.gd`) and its state in the port
(`app/src/hud/`, `play.rs`, the engine's `render::hud` and `app`).

Status: **done** (same layout, colours, fonts and timing, as far as the
engine draws them), **close** (present, with a stated difference),
**missing** (with the reason).

Units: Godot draws on a 1281 x 720 canvas stretched to the window; the port
draws in pixels with `u = height / 720` per design unit, and per-view sizes
use Godot's layout scale `clamp(min(w / 960, h / 540), 0.58, 1)`.

Fonts: Anton (poster numbers, titles, banners), Barlow Condensed ExtraBold
(buttons, card bands, prompts) and SemiBold (labels), as `ui_style.gd`;
all three ship under the OFL in `assets/fonts`. The logo is the cover-art
picture `assets/branding/logo.png`.

## Checking it

Headless, on the software rasteriser (never a window):

```
KERN_ADAPTER=software target/debug/growing-guns script games/growing-guns/scripts/ui.json --out DIR
```

| Script | Run with | Shows |
|---|---|---|
| `ui.json` | | HUD, firing, Tab scoreboard, pause, settings and a slider, card flip, pick, focus, picked |
| `ui-match.json` | `--rounds 1` | hit marker and damage arcs, match-over pop and buttons, start screen and its settings |
| `ui-split.json` | `--players 2`, `--players 4` | split-screen HUDs and a card pick in one view |
| `ui-names.json` | `--players 2` | party names, name tags, network error panel, Splitscreen Club pause, ADVANCED, settings with per-player aim |
| `ui-phoenix.json` | | explosion flash, phoenix white rising and fading |
| `ui-text.json` | | join field typing, editing, scrolling and paste (pause and start screen), Player name, the pad's on-screen keyboard, the name in the scoreboard |
| `ui-hotjoin.json` | `--players 1` | splitscreen hint, SPLITSCREEN, PRESS X TO JOIN, pads joining over bots, PLAYER MENU, LEAVE, splitscreen off |
| `golden.json` | `tools/golden.sh` | the pixel references (HUD included) |

Debug commands for scripts: `menu start|pause|settings|club|advanced|none`,
`focus join|name`, `split on|off`, `leave K`, `copy`, `names A,B`,
`net-error TEXT|none` (and the gameplay ones, `help`). Script steps can
type (`text`, `keys`, `clipboard`) and drive virtual pads (`pads`, `device`);
see `engine/crates/app/src/script.rs`. `nettest` joins through the pause
menu's field (the ticket pasted in) and checks that COPY puts the ticket on
the clipboard (the in-process one).

## In-game HUD (per view, `render_player.gd`)

| Element | Godot | Shows, when | Rust |
|---|---|---|---|
| Health plate | `_build_hp_panel` | heart + health, slanted plate with red edge, bottom row at x -290 | done; GHOST while a ghost |
| Ammo plate | `_build_value_panel` | bullet icon, LMB/RT prompt, `ammo / mag`; reload bar while reloading | done |
| Special plate | `_build_value_panel` | grenade icon, RMB/LT prompt, special name (AIR STRIKE xN with charges); cooldown bar | done |
| Dash plate | `_build_dash_panel` | dash icon, SHIFT/RB prompt, two charge bars refilling | done |
| Pad prompts | `hud_icon.gd prompt_text` | LMB/RMB/Shift become RT/LT/RB for pad players | done (view 1 and `--no-kbm` on pads) |
| Crosshair | `crosshair.gd` | four yellow strokes, gap opens with spread (x400) | done |
| Hit marker | `hit_marker.gd` | four diagonal strokes; white body, yellow head, red kill; scale out 0.13 s, fade 0.16 s | done |
| Damage arcs | `damage_indicator.gd` | red arc toward the attacker, 0.28 s | done |
| Damage numbers | `game.gd _show_hit_damage_number` | Label3D 64 px at 0.004 m/px, rises 0.9 m, no depth test | done (projected HUD text, as no_depth_test) |
| Name tags | `player.tscn NameLabel` | Label3D 1.4 m above bots and remote players, outline in identity colour; hidden for seats on this machine, the dead and ghosts | done (sprites from a canvas; walls hide them as in Godot) |
| Death overlay | `show_death_effect`, `_blend_death_into_card_pick` | red after a lava/fall death, blending into the card pick's dark red | done |
| Ghost tint | `ghost` ColorRect | blue tint while spectating | done |
| Pickup toast | `show_pickup_toast` | plate at y 72..132: title and subtitle in the pickup's colour | done |
| Modifier toast | `show_round_modifier` | y 36..116, 24 px, 2.5 s at round start | done |
| Round-win strip | `show_round_win` | YOU WIN THE ROUND / NAME WINS THE ROUND / ROUND WON - WAITING FOR ... (display 30, red shadow) | done |
| Revive label | `_build_revive_panel` | REVIVING... above the plates while reviving a teammate (coop) | done |
| Phoenix white | `set_phoenix_fade` | card phoenix: white rising with the ascent, 0.9 s fade at the sky drop; coop down: white clearing by the apex | done |
| Lens flare / impact wash | `lens_flare_overlay.gd` | shader flare on rockets and flashlights, 2-frame wash on air-strike impacts | air-strike rocket: the ghost chain as HUD discs per view (`view/airstrike.rs` `flare`; blended, Godot adds); impact: the existing explosion flash; flashlights missing |

## Shared overlays (`game.gd`)

| Element | Godot | Shows, when | Rust |
|---|---|---|---|
| Round banner | `RoundBanner`, `_announce` | X died / X is down / X joined / X left / AIR STRIKE INCOMING / ION CANNON INCOMING / LAVA LEAK / WAVE n (enemy list) / WAVE n CLEARED | party names in place of P1..P4; eliminations use the shared kill feed |
| Match-over banner | `_match_over`, `_animate_match_win_banner` | YOU'RE THE WINNER / NAME WINS THE MATCH / WAVE n FAILED at 92, pop from 0.4 with back-out over 0.55 s, then breathing 1.05 | done |
| Match-over buttons | `_build_rematch_overlay` | REMATCH, 5 MORE ROUNDS, EXIT GAME (coop: TRY AGAIN FROM WAVE 1) 260 x 46, fade in at 1.4 s, unlocked at 2.4 s with focus on 5 MORE ROUNDS | done; mouse too. One local vote (Godot's per-peer vote tally is not ported) |
| Kill feed | `_show_kill_feed` | top right from y 96, "A => B", 3+ humans, 3 s then fade, 6 lines | one shared upper-left notice across all player counts; shows the killer and victim or the environmental death, with successive kills shown in order |
| Coop kill count | `Scoreboard` label | KILLED n/m top right in coop | done |
| Tab scoreboard | `_build_tab_overlay` | SCOREBOARD / CO-OP WAVE n, rounds per player, ping or BOT, card pills | includes match kills per player; ping shows "-- ms" (no ping measurement yet) |
| Explosion flash | `trigger_explosion_sidechain` | warm white wash, reach max(6 r, 12) m, alpha up to 0.96, layer 50 | close: per view from each player's distance (Godot: the primary player's); the exposure duck is not ported |
| Round flash | `_round_start_flash` | full white 0.25 s then 0.45 s sine fade, over everything, every round and match start | done |
| Ion flash | `ion_cannon.gd` | brief per-view white at detonation | done (`view::ion`) |
| Network error panel | `_on_network_status_changed` | red panel top right with the error text | done (host lost, hosting failed); Godot's "Logs:" path line is left out |
| Splitscreen hint | `_build_splitscreen_hint` | "2-PLAYER: open menu (Esc) -> SPLITSCREEN" when a spare pad is connected | done (host or offline, no party; one pad counts only while the keyboard was used last) |
| Custom cursor | `_build_custom_cursor` | a drawn cursor so the retro shader bends it | missing: only needed for the retro shader; the OS cursor shows |
| Retro filter | `retro_filter.gd` | optional CRT/fisheye post shader | missing: no such post pass in the renderer |
| Version nudge | `version_check.gd` | "new version" button from a web manifest | not ported (distribution concern, no network call wanted) |
| Dev panel | `dev_panel.gd` | debug hotkeys and toggles | replaced by debug commands (`help`) and the live MCP tools |

## Card pick (`render_player.gd`, `card_face.gd`)

| Element | Rust |
|---|---|
| Dark backdrop blending from the death red, PICK A CARD / WAVE COMPLETE - PICK A CARD title | done |
| Cards 220 x 300 scaled 0.7..1.5, flip-in at 0.5 + 0.19 k s (back-out 0.15 s), 0.75 s lock | done |
| Category band, icon, name, description with pad prompts, stat chips (+/- per stat) | done |
| Focus: lift 10, grow 12 %, eased highlight; idle bob; rare holo sheen and gold shadow | done |
| Exit: picked card flies up, others fade, title fades 0.12 s | done |
| Keyboard, pad and mouse (hover, click) | done |
| Timeout | the 10 s deadline (party setting `card_pick_seconds`) auto-picks in the sim; Godot shows no countdown either |

## Menus

| Element | Godot | Rust |
|---|---|---|
| Start screen | `start_screen.tscn`: dim 0.32, logo 440 x 190, gothic stone frame 350 x 392 with spikes, gold trim and skull, VERSUS / WAVE SURVIVAL / SETTINGS, match ID share, join field, background bot fight with an orbiting camera (82 m, 34 up, FOV 66) | done; the join field with JOIN beside it (see lobby) |
| Pause menu | `_build_pause_menu`: logo, share ID + HOST ONLINE / COPY, join field, Bots - n +, VERSUS, WAVE SURVIVAL, SPLITSCREEN, SETTINGS, RESUME, EXIT GAME, arena seed | done; COPY puts the ID on the clipboard and shows COPIED! for 1.5 s; the join notice follows `_update_join_form` (paste / own ID / valid, connecting / connecting...) |
| Splitscreen Club pause | `_build_gamenight_pause_menu`: slab with red edge and hatching, logo, status line, RESUME, NEW MATCH, SWITCH TO ..., AI PLAYERS - n +, ADVANCED, SETTINGS | done |
| ADVANCED | `_gn_open_advanced`: the party's settings as sliders, sent through Splitscreen Club | done |
| Settings | `build_settings_panel`: Player name, Retro shader, Music, Mouse, Stick (or Aim: NAME per party player), Movement tilt, BACK; saved to settings.cfg | close: Player name (20 characters, Player_NNN at first start, committed on Enter, on leaving the field and on BACK; an empty name keeps the old one) names the first local seat in the scoreboard, kill feed and banners; it is not sent to the host (the protocol has no name) and under Splitscreen Club the party names everyone, so the field is not offered there. No Retro shader; Movement tilt leans the gun (the port has no camera roll); saved to `$XDG_CONFIG_HOME/growing-guns/settings.json` |
| Text fields | Godot `LineEdit`: typing, caret, Ctrl+V/C, IME | done (engine `app::TextField`): typed text, IME commits, Backspace/Delete/arrows/Home/End, Ctrl/Cmd+V and C, Enter submits, Esc backs out, Up/Down/Tab leave the field; the text scrolls to keep the caret in view. No selection or mouse caret placement. A pad gets an on-screen keyboard on A (`app::ui::CharPicker`: letters, digits, - _ ., SPACE, SHIFT, DEL, PASTE, DONE; X deletes, START done, B cancels), which Godot does not have |
| Pause behaviour | one human: the match stops; more: it runs on | done |
| Mouse | hover focuses, click takes, sliders drag; mouse released in menus | done (engine `Game::pointer`, `mouse_capture`) |

## Splitscreen (`splitscreen_manager.gd`)

| Element | Rust |
|---|---|
| Two views side by side, four in quarters | done (`SplitLayout::SideBySide`; the engine keeps a 4 px gap Godot does not have) |
| Each view its own HUD, toasts, card pick, death and phoenix overlays | done |
| Prompts per device (view 1 keyboard, others pad) | done |
| Join prompt "PRESS X TO JOIN" | done: top centre (22, gold, outline 8) while splitscreen is on and fewer than 4 seats are used; the mouse-click join for a pad-led host is not ported |
| SPLITSCREEN (pause menu) | done: on, the first seat keeps the keyboard (and the pad used last); off, every joined seat goes back to a bot, "SPLITSCREEN OFF". Godot also clears the bots and waits for joins; here a joining pad takes over a bot's seat instead. Hidden under Splitscreen Club (the party seats everyone), host or offline only |
| Hot-join | done: an unused pad's X gives it a local seat between ticks on the host: a bot's seat (never a remote's or a wave enemy's), or offline in versus with no bot left a new seat (up to 8 players); online with no bot free it says GAME FULL. The new view, HUD and name tag hiding come at once (the views are rebuilt), "P3 joined". The X does not also reload. `--players N` seats take the devices in order as before |
| PLAYER MENU | done: a joined seat's Start opens it (RESUME / LEAVE, "> LEAVE <" focus, A or X takes, B or Start resumes), drawn in the screen's centre as Godot does; LEAVE gives the seat back to a bot ("P2 left") |

## Lobby and joining

| Element | Rust |
|---|---|
| Host online (iroh) from the menu, match ID shown | done; the ID is shown in the field and printed (`play --host` also writes `growing-guns-ticket.txt`) |
| Join by pasting an ID into the field | done: start screen and pause menu; the ID is taken out of a pasted line or block (`extract_iroh_node_id`), a valid one joins at once (or on Enter / JOIN), your own is refused; the join runs on a thread ("Connecting...", JOIN shows "...") and the new session replaces this one; a failure shows under the menu. `play --join TICKET` still works |
| Splitscreen Club: names, seats, settings, pause | done; names replace P1..P4 in the scoreboard, kill feed, banners, round strip and tags |
| "X joined" / "X left" | done (a remote taking or leaving a seat) |
| Ping per player | missing: the session measures no round trip yet |

## Engine additions made for this

- `render::hud`: `register_font` and `FontFace::Custom`, `measure`,
  `polygon`, `arc`, 2048 text runs; `HudDrawList::provide_image` (a list
  carries its pictures, uploaded once by whoever prepares it).
- `render::split_rects_with` and `SplitLayout` (two views side by side).
- `app::Game`: `split`, `escape`, `quit`, `tuning`; the pointer and
  capture hooks came with the mouse work.
- Text and clipboard: `Game::text_input` and `Game::text` (`app::TextEvent`:
  typed text, IME commits, editing keys, paste, copy), `app::TextField`,
  `app::ui::text_field` and `app::ui::CharPicker`, `app::clipboard`
  (arboard, text only; in-process in scripts and where no clipboard exists).
- Seats: `app::Device`, `Devices::device_controls`, `Game::devices` (every
  device's input before each tick) and `Game::seats` (the devices bound to
  each seat), `app::input::route` and `app::seat_controls`.
- Scripts: `text`, `keys`, `clipboard`, `pads` and `device` steps.
