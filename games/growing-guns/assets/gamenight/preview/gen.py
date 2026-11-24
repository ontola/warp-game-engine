"""Growing Guns store preview v2 (2026-11-24): more action and gun variety.

Joep on v1: "mist de gave actie en gun variatie. Ik wil wel een minigun zien
en een paar grote explosies". Same pipeline as v1:
one continuous scripted match, staged cuts, each frame = 2 ticks (30 fps).

Cuts (about 15 s at 30 fps):
  a  MINIGUN shredding three bots in front of the stands (60 frames)
  b  BAZOOKA rockets into a group: real massive blasts (45 frames)
  c  air-strike sized blast (30 m) that carves the masonry (45 frames)
  d  SHOTGUN + EXTRA BARREL while running at the bots (40 frames)
  e  minigun + EXPLOSIVE rounds sweeping across the crowd, chained booms (60 frames)

Usage: python gen.py [ARENA] [OUT.json]   (default OUT: script.json next to this file)
"""
import json, os, sys

ARENA = sys.argv[1] if len(sys.argv) > 1 else "85"
OUT = sys.argv[2] if len(sys.argv) > 2 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "script.json")
steps = [{"cmd": [f"arena {ARENA}", "health 0 100"], "ticks": 120}]
held = []  # cards player 0 currently holds, so each cut starts from a clean gun


def gun(cards):
    """Swap player 0's gun cards; the gun grows into them during the lead-in."""
    global held
    cmds = [f"remove 0 {c}" for c in held] + [f"give 0 {c}" for c in cards]
    held = list(cards)
    return cmds


def cut(name, setup, frames=30, mv=None, look=None, hold=None, lead=20, every_cmd=None):
    steps.append({"cmd": setup + ["health 0 100", "ammo 0 999"], "ticks": lead})
    for i in range(frames):
        s = {"ticks": 2, "capture": f"{name}_{i:03d}.png"}
        if mv: s["mv"] = mv
        if look: s["look"] = look
        if hold: s["hold"] = hold
        cmds = list((every_cmd or {}).get(i, []))
        if i % 8 == 0: cmds += ["health 0 100", "ammo 0 999"]
        if cmds: s["cmd"] = cmds
        steps.append(s)


def face_off(me, at, bots):
    c = [f"teleport 0 {me[0]} {me[1]} {me[2]}", f"aim 0 {at[0]} {at[1]} {at[2]}"]
    for i, b in enumerate(bots, start=1):
        c += [f"teleport {i} {b[0]} {b[1]} {b[2]}", f"health {i} 100"]
    return c


FIRE = ["RIGHT_TRIGGER"]

# a: minigun, ten spinning barrels, tracers into the bots; the stadium behind.
cut("a", gun(["minigun", "big_mag"]) + face_off((7, 2, 12), (0, 2.2, 1), [(-3, 2, 3), (2, 2, 0), (-1, 2, -3)]),
    frames=60, lead=110, look=[-0.002, 0.0], hold=FIRE)
# b: bazooka rockets into a tight group, two real massive blasts plus one queued boom.
cut("b", gun(["bazooka"]) + face_off((12, 2, 14), (0, 2, 0), [(-1, 2, 1), (1.5, 2, -1), (0, 2, 2)]),
    frames=45, lead=110, hold=FIRE, every_cmd={18: ["boom -2 1.5 -2 12 80"]})
# c: an air-strike sized blast (radius 30 like the rocket) that carves the masonry.
cut("c", face_off((-16, 2, 16), (0, 3, 0), [(1, 2, -1), (-2, 2, -4), (3, 2, 2)]),
    frames=45, lead=20, every_cmd={6: ["blast 0 3 0 8 200", "boom 0 2 0 30 120"], 26: ["boom 4 2 -3 10"]})
# d: shotgun with an extra barrel, running at the bots and firing.
cut("d", gun(["shotgun", "extra_barrel"]) + face_off((-8, 2, 16), (0, 2, 0), [(-2, 2, 2), (3, 2, -2), (6, 2, 4)]),
    frames=40, lead=110, mv=[0.2, 1.0], look=[0.003, 0.0], hold=FIRE)
# e: minigun with explosive rounds sweeping past the crowd, booms chaining along the row.
cut("e", gun(["minigun", "explosive", "big_mag"]) + face_off((0, 2, -14), (0, 3, 0), [(-2, 2, -2), (2, 2, -4), (0, 2, -6)]),
    frames=60, lead=110, look=[0.003, 0.0], hold=FIRE,
    every_cmd={10: ["boom -3 1.5 -3 9"], 24: ["boom 2 1.5 -5 11"], 40: ["boom 0 1.5 -1 14 90"]})

steps.append({"state": "end.json"})
json.dump({"size": [960, 540], "steps": steps}, open(OUT, "w"))
print(len(steps), "steps, arena", ARENA, "->", OUT)
