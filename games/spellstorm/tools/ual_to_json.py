#!/usr/bin/env python3
"""Quaternius' Universal Animation Library (CC0) to app/assets/anim/ual.json.

    python3 -I tools/ual_to_json.py UAL1_Standard.glb app/assets/anim/ual.json

Get UAL1_Standard.glb from https://quaternius.itch.io/universal-animation-library
(the free Standard pack, folder Unreal-Godot). Only the body bones are kept
(no fingers). Each clip is sampled at 30 fps; per bone and frame it stores the
bone's rotation in model space (Y up, facing +Z, the character's left at +X)
relative to a rest pose: the T-pose with the arms let down to hang at the
sides, the way our monsters are modelled. A monster turns its parts by these
deltas (see app/src/rig.rs). The pelvis also stores how far it moved from its
rest spot, over its height.
"""
import json, math, struct, sys

FPS = 30
BONES = ["pelvis", "spine_01", "spine_02", "spine_03", "neck_01", "Head",
         "clavicle_l", "upperarm_l", "lowerarm_l", "hand_l",
         "clavicle_r", "upperarm_r", "lowerarm_r", "hand_r",
         "thigh_l", "calf_l", "foot_l", "ball_l",
         "thigh_r", "calf_r", "foot_r", "ball_r"]
CLIPS = ["Idle_Loop", "Walk_Loop", "Jog_Fwd_Loop", "Sprint_Loop", "Crouch_Idle_Loop",
         "Crouch_Fwd_Loop", "Death01", "Hit_Chest", "Hit_Head", "Punch_Cross", "Punch_Jab",
         "Sword_Attack", "Sword_Idle", "Spell_Simple_Enter", "Spell_Simple_Idle_Loop",
         "Spell_Simple_Shoot", "Spell_Simple_Exit", "Jump_Start", "Jump_Land", "Roll"]
REF = "A_TPose"
# How far the rest pose lets the arms down from the T-pose, in degrees.
ARM_DROP = 78.0


def load(path):
    b = open(path, "rb").read()
    assert b[:4] == b"glTF", "not a glb"
    off, js, bin_ = 12, None, None
    while off < len(b):
        ln, ty = struct.unpack_from("<II", b, off)
        off += 8
        if ty == 0x4E4F534A:
            js = json.loads(b[off:off + ln])
        else:
            bin_ = b[off:off + ln]
        off += ln
    return js, bin_


def accessor(js, bin_, i):
    a = js["accessors"][i]
    v = js["bufferViews"][a["bufferView"]]
    n = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}[a["type"]]
    assert a["componentType"] == 5126
    o = v.get("byteOffset", 0) + a.get("byteOffset", 0)
    st = v.get("byteStride", n * 4)
    return [struct.unpack_from("<%df" % n, bin_, o + k * st) for k in range(a["count"])]


def qmul(a, b):
    ax, ay, az, aw = a
    bx, by, bz, bw = b
    return (aw * bx + ax * bw + ay * bz - az * by,
            aw * by - ax * bz + ay * bw + az * bx,
            aw * bz + ax * by - ay * bx + az * bw,
            aw * bw - ax * bx - ay * by - az * bz)


def qinv(q):
    return (-q[0], -q[1], -q[2], q[3])


def qrot(q, v):
    p = qmul(qmul(q, (v[0], v[1], v[2], 0.0)), qinv(q))
    return p[:3]


def nlerp(a, b, t):
    d = sum(x * y for x, y in zip(a, b))
    s = -1.0 if d < 0 else 1.0
    q = [x + (s * y - x) * t for x, y in zip(a, b)]
    n = math.sqrt(sum(x * x for x in q)) or 1.0
    return tuple(x / n for x in q)


def sample(times, values, t, rot):
    if t <= times[0]:
        return values[0]
    for k in range(1, len(times)):
        if t <= times[k]:
            u = (t - times[k - 1]) / max(times[k] - times[k - 1], 1e-9)
            a, b = values[k - 1], values[k]
            return nlerp(a, b, u) if rot else tuple(x + (y - x) * u for x, y in zip(a, b))
    return values[-1]


