"""Bake the street tree and border from reusable opaque shoots. All dimensions are metres."""
import json
import math
import random
import struct
from pathlib import Path

OUT = Path(__file__).resolve().parents[1] / "assets" / "plants"
UP = (0, 1, 0)
TAU = math.tau
# PBR base colours are linear. The lower leaf surface uses the same matte material.
TONES = {
    "bark": (.22, .153, .093), "mulch": (.105, .066, .038),
    "edging": (.28, .265, .206), "deep leaf": (.078, .169, .057),
    "sun leaf": (.154, .252, .072), "sage": (.185, .290, .148),
    "flower": (.60, .39, .57),
}


def add(a, b):
    return tuple(x + y for x, y in zip(a, b))


def scale(a, k):
    return tuple(x * k for x in a)


def between(a, b, f):
    return add(scale(a, 1 - f), scale(b, f))


def cross(a, b):
    return (a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0])


def unit(a):
    return scale(a, 1/math.hypot(*a))


class Geometry:
    def __init__(self):
        self.groups = {}

    def patch(self, tone, points, normals, triangles):
        p, n, ix = self.groups.setdefault(tone, ([], [], []))
        start = len(p)
        p.extend(points)
        n.extend(normals)
        ix.extend(start + i for triangle in triangles for i in triangle)


def shoot(mesh, points, radii, sides=6, tone="bark"):
    # Tapered wood with smooth normals. Overlapping rings hide the ends at forks.
    verts, norms, triangles = [], [], []
    for j, (point, radius) in enumerate(zip(points, radii)):
        tangent = unit(add(points[min(j+1, len(points)-1)], scale(points[max(j-1, 0)], -1)))
        right = unit(cross(tangent, (0, 0, 1) if abs(tangent[2]) < .90 else (1, 0, 0)))
        front = cross(tangent, right)
        for i in range(sides):
            angle = TAU*i/sides
            normal = add(scale(right, math.cos(angle)), scale(front, math.sin(angle)))
            p = add(point, scale(normal, radius))
            verts.append((p[0],0,p[2]) if j == 0 and point[1] == 0 else p)
            norms.append(normal)
            if j:
                a, b = (j-1)*sides+i, (j-1)*sides+(i+1)%sides
                triangles.extend(((a, b, b+sides), (a, b+sides, a+sides)))
    mesh.patch(tone, verts, norms, triangles)


def soil(mesh, w, d):
    # Clipped corners on the corten/stone lip; the soil is slightly recessed.
    def ring(width, depth, y):
        x, z = width/2, depth/2
        k = min(x, z)*.18
        return [(a,y,b) for a,b in [(-x+k,-z),(-x,-z+k),(-x,z-k),(-x+k,z),
                                     (x-k,z),(x,z-k),(x,-z+k),(x-k,-z)]]
    for low, high in [(ring(w,d,0),ring(w,d,.034)), (ring(w,d,.034),ring(w-.07,d-.07,.031))]:
        for i in range(8):
            j = (i+1)%8
            pts = [low[i],low[j],high[j],high[i]]
            n = unit(cross(add(pts[1],scale(pts[0],-1)),add(pts[2],scale(pts[0],-1))))
            # The top ring winds upwards; outer wall faces away from the bed.
            mesh.patch("edging",pts,[n]*4,[(0,1,2),(0,2,3)])
    inside = ring(w-.07,d-.07,.031)
    mesh.patch("mulch", inside, [UP]*8, [(0,i,i+1) for i in range(1,7)])


def blade(mesh, root, axis, width, length, tone):
    # A folded ovate blade, like plant_spray::leaf, with one shared midrib.
    axis = unit(axis)
    side = unit(cross(UP,axis))
    normal = unit(cross(axis,side))
    middle = add(root,scale(axis,length*.44))
    ridge = add(middle,scale(normal,width*.20))
    pts = [root, add(middle,scale(side,-width*.5)), add(root,scale(axis,length)),
           add(middle,scale(side,width*.5)), ridge]
    across = scale(side,.28)
    ns = [normal,unit(add(normal,scale(across,-1))),normal,unit(add(normal,across)),normal]
    mesh.patch(tone,pts,ns,[(0,1,4),(1,2,4),(2,3,4),(3,0,4)])


