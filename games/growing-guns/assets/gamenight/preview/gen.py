"""Growing Guns store preview: one continuous scripted match, five staged moments.

Each cut captures 30 frames, one every 2 ticks (60 Hz sim -> 30 fps).
Bots are placed in front of player 0 and fight for real; player 0's health is
topped up to 100 so the HUD shows a normal value.
"""
import json, sys

ARENA = sys.argv[1] if len(sys.argv) > 1 else "85"
steps = [{"cmd": [f"arena {ARENA}", "health 0 100"], "ticks": 120}]


def cut(name, setup, frames=30, mv=None, look=None, hold=None, lead=20, every_cmd=None):
    steps.append({"cmd": setup + ["health 0 100"], "ticks": lead})
    for i in range(frames):
        s = {"ticks": 2, "capture": f"{name}_{i:03d}.png"}
        if mv: s["mv"] = mv
        if look: s["look"] = look
        if hold: s["hold"] = hold
        cmds = list((every_cmd or {}).get(i, []))
        if i % 8 == 0: cmds.append("health 0 100")
        if cmds: s["cmd"] = cmds
        steps.append(s)


def face_off(me, at, bots):
    c = [f"teleport 0 {me[0]} {me[1]} {me[2]}", f"aim 0 {at[0]} {at[1]} {at[2]}"]
    for i, b in enumerate(bots, start=1):
        c.append(f"teleport {i} {b[0]} {b[1]} {b[2]}")
    return c


# 1: a firefight, the bots spread in front, the stadium behind them.
cut("a", face_off((7, 2, 12), (0, 2.2, 1), [(-3, 2, 3), (2, 2, 0), (-1, 2, -3)]), look=[-0.003, 0.0], hold=["RIGHT_TRIGGER"])
# 2: an explosion among the bots.
cut("b", face_off((10, 2, 12), (0, 2, 0), [(-1, 2, 1), (1.5, 2, -1), (0, 2, 2)]), lead=10, every_cmd={4: ["boom 0 1.5 0 6"]})
# 3: a blast carves the masonry beside a bot.
cut("c", face_off((-10, 2, 10), (0, 3, 0), [(1, 2, -1), (-2, 2, -4), (3, 2, 2)]), lead=10, every_cmd={2: ["blast 0 3 0 4 80"], 12: ["boom 2 2 -2 5"]})
# 4: running at the bots, firing.
cut("d", face_off((-8, 2, 16), (0, 2, 0), [(-2, 2, 2), (3, 2, -2), (6, 2, 4)]), mv=[0.2, 1.0], look=[0.003, 0.0], hold=["RIGHT_TRIGGER"])
# 5: the crowd and the stands, a wide sweep while firing.
cut("e", face_off((0, 2, -14), (0, 3, 0), [(-2, 2, -2), (2, 2, -4), (0, 2, -6)]), lead=10, look=[0.003, 0.0], hold=["RIGHT_TRIGGER"], every_cmd={6: ["boom -1 1.5 -3 6"]})

steps.append({"state": "end.json"})
json.dump({"size": [960, 540], "steps": steps}, open(r"C:\dev\perf\2026-11-22-gg-preview\preview.json", "w"))
print(len(steps), "steps, arena", ARENA)