def main(src, out):
    js, bin_ = load(src)
    nodes = js["nodes"]
    parent = {}
    for i, n in enumerate(nodes):
        for c in n.get("children", []):
            parent[c] = i
    index = {n.get("name"): i for i, n in enumerate(nodes)}

    def world(local, i):
        """(rotation, translation) of node i in the scene."""
        chain = []
        j = i
        while j is not None:
            chain.append(j)
            j = parent.get(j)
        r, t = (0.0, 0.0, 0.0, 1.0), (0.0, 0.0, 0.0)
        for j in reversed(chain):
            lr, lt = local[j]
            t = tuple(a + b for a, b in zip(t, qrot(r, lt)))
            r = qmul(r, lr)
        return r, t

    rest = {i: (tuple(n.get("rotation", (0, 0, 0, 1))), tuple(n.get("translation", (0, 0, 0)))) for i, n in enumerate(nodes)}
    anims = {a["name"]: a for a in js["animations"]}

    def frames(name):
        a = anims[name]
        tracks = {}
        end = 0.0
        for ch in a["channels"]:
            path = ch["target"]["path"]
            if path == "scale":
                continue
            s = a["samplers"][ch["sampler"]]
            times = [x[0] for x in accessor(js, bin_, s["input"])]
            end = max(end, times[-1])
            tracks[(ch["target"]["node"], path)] = (times, accessor(js, bin_, s["output"]))
        n = max(1, round(end * FPS)) + 1
        out = []
        for f in range(n):
            t = min(f / FPS, end)
            local = dict(rest)
            for (node, path), (times, vals) in tracks.items():
                v = sample(times, vals, t, path == "rotation")
                r, tr = local[node]
                local[node] = (v, tr) if path == "rotation" else (r, v)
            out.append({b: world(local, index[b]) for b in BONES})
        return end, out

    _, ref_frames = frames(REF)
    ref = dict(ref_frames[0])
    for b in BONES:
        side = 1.0 if b.endswith("_l") else -1.0
        if b.split("_")[0] in ("upperarm", "lowerarm", "hand"):
            # Down about +Z: the left arm (+X) to -Y, the right (-X) too.
            a = math.radians(ARM_DROP) * -side / 2.0
            drop = (0.0, 0.0, math.sin(a), math.cos(a))
            r, t = ref[b]
            sh = ref["upperarm" + b[-2:]][1]
            t = tuple(x + y for x, y in zip(sh, qrot(drop, tuple(p - q for p, q in zip(t, sh)))))
            ref[b] = (qmul(drop, r), t)
    hip = ref["pelvis"][1][1]
    # Check the facing: toes ahead of the ankles, the left foot at +X.
    fwd = [a - b for a, b in zip(ref["ball_l"][1], ref["foot_l"][1])]
    side = ref["thigh_l"][1][0] - ref["thigh_r"][1][0]
    assert fwd[2] > abs(fwd[0]), ("UAL no longer faces +Z", fwd)
    assert side > 0, ("UAL's left is no longer +X", side)
    clips = {}
    for name in CLIPS:
        end, fr = frames(name)
        rot = {b: [] for b in BONES}
        root = []
        for w in fr:
            for b in BONES:
                d = qmul(w[b][0], qinv(ref[b][0]))
                if d[3] < 0:
                    d = tuple(-x for x in d)
                rot[b].extend(round(x, 4) for x in d)
            root.extend(round((a - b) / hip, 4) for a, b in zip(w["pelvis"][1], ref["pelvis"][1]))
        # How fast the clip walks (in hip heights a second): the foot on the
        # floor slides back under the in-place body at that speed.
        slide = []
        for a, b in zip(fr, fr[1:]):
            foot = min(("ball_l", "ball_r"), key=lambda f: a[f][1][1])
            if a[foot][1][1] < 0.05:
                slide.append((a[foot][1][2] - b[foot][1][2]) * FPS / hip)
        slide.sort()
        speed = slide[len(slide) // 2] if len(slide) > 4 else 0.0
        # When a blow lands: the hand at its fastest.
        fastest, hit = 0.0, 0.0
        for k, (a, b) in enumerate(zip(fr, fr[1:])):
            for h in ("hand_l", "hand_r"):
                v = math.dist(a[h][1], b[h][1])
                if v > fastest:
                    fastest, hit = v, (k + 1) / FPS
        clips[name] = {"length": round(end, 4), "frames": len(fr), "speed": round(max(speed, 0.0), 3),
                       "hit": round(hit, 3), "rot": rot, "root": root}
        print(f"{name}: {len(fr)} frames, {end:.2f} s, speed {speed * hip:.2f} m/s, hit {hit:.2f} s")
    data = {
        "source": "Universal Animation Library (Standard) by Quaternius, CC0, https://quaternius.itch.io/universal-animation-library",
        "fps": FPS,
        "bones": BONES,
        "hip": round(hip, 4),
        "joints": {b: [round(x, 4) for x in ref[b][1]] for b in BONES},
        "clips": clips,
    }
    with open(out, "w") as f:
        json.dump(data, f, separators=(",", ":"))


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
