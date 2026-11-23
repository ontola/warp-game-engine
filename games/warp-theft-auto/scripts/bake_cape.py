"""Bake the shared offshore cape. Run from any directory with Python 3."""
import json
import math
from pathlib import Path
import struct

N = 44
RINGS = 10


def point(angle, r):
    coast = 1 + .11 * math.sin(3*angle+.6) + .075 * math.sin(7*angle) + .033 * math.cos(13*angle)
    x = .51*r*coast*math.cos(angle)
    z = .29*r*coast*math.sin(angle) + .08*math.sin(3*x)
    # A long weathered spine, with a lower saddle and a high western bluff.
    spine = .78 + .30*math.sin(10*x+.5) + .15*math.cos(15*x-3*z)
    crest = (1-r)**.88 * spine
    shelf = .035*(1-r) + .068*max(0, min(1, (1-r)*8))
    return (x, crest + shelf - .044, z)


def bake():
    vertices = [(0.0, .92, 0.0)]
    for row in range(1, RINGS+1):
        for i in range(N):
            angle = 2*math.pi*(i+.27*(row % 2))/N
            vertices.append(point(angle, row/RINGS))
    indices = [[], [], []]
    for i in range(N):
        indices[1].extend((0, 1+(i+1)%N, 1+i))
    for ring in range(1, RINGS):
        for i in range(N):
            lo = 1 + (ring-1)*N
            hi = lo + N
            a,b,c,d = lo+i, lo+(i+1)%N, hi+i, hi+(i+1)%N
            material = 2 if ring == RINGS-1 else int(ring < 6 or (i//4+ring)%5 != 0)
            indices[material].extend((a,b,c, b,d,c))
    data = bytearray()
    views, accessors = [], []

    def accessor(values, fmt, kind, count, fields):
        while len(data)%4:
            data.append(0)
        raw = struct.pack("<" + fmt*len(values), *values)
        views.append({"buffer": 0, "byteOffset":len(data), "byteLength":len(raw)})
        data.extend(raw)
        accessors.append({"bufferView":len(views)-1,"componentType":kind,"count":count,**fields})
        return len(accessors)-1

    pos = accessor([a for p in vertices for a in p],"f",5126,len(vertices),{
        "type":"VEC3","min":list(map(min,zip(*vertices))),"max":list(map(max,zip(*vertices)))})
    # Stable normals in normalized mesh space; Warp transforms them for each instance.
    normals = [[0.0,0.0,0.0] for _ in vertices]
    for group in indices:
        for k in range(0,len(group),3):
            face = group[k:k+3]
            a,b,c = (vertices[i] for i in face)
            u,v = [p-q for p,q in zip(b,a)], [p-q for p,q in zip(c,a)]
            cross = [u[1]*v[2]-u[2]*v[1], u[2]*v[0]-u[0]*v[2], u[0]*v[1]-u[1]*v[0]]
            for i in face:
                for j in range(3):
                    normals[i][j] += cross[j]
    norm = accessor([x/math.hypot(*n) for n in normals for x in n],"f",5126,len(vertices),{"type":"VEC3"})
    primitives = []
    tones = [[.17,.22,.22,1], [.15,.24,.19,1], [.40,.39,.31,1]]
    for i,group in enumerate(indices):
        ix = accessor(group,"H",5123,len(group),{"type":"SCALAR"})
        primitives.append({"attributes":{"POSITION":pos,"NORMAL":norm},"indices":ix,"material":i})
    doc = {"asset":{"version":"2.0","generator":"Port Meridian cape baker"},
           "scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
           "meshes":[{"primitives":primitives}],"bufferViews":views,"accessors":accessors,
           "materials":[{"pbrMetallicRoughness":{"baseColorFactor":c,"metallicFactor":0,"roughnessFactor":1}} for c in tones],
           "buffers":[{"byteLength":len(data)}]}
    desc = json.dumps(doc,separators=(",",":")).encode()
    desc += b" " * (-len(desc)%4)
    data.extend(b"\0" * (-len(data)%4))
    chunks = struct.pack("<I4s",len(desc),b"JSON")+desc+struct.pack("<I4s",len(data),b"BIN\0")+data
    target = Path(__file__).resolve().parents[1]/"assets"/"cape.glb"
    target.write_bytes(struct.pack("<4sII",b"glTF",2,12+len(chunks))+chunks)


if __name__ == "__main__":
    bake()