def spray(seed, tone):
    rng = random.Random(seed)
    g = Geometry()
    root = (0,0,0)
    end = (rng.uniform(-.05,.05),.12,1)
    shoot(g,[root,between(root,end,.52),end],[.021,.014,.006],4,tone)
    for row in range(2):
        for hand in [-1,1]:
            base = between(root,end,.24+row*.37+.04*(hand+1))
            delta = (hand*(.48-row*.085),rng.uniform(.08,.20),.22+row*.015)
            shoot(g,[base,add(base,delta)],[.010,.003],3,tone)
            for pair in range(2):
                for edge in [-1,1]:
                    tip = add(base,scale(delta,.26 + pair*.45 + .045*edge))
                    reach = .32 + rng.random()*.10
                    blade(g,tip,(hand*.6+.28*edge,rng.uniform(-.30,.28),.34+.62*edge),
                          reach*.67,reach,tone)
    return g


def spikes():
    g = Geometry()
    rng = random.Random(229)
    for i in range(4):
        angle = i*2.4
        root = (.095*math.sin(angle),0,.09*math.cos(angle))
        h = rng.uniform(.42,.70)
        tip = (root[0]*2.6,h,root[2]*2.8)
        shoot(g,[root,between(root,tip,.65),tip],[.008,.006,.003],3,"sage")
        for j in range(5):
            a = angle+j*2.4
            leaf = between(root,tip,.1+j*.11)
            blade(g,leaf,(math.cos(a),.36,math.sin(a)),.034,.16-j*.012,"sage")
        for j in range(4):
            a = angle+j*2.4
            p = between(root,tip,.73+j*.075)
            for k in range(3):
                b = a+k*TAU/3
                blade(g,p,(math.cos(b),.65,math.sin(b)),.028,.045-j*.003,"flower")
    return g


class Model:
    def __init__(self):
        self.meshes = []
        self.nodes = []

    def part(self, geometry):
        self.meshes.append(geometry)
        return len(self.meshes)-1

    def recolor(self, part, previous, tone):
        g = Geometry()
        g.groups = {(tone if key == previous else key): data for key, data in self.meshes[part].groups.items()}
        return self.part(g)

    def place(self, part, at, yaw=0, pitch=0, roll=0, size=(1,1,1)):
        # In shoot space +Z follows the branch and +Y faces its leaves upwards.
        # Compose rotations about local Y, -X and Z. TRS stores each repeated
        # sprig's rotation in four components and preserves orthogonal axes.
        sy, cy = math.sin(yaw/2), math.cos(yaw/2)
        sp, cp = math.sin(pitch/2), math.cos(pitch/2)
        sr, cr = math.sin(roll/2), math.cos(roll/2)
        q = (-cy*sp*cr + sy*cp*sr, sy*cp*cr + cy*sp*sr,
             sy*sp*cr + cy*cp*sr, cy*cp*cr - sy*sp*sr)
        self.nodes.append({"mesh":part, "translation":[round(v,4) for v in at],
                           "rotation":[round(v,6) for v in q], "scale":[round(v,4) for v in size]})

    def save(self, name, size):
        # The same GLB v2 chunk layout as bake_cape.py. Repeated sprigs refer
        # to one mesh; Warp's glTF importer merges all nodes for a single placement.
        blob, views, accessors, meshes, tones = bytearray(), [], [], [], []
        def accessor(rows, fmt, component, shape, bounds=False):
            blob.extend(bytes(-len(blob)%4))
            start = len(blob)
            item = struct.Struct("<"+fmt)
            for row in rows:
                blob.extend(item.pack(*row))
            views.append({"buffer":0,"byteOffset":start,"byteLength":len(blob)-start})
            a = {"bufferView":len(views)-1,"componentType":component,"count":len(rows),"type":shape}
            if bounds:
                a.update(min=list(map(min,zip(*rows))),max=list(map(max,zip(*rows))))
            accessors.append(a)
            return len(accessors)-1
        attributes = {}
        for geometry in self.meshes:
            prims = []
            for tone,(pts,norms,ix) in geometry.groups.items():
                if tone not in tones:
                    tones.append(tone)
                # Fixed-grid positions and quantized normals still use the standard
                # float attributes supported by the browser and native glTF loader.
                key = id(pts)
                if key not in attributes:
                    pos = [tuple(round(v*256)/256 for v in p) for p in pts]
                    normal = [tuple(round(v,6) for v in unit(tuple(round(c*16)/16 for c in n))) for n in norms]
                    attr = {"POSITION":accessor(pos,"3f",5126,"VEC3",True), "NORMAL":accessor(normal,"3f",5126,"VEC3")}
                    idx = accessor([(i,) for i in ix],"H",5123,"SCALAR")
                    attributes[key] = attr,idx
                attr,idx = attributes[key]
                prims.append({"attributes":attr,"indices":idx,"material":tones.index(tone)})
            meshes.append({"primitives":prims})
        mats = [{"name":t,"doubleSided":t in ("sun leaf","deep leaf","sage","flower"),
                 "pbrMetallicRoughness":{"baseColorFactor":[*TONES[t],1],"roughnessFactor":.91,"metallicFactor":0}}
                for t in tones]
        doc = {"asset":{"version":"2.0","generator":"Warp bake_plants.py"},"scene":0,
               "scenes":[{"nodes":[len(self.nodes)]}],
               "nodes":[*self.nodes,{"children":list(range(len(self.nodes))),"scale":size}],"meshes":meshes,
               "bufferViews":views,"accessors":accessors,"materials":mats,"buffers":[{"byteLength":len(blob)}]}
        head = json.dumps(doc,separators=(",",":")).encode()
        head += b" " * (-len(head)%4)
        blob.extend(bytes(-len(blob)%4))
        chunks = struct.pack("<I4s",len(head),b"JSON")+head+struct.pack("<I4s",len(blob),b"BIN\0")+blob
        (OUT/(name+".glb")).write_bytes(struct.pack("<4sII",b"glTF",2,12+len(chunks))+chunks)


