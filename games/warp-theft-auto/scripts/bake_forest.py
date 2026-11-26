"""Rebuild the shared woodland stand. Run with Python 3; no textures or external packages."""
import json
import math
import random
import re
import struct
from pathlib import Path

from bake_plants import Geometry, Model, TONES, add, between, blade, crown_envelope, scale, shoot, unit

GAME = Path(__file__).resolve().parents[1]
TAU = math.tau


def polar(point, heading, reach, rise):
    return add(point, (reach*math.cos(heading), rise, reach*math.sin(heading)))


def crown(mesh, center, radius, depth, tilt, rng, tone, low):
    """A canopy lobe. Irregular skirts and small groups of leaves break the lower edge."""
    sides = 8
    extents = [radius*rng.uniform(.69,1.12) for _ in range(sides)]
    points, normals, triangles = [], [], []
    for j,(y,width) in enumerate([(-.46,.37),(-.20,1.0),(.43,.77),(.79,.22)]):
        for i,r in enumerate(extents):
            a = TAU*i/sides + tilt + .12*(j%2)
            h = depth*(y+rng.uniform(-.16,.16))
            x,z = math.cos(a),math.sin(a)
            points.append(add(center,(x*r*width,h+.11*r*x,z*r*width)))
            normals.append(unit((x,[-.82,-.18,.58,1.65][j],z)))
            if j:
                lo = (j-1)*sides
                k = (i+1)%sides
                triangles.extend(((lo+i,lo+i+sides,lo+k),(lo+k,lo+i+sides,lo+k+sides)))
    for i in range(1,sides-1):
        triangles.extend(((0,i,i+1),(3*sides,3*sides+i+1,3*sides+i)))
    lobe = Geometry() if low else mesh
    lobe.patch(tone,points,normals,triangles)
    # Folded branch tips catch light at the silhouette and underneath the crown.
    for i in [0,2,5]:
        a = TAU*i/sides+tilt
        tip = polar(center,a,extents[i]*.74,-depth*.27)
        for da in [-.34,.3]:
            angle = a + da
            blade(lobe,tip,(math.cos(angle),rng.uniform(-.76,.12),math.sin(angle)),
                  radius*.22,radius*rng.uniform(.34,.58),tone)
    if low:
        crown_envelope(mesh, lobe.groups[tone][0], tone, 5)


