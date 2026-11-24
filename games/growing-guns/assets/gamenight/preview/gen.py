"""Growing Guns store preview v3 (2026-11-24): five seconds, six maps, all moving.

Joep on v1: "mist de gave actie en gun variatie" (minigun, big explosions).
On v2: the player didn't move, only one map, wants a bit of blackout, and
store clips are 5 s. So: six cuts of 25 frames (one frame per 2 ticks = 30 fps,
150 frames = 5.0 s), each on its own arena, player 0 always strafing or
running (and jumping in some), bots fighting for real.

  a  minigun strafing past three bots
  b  bazooka rockets into a group: real massive blasts
  c  a big blast carves the masonry while running sideways
  d  shotgun + extra barrel, running and jumping at the bots
  e  explosive minigun sweeping past the crowd, booms along the row
  f  blackout: dark arena, flashlights, muzzle flashes

Render: `growing-guns script preview/script.json --players 1 --seed 7` on a
real GPU (Mancave), encode crf 26. Usage: python gen.py [OUT.json]
"""
import json, os, sys

OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "script.json")
FRAMES = 25
steps = [{"cmd": ["arena 85", "health 0 100"], "ticks": 60}]
held = []  # cards player 0 holds, so each cut starts from a clean gun


def gun(cards):
    """Swap player 0's gun cards; the gun grows into them during the lead-in."""
    global held
    cmds = [f"remove 0 {c}" for c in held] + [f"give 0 {c}" for c in cards]
    held = list(cards)
    return cmds


def cut(name, setup, mv, mag=None, look=None, hold=None, jump=(), lead=110, every_cmd=None, refresh=None, every=8):
    """One 25-frame cut. `mag` refills ammo to the gun's real magazine (so the
    HUD shows a normal count); `jump` lists frames that press A."""
    steps.append({"cmd": setup + ["health 0 100"], "ticks": lead})
    for i in range(FRAMES):
        s = {"ticks": 2, "capture": f"{name}_{i:03d}.png", "mv": mv}
        if look: s["look"] = look
        buttons = list(hold or []) + (["SOUTH"] if i in jump else [])
        if buttons: s["hold"] = buttons
        cmds = list((every_cmd or {}).get(i, []))
        if i % every == 0:
            cmds += ["health 0 100"] + ([f"ammo 0 {mag}"] if mag else []) + list(refresh or [])
        if cmds: s["cmd"] = cmds
        steps.append(s)


def face_off(arena, me, at, bots, hp=100):
    c = [f"arena {arena}", f"teleport 0 {me[0]} {me[1]} {me[2]}", f"aim 0 {at[0]} {at[1]} {at[2]}"]
    for i, b in enumerate(bots, start=1):
        c += [f"teleport {i} {b[0]} {b[1]} {b[2]}", f"health {i} {hp}"]
    return c


FIRE = ["RIGHT_TRIGGER"]
TOUGH = [f"health {i} 1000" for i in (1, 2, 3)] + ["bots resume"]  # the minigun would end the round

# a: minigun strafing left past three bots some 20 m off (point-blank looked odd), the stadium behind.
cut("a", gun(["minigun", "big_mag"]) + face_off(85, (10, 2, 18), (0, 2.2, -2), [(-4, 2, -2), (2, 2, -4), (-1, 2, -7)], hp=1000),
    mv=[-0.8, 0.2], mag=120, look=[-0.002, 0.0], hold=FIRE, refresh=TOUGH, every=4)
# b: bazooka into a tight group while backing off to the side.
cut("b", gun(["bazooka"]) + face_off(12, (12, 2, 14), (0, 2, 0), [(-1, 2, 1), (1.5, 2, -1), (0, 2, 2)]),
    mv=[0.6, -0.3], hold=FIRE, lead=140)
# c: a big blast carves the masonry while running sideways (no gun change).
cut("c", face_off(31, (-16, 2, 16), (0, 3, 0), [(1, 2, -1), (-2, 2, -4), (3, 2, 2)]),
    mv=[0.8, 0.3], look=[0.002, 0.0], lead=30, every_cmd={3: ["blast 0 3 0 8 200", "boom 0 2 0 14 120"]})
# d: shotgun with an extra barrel, charging and jumping at the bots.
cut("d", gun(["shotgun", "extra_barrel"]) + face_off(47, (-8, 2, 16), (0, 2, 0), [(-2, 2, 2), (3, 2, -2), (6, 2, 4)]),
    mv=[0.2, 1.0], look=[0.003, 0.0], hold=FIRE, jump=range(6, 10))
# e: minigun with explosive rounds sweeping past the crowd, booms along the row.
cut("e", ["bots pause"] + gun(["minigun", "explosive", "big_mag"]) + face_off(63, (0, 2, -14), (0, 3, 0), [(-2, 2, -2), (2, 2, -4), (0, 2, -6)]),
    mv=[0.7, 0.4], mag=120, look=[0.004, 0.0], hold=FIRE, every=1, every_cmd={4: ["boom -3 1.5 -3 9"], 14: ["boom 2 1.5 -5 11"]})
# f: blackout, the next round starts dark: flashlights and muzzle flashes only.
cut("f", ["bots resume", "modifier blackout", "round", "cage"] + gun(["minigun", "big_mag"])
    + [f"teleport 0 6 2 10", "aim 0 0 2 0"] + [f"teleport {i} {x} 2 {z}" for i, (x, z) in enumerate([(-2, 1), (2, -1), (0, -3)], start=1)],
    mv=[-0.5, 0.6], mag=120, look=[-0.003, 0.0], hold=FIRE, lead=150, refresh=TOUGH, every=4)

steps.append({"state": "end.json"})
json.dump({"size": [1920, 1080], "steps": steps}, open(OUT, "w"))
print(len(steps), "steps,", sum(1 for s in steps if "capture" in s), "frames ->", OUT)