def tree():
    m, wood = Model(), Geometry()
    soil(wood,1.12,1.12)
    trunk = [(0,0,0),(.04,1.15,-.03),(-.06,2.18,-.06),(.02,2.95,.04),(.1,3.7,.12),
             (.02,4.38,.09),(-.16,5.1,.02),(-.18,5.70,-.10),(-.13,6.16,-.12)]
    shoot(wood,trunk,[.21,.17,.14,.12,.10,.075,.052,.034,.018],8)
    m.place(m.part(wood),(0,0,0))
    leaf = m.part(spray(35,"deep leaf"))
    foliage = [leaf,m.recolor(leaf,"deep leaf","sun leaf")]
    rng = random.Random(1958)
    for i in range(11):
        base = trunk[2+i//2]
        a = .26+i*2.4
        spread = [.95,1.10,1.32,1.04,1.32,1.13,1.16,.93,1.08,.67,.57][i]
        arm = (math.sin(a),0,math.cos(a))
        fork = add(add(base,scale(arm,spread*.43)),(0,.42,0))
        tip = add(add(base,scale(arm,spread)),(0,rng.uniform(.82,1.27),0))
        shoot(wood,[base,fork,tip],[max(.031,.098-i*.006),.053,.017],6)
        for j in range(6):
            direction = a+(j-2.5)*.56+rng.uniform(-.16,.16)
            at = between(fork,tip,.20 + .74*((j*3)%7)/6)
            size = rng.uniform(.78,1.05)
            m.place(foliage[(i+j//2)%2],at,direction,rng.uniform(-.08,.72),rng.uniform(-.85,.85),
                    (size*1.18,size,size*1.13))
    return m


def bed():
    m, roots = Model(), Geometry()
    soil(roots,3.72,1.54)
    m.place(m.part(roots),(0,0,0))
    leaf = m.part(spray(35,"deep leaf"))
    parts = [leaf,m.recolor(leaf,"deep leaf","sage")]
    flowers = m.part(spikes())
    rng = random.Random(2319)
    for j,(x,z,h) in enumerate([(-1.38,.12,.58),(-.80,-.19,.71),(-.16,.19,.68),(.45,-.20,.73),(1.22,.12,.68)]):
        for i in range(7):
            a = i*2.4+j*.67
            pitch = .37 + .49*(i%3)
            size = rng.uniform(.61,.77)
            at = (x+.09*math.sin(a),.105+h*(i%3)*.105,z+.10*math.cos(a))
            m.place(parts[j%2],at,a,pitch,rng.uniform(-.53,.53),(size*.79,size,size))
        if j%2 == 0:
            m.place(flowers,(x+.17,.044,z+.14),j*.75,size=(.84,1.30,.84))
    return m


if __name__ == "__main__":
    OUT.mkdir(parents=True,exist_ok=True)
    tree().save("street-tree",(.90,.875,.90))
    bed().save("shrub-bed",(1,.80,.84))