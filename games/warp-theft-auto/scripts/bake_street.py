"""Bake streetlights and litter bins in metres. Neither prop uses any textures."""
import argparse
import gzip
import math
from pathlib import Path

from mesh_glb import glb

ROOT = Path(__file__).resolve().parents[1] / 'assets' / 'street'


def material(name, color, rough, metal, surface=None):
    data = {'name': name, 'pbrMetallicRoughness': {
        'baseColorFactor': [*color, 1], 'roughnessFactor': rough, 'metallicFactor': metal}}
    if surface is not None:
        data['extras'] = {'warpSurface': surface}
    return data


def unit(a):
    length = math.hypot(*a)
    return tuple(x / length for x in a)


def cross(a, b):
    return (a[1]*b[2] - a[2]*b[1], a[2]*b[0] - a[0]*b[2], a[0]*b[1] - a[1]*b[0])


class Group:
    def __init__(self):
        self.points, self.normals, self.indices, self.vertices = [], [], [], {}

    def face(self, points, normals=None):
        if normals is None:
            a, b, c = points[:3]
            normal = unit(cross(tuple(y-x for x,y in zip(a,b)), tuple(z-x for x,z in zip(a,c))))
            normals = [normal] * len(points)
        local = []
        for point, normal in zip(points, normals):
            point = tuple(round(c, 4) for c in point)
            normal = tuple(round(c, 6) for c in normal)
            key = (point, normal)
            if key not in self.vertices:
                self.vertices[key] = len(self.points)
                self.points.append(point)
                self.normals.append(normal)
            local.append(self.vertices[key])
        for j in range(1, len(local)-1):
            self.indices.extend((local[0], local[j], local[j+1]))


def ring(y, rx, rz, sides, z=0):
    return [(rx*math.cos(i*math.tau/sides), y, z+rz*math.sin(i*math.tau/sides)) for i in range(sides)]


def bands(group, profile, sides, center_z=0):
    """Revolve consecutive elliptical rings; inward rings form top rims with an open throat."""
    for (y0, x0, z0), (y1, x1, z1) in zip(profile, profile[1:]):
        low = ring(y0, x0, z0, sides, center_z)
        high = ring(y1, x1, z1, sides, center_z)
        normals = []
        for i in range(sides):
            a = i * math.tau / sides
            edge = tuple(b-a for a,b in zip(low[i], high[i]))
            tangent = (-(x0+x1)*math.sin(a), 0, (z0+z1)*math.cos(a))
            normals.append(unit(cross(edge, tangent)))
        for i in range(sides):
            j = (i+1) % sides
            group.face([low[i], high[i], high[j], low[j]], [normals[i], normals[i], normals[j], normals[j]])


def lamp():
    materials = [material('Oxidised plinth', (.052,.069,.073), .64, .35),
                 material('Enamel grey', (.20,.26,.26), .44, .42),
                 material('Opal diffuser', (.83,.78,.65), .55, 0, surface=31)]
    base, steel, lens = (Group() for _ in materials)
    bands(base, [(0,.145,.145),(.06,.145,.145),(.11,.116,.116),(.53,.105,.105),(.60,.080,.080)], 8)
    # Tapered shaft bends forward above the walking clearance, terminating inside the head.
    spine = [(0,.36,.082),(0,1.12,.077),(0,4.47,.061),(.055,4.85,.059),
             (.22,5.12,.055),(.50,5.32,.05),(.9,5.43,.045),(1.60,5.44,.040)]
    rings = []
    for i,(z,y,r) in enumerate(spine):
        below, above = spine[max(0,i-1)], spine[min(i+1,len(spine)-1)]
        dz, dy = unit((above[0]-below[0], above[1]-below[1]))
        dirs = [(math.cos(j*math.tau/8), -dz*math.sin(j*math.tau/8), dy*math.sin(j*math.tau/8)) for j in range(8)]
        rings.append(([(r*x,y+r*b,z+r*c) for x,b,c in dirs], dirs))
    for (a,an), (b,bn) in zip(rings,rings[1:]):
        for i in range(8):
            j=(i+1)%8
            steel.face([a[i],b[i],b[j],a[j]], [an[i],bn[i],bn[j],an[j]])
    # A cast cobra head with a rolled edge. The large downward-facing opal lens is surface 31.
    bands(steel, [(5.26,.16,.32),(5.28,.199,.375),(5.36,.205,.39),(5.445,.176,.34),(5.48,.103,.20)], 10, 1.87)
    steel.face(ring(5.48,.103,.20,10,1.87)[::-1])
    bands(lens, [(5.23,.144,.285),(5.26,.16,.32)], 10, 1.87)
    lens.face(ring(5.23,.144,.285,10,1.87))
    return materials, [base,steel,lens]


def bin_model():
    materials = [material('Cast foot and liner', (.018,.027,.027), .83, .12),
                 material('Port green enamel', (.048,.16,.149), .52, .24),
                 material('Brushed alloy rim', (.35,.39,.36), .51, .54)]
    dark, enamel, alloy = (Group() for _ in materials)
    bands(dark, [(0,.200,.200),(.055,.232,.232),(.10,.232,.232),(.14,.215,.215),(.73,.215,.215)], 12)
    # The liner is visible through the gaps between the pressed ribs and inside the lid.
    dark.face(ring(.73,.215,.215,12)[::-1])
    ribs = 14
    for i in range(ribs):
        angle = math.tau*i/ribs
        def edge(offset,y,r):
            a=angle+offset*math.tau/ribs
            return (r*math.cos(a), y, r*math.sin(a))
        # A folded section has two narrow bevels and a broad face.
        widths = [(-.43,.224),(-.34,.244),(.34,.244),(.43,.224)]
        for (a,ra),(b,rb) in zip(widths,widths[1:]):
            enamel.face([edge(a,.105,ra),edge(a,.727,ra+.012),edge(b,.727,rb+.012),edge(b,.105,rb)])
    bands(enamel, [(.71,.252,.252),(.74,.268,.268),(.78,.268,.268),(.803,.251,.251)], 14)
    bands(alloy, [(.80,.25,.25),(.827,.268,.268),(.850,.27,.27),(.880,.249,.249),
                  (.880,.164,.164),(.862,.148,.148)], 14)
    bands(dark, [(.862,.148,.148),(.731,.143,.143)], 14)
    return materials, [dark,enamel,alloy]


def main():
    args=argparse.ArgumentParser(description=__doc__)
    args.add_argument('--check', action='store_true', help='verify the checked-in meshes')
    check=args.parse_args().check
    if not check:
        ROOT.mkdir(parents=True, exist_ok=True)
    for name, create in [('lamp', lamp), ('bin', bin_model)]:
        materials, groups = create()
        blob = glb({m['name']: g for m,g in zip(materials,groups)}, materials, 'Port Meridian street furniture')
        path = ROOT / (name + '.glb')
        if check:
            if path.read_bytes() != blob:
                raise SystemExit(f'{name}: generated mesh differs')
        else:
            path.write_bytes(blob)
        print(f'{name}: {sum(len(g.indices)//3 for g in groups)} triangles, {len(blob)} bytes, {len(gzip.compress(blob,mtime=0))} gzipped')


if __name__ == '__main__':
    main()
