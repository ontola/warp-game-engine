"""Growing Guns store preview v5 (2026-11-24): real gameplay, then an edit.

Joep on v4: "Hij schiet overal gewoon op de muur. Dat is een beetje
belachelijk. Doe ook dat ze op elkaar schieten, dat ze een beetje springen.
Dat er een keer een raket uit de lucht komt. Gewoon wat gevarieerde, maar wel
echte gameplay." So nothing is staged any more: a bot plays player 0
(`autopilot 0`), all four bots fight, jump and pick cards for real, and each
segment just hands out a loadout and lets the round run.

Two steps:
  1. capture: `python gen.py` writes script.json (five 4 s segments, 600
     frames, one every 2 ticks = 30 fps). Render it on a real GPU (Mancave):
     `growing-guns script preview/script.json --out DIR --players 1 --seed 7`,
     then encode all frames in order to raw.mp4 (`ffmpeg -framerate 30 -i
     DIR/%*.png` sorted by name, crf 18).
  2. edit: `python gen.py edit raw.mp4 preview.mp4 preview.jpg` cuts the
     EDIT windows below (5.0 s total) and the poster frame. EDIT is chosen by
     watching raw.mp4: pick moments where players shoot each other.

Segments:
  a  arena 85: player 0 minigun, the bots bazooka / shotgun x2 / explosive
  b  arena 47: player 0 bazooka, the bots minigun / rapid fire / homing
  c  arena 12: player 0 explosive rapid fire, the bots shotgun / minigun / sniper
  d  an AIR STRIKES round: rockets fall from the sky; player 0 shotgun x2
  e  a BLACKOUT round: flashlights and muzzle flashes; player 0 minigun
"""
import json, os, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
SEG_FRAMES = 120  # 4 s per segment
FPS = 30

# (segment, first frame, frame count) in raw.mp4 order; 150 frames = 5.0 s.
# Placeholder until the first raw render has been watched.
EDIT = [("a", 20, 25), ("b", 40, 25), ("c", 30, 25), ("d", 50, 25), ("e", 40, 25), ("a", 80, 25)]
POSTER = ("d", 60)

steps = [{"cmd": ["arena 85", "autopilot 0 on", "bots resume", "modifier none"], "ticks": 60}]
held = {i: [] for i in range(4)}


def loadout(cards):
    """cards[i] = player i's cards; replaces what they had from earlier segments."""
    cmds = []
    for i, want in enumerate(cards):
        cmds += [f"remove {i} {c}" for c in held[i]] + [f"give {i} {c}" for c in want]
        held[i] = list(want)
    return cmds


def segment(name, setup, lead=240):
    """Let the round run `lead` ticks (bots meet up, guns grow), then capture.
    Only player 0's health is topped up, so the camera stays in the fight;
    the bots win and lose for real."""
    steps.append({"cmd": setup, "ticks": lead})
    for i in range(SEG_FRAMES):
        s = {"ticks": 2, "capture": f"{name}_{i:03d}.png"}
        if i % 16 == 0: s["cmd"] = ["health 0 100"]
        steps.append(s)


def round_with(modifier):
    """Modifiers take hold at a round start; `round` builds that round's own arena."""
    return [f"modifier {modifier}", "round", "cage"]


segment("a", ["arena 85"] + loadout([["minigun", "big_mag"], ["bazooka"], ["shotgun", "extra_barrel"], ["explosive", "rapid_fire"]]))
segment("b", ["arena 47"] + loadout([["bazooka"], ["minigun"], ["rapid_fire", "big_mag"], ["homing", "dmg_up"]]))
segment("c", ["arena 12"] + loadout([["explosive", "rapid_fire"], ["shotgun"], ["minigun", "big_mag"], ["sniper"]]))
segment("d", round_with("air_strikes") + loadout([["shotgun", "extra_barrel"], ["bazooka"], ["minigun"], ["explosive"]]), lead=300)
segment("e", round_with("blackout") + loadout([["minigun", "big_mag"], ["shotgun"], ["rapid_fire"], ["bazooka"]]), lead=240)
steps.append({"cmd": ["modifier none", "autopilot 0 off"], "ticks": 1})
steps.append({"state": "end.json"})


def edit(raw, out, poster):
    order = [s for s in "abcde"]
    start = lambda seg, f: (order.index(seg) * SEG_FRAMES + f) / FPS
    parts = "".join(f"[0:v]trim=start={start(s, f):.4f}:duration={n / FPS:.4f},setpts=PTS-STARTPTS[v{k}];" for k, (s, f, n) in enumerate(EDIT))
    graph = parts + "".join(f"[v{k}]" for k in range(len(EDIT))) + f"concat=n={len(EDIT)}:v=1:a=0[out]"
    subprocess.run(["ffmpeg", "-y", "-v", "error", "-i", raw, "-filter_complex", graph, "-map", "[out]", "-an",
                    "-c:v", "libx264", "-crf", "26", "-pix_fmt", "yuv420p", "-movflags", "+faststart", out], check=True)
    subprocess.run(["ffmpeg", "-y", "-v", "error", "-ss", f"{start(*POSTER):.4f}", "-i", raw, "-frames:v", "1", "-q:v", "3", poster], check=True)
    print("edited", sum(n for _, _, n in EDIT), "frames ->", out, "+", poster)


if len(sys.argv) > 1 and sys.argv[1] == "edit":
    edit(*sys.argv[2:5])
else:
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "script.json")
    json.dump({"size": [1920, 1080], "steps": steps}, open(out, "w"))
    print(len(steps), "steps,", sum(1 for s in steps if "capture" in s), "frames ->", out)