def pine_or_birch(birch, low=False):
    rng = random.Random(142 if birch else 483)
    g = Geometry()
    wood = "birch bark" if birch else "bark"
    h = 8.65 if birch else 10.1
    trunk = [(0,0,0),(.03,1.2,0),(-.05,2.8,.03),(.08,4.1,.02),
             (.21,5.45,-.13),(.11,6.70,-.04),(-.13,h-.90,-.20)]
    r = .22 if birch else .32
    if low:
        shoot(g,[trunk[i] for i in (0,3,6)],[r,r*.58,.048],4,wood)
    else:
        shoot(g,trunk,[r,r*.85,r*.68,r*.58,r*.43,r*.24,.048],7,wood)
    for i in range(6):
        a = 2.40*i+rng.uniform(-.28,.28)
        base = trunk[2+i//2]
        reach = (1.65 if birch else 2.24)*rng.uniform(.72,1.06) * (1-.035*i)
        top = polar(base,a,reach,(2.00+i*.048) if birch else (2.55+i*.077))
        elbow = polar(base,a,reach*.49,(top[1]-base[1])*.34)
        if low:
            shoot(g,[base,top],[r*.57-i*.007,.025],3,wood)
        else:
            shoot(g,[base,elbow,top],[r*.57-i*.007,.079,.025],5,wood)
        size = (1.31 if birch else 1.85)*rng.uniform(.75,1.05)
        depth = size*(1.13 if birch else .63)
        tone = ("sage" if i%3==1 else "sun leaf") if birch else ("pine tips" if i%4==1 else "pine")
        crown(g,top,size,depth,a,rng,tone,low)
    apex = add(trunk[-1],(.10,1.,.08))
    shoot(g,[trunk[-1],apex],[.059,.020],3 if low else 5,wood)
    crown(g,apex,1.27 if birch else 1.54,1.03 if birch else .69,1.2,rng,"sage" if birch else "pine",low)
    return g


def fir_bough(g, base, length, width, a, droop, rng):
    # A spreading twig fan with a raised midrib and individual serrated tips.
    normal = (math.cos(a),0,math.sin(a))
    side = (-normal[2],0,normal[0])
    vs, ns, ix = [], [], []
    for j in range(5):
        t = j/5
        reach = length*t
        center = add(base,add(scale(normal,reach),(0,-droop*t+.22*t*t,0)))
        breadth = width*(.23+math.sin(math.pi*(.06+t*.88)))*(.68 if j%2 else 1.)
        for edge in [-1,0,1]:
            tip = add(center,scale(side,breadth*edge*rng.uniform(.81,1.13)))
            vs.append(add(tip,(0,(.28 if edge==0 else -.18)*(length/2),0)))
            ns.append(unit(add(scale(normal,.19),add((0,1,0),scale(side,.79*edge)))))
        if j:
            i = (j-1)*3
            ix.extend([(i,i+1,i+3),(i+1,i+4,i+3),(i+1,i+2,i+4),(i+2,i+5,i+4)])
    vs.append(add(base,add(scale(normal,length),(0,.19-droop,0))))
    ns.append(unit(add(normal,(0,1,0))))
    ix.extend(((12,13,15),(13,14,15)))
    # The fan can be seen from the trail and from above. Use the existing
    # opaque two-sided leaf material convention from the street vegetation.
    g.patch("deep leaf",vs,ns,ix)


def fir(low=False):
    rng = random.Random(553)
    g = Geometry()
    shoot(g,[(0,0,0),(.04,2.45,.02),(-.08,6.,-.03),(.04,9.,-.04),(.08,12.2,0)],
          [.29,.205,.14,.083,.012],4 if low else 7)
    for ring in range(8):
        height = 3.5 + ring*1.19
        reach = (3.03-ring*.32)*rng.uniform(.91,1.06)
        count = 5 if ring<5 else 4
        whorl = Geometry() if low else g
        for arm in range(count):
            a = TAU*(arm/count + ring*.217) + rng.uniform(-.15,.15)
            length = reach*rng.uniform(.76,1.14)
            base = (.035*math.sin(ring),height+rng.uniform(-.26,.26),0)
            fir_bough(whorl,base,length,length*.37,a,.24+.46*length,rng)
        if low:
            crown_envelope(g, whorl.groups["deep leaf"][0], "deep leaf", count)
    return g


def understory():
    rng = random.Random(865)
    g = Geometry()
    for i in range(4):
        a = i*2.4
        x,z = math.cos(a),math.sin(a)
        run = rng.uniform(.68,1.08)
        rise = rng.uniform(.40,.79)
        root = (x*.04,.03,z*.04)
        bend = (x*.30*run,rise*.84,z*.30*run)
        end = (x*run,rise*.52,z*run)
        shoot(g,[root,bend,end],[.018,.012,.006],3,"sage")
        for j in range(4):
            t = (j+.45)/4
            p = between(bend,end,t)
            for hand in [-1,1]:
                direction = a+hand*1.02
                blade(g,p,(math.cos(direction),.20-t*.59,math.sin(direction)),.115*(1-.52*t),.29*(1-.56*t),"sage")
    return g


def stand(low=False):
    # The same root records drive the collision query in Luau. Include only
    # the mesh/scene placements here; no renderer or physics state is cooked.
    recipe = (GAME/"forest.module.luau").read_text()
    decimal = r"(-?[\d.]+)"
    pattern = r"\{" + ",".join([decimal]*4) + r',"(pine|birch|fir)"\}'
    roots = re.findall(pattern,recipe)
    assert len(roots)==15
    m = Model()
    # These two extra tones use the same physically matte materials as the street.
    TONES.update({"pine":(.062,.139,.073),"pine tips":(.101,.192,.093),"birch bark":(.50,.46,.34)})
    parts = {"pine":m.part(pine_or_birch(False,low)), "birch":m.part(pine_or_birch(True,low)), "fir":m.part(fir(low))}
    for x,z,k,degrees,kind in roots:
        m.place(parts[kind],(float(x),0,float(z)),math.radians(float(degrees)),size=(float(k),)*3)
    if low:
        return m
    fern = m.part(understory())
    rng = random.Random(3174)
    for _ in range(15):
        x,z = rng.uniform(-10.7,10.7),rng.uniform(-10.7,10.7)
        k = rng.uniform(.7,1.30)
        m.place(fern,(x,0,z),rng.random()*TAU,size=(k,k,k))
    # Crown details are centimetres across. A 1/32 m grid compresses their
    # float attributes while keeping finer stem and ground-fern geometry intact.
    for geometry in m.meshes[:3]:
        for tone,(points,normals,_) in geometry.groups.items():
            if "bark" not in tone:
                points[:] = [tuple(round(v*32)/32 for v in p) for p in points]
                normals[:] = [unit(tuple(round(v*6)/6 for v in n)) for n in normals]
    return m


def trail_copse(source_name="forest-stand.glb"):
    # Reuse the indexed positions, normals and materials already downloaded with
    # forest-stand. The second glTF describes only a narrow arrangement of them.
    source = (GAME / "assets/plants" / source_name).read_bytes()
    json_bytes = struct.unpack_from("<I", source, 12)[0]
    doc = json.loads(source[20:20 + json_bytes])
    nodes = doc["nodes"]
    config = (GAME / "forest.module.luau").read_text().split("local trailRoots = {", 1)[1].split("\n}", 1)[0]
    roots = re.findall(r"\{(\d+),([-\d.]+),([-\d.]+)\}", config)
    assert len(roots) == 4
    doc["nodes"] = [dict(nodes[int(i)-1], translation=[float(x),0,float(z)]) for i,x,z in roots]
    doc["scenes"] = [{"nodes": list(range(len(roots)))}]
    doc["buffers"] = [{"uri": source_name, "byteLength": len(source)}]
    for view in doc["bufferViews"]:
        view["byteOffset"] += 28 + json_bytes
    return json.dumps(doc, separators=(",", ":")).encode()


if __name__ == "__main__":
    stand().save("forest-stand",(1,1,1))
    (GAME / "assets/plants/trail-copse.gltf").write_bytes(trail_copse())
    stand(low=True).save("forest-stand-low",(1,1,1),closed=True)
    (GAME / "assets/plants/trail-copse-low.gltf").write_bytes(trail_copse("forest-stand-low.glb"))
