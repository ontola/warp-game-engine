"""Bake Port Meridian's western land, joined road and matching Luau coordinates.
Run with Python 3 from any directory. --check verifies the checked-in pair.
"""
import argparse
from collections import defaultdict
from dataclasses import dataclass, field
import gzip
import json
import math
from pathlib import Path
import struct

ROOT = Path(__file__).resolve().parents[1]
# Walkable bank top in metres (x, south, north). The city plate starts at x=-178.
COAST = [
    (-1210, -18, 10), (-1203, -44, 38), (-1190, -67, 62),
    (-1168, -98, 88), (-1140, -142, 102), (-1110, -183, 130),
    (-1074, -202, 176), (-1038, -195, 224), (-1002, -185, 255),
    (-966, -202, 262), (-930, -235, 253), (-894, -279, 238),
    (-858, -310, 230), (-820, -318, 241), (-782, -297, 272),
    (-746, -268, 306), (-710, -250, 324), (-672, -249, 327),
    (-634, -269, 302), (-598, -300, 263), (-562, -317, 225),
    (-526, -310, 206), (-490, -282, 213), (-454, -248, 232),
    (-418, -227, 251), (-382, -224, 256), (-346, -233, 242),
    (-310, -238, 218), (-276, -224, 192), (-242, -201, 179),
    (-209, -183, 178), (-178, -178, 178),
]
JUNCTION = (-330, 0)
CONNECTOR = [(-178, 0), JUNCTION]
LOOP = [JUNCTION, (-490,-72), (-675,-127), (-870,-145), (-1030,-78),
        (-1100,0), (-1010,110), (-826,145), (-649,110), (-475,62), JUNCTION]
UP = (0, 1, 0)
# Linear base colours. No textures, metallic reflections or per-plant materials.
COLORS = {
    'Heath': (.143,.193,.093), 'Grassy hollows': (.130,.182,.091),
    'Rough pasture': (.155,.200,.105), 'Dry grass': (.174,.208,.118),
    'Dunes': (.32,.295,.204), 'Wet sand': (.245,.230,.175),
    'Weathered rock': (.210,.212,.176), 'Scree': (.266,.262,.196),
    'Gravel verge': (.209,.198,.159), 'Asphalt': (.119,.130,.131),
    'Edge paint': (.557,.556,.438), 'Lane paint': (.544,.445,.248),
}


def unit(x, z):
    length = math.hypot(x,z)
    return x/length, z/length


def mix(a, b, t):
    return tuple(x+(y-x)*t for x,y in zip(a,b))


def rounded_loop():
    route = [JUNCTION]
    for before, corner, after in zip(LOOP, LOOP[1:], LOOP[2:]):
        distance = min(23, math.dist(before,corner)*.18, math.dist(corner,after)*.18)
        entry = mix(corner,before,distance/math.dist(corner,before))
        exit_point = mix(corner,after,distance/math.dist(corner,after))
        for k in range(4):
            t = k/3
            point = mix(mix(entry,corner,t),mix(corner,exit_point,t),t)
            route.append(tuple(round(v,2) for v in point))
    return route+[JUNCTION]


def bank_bounds(x):
    for (x0, low, high),(x1, low1, high1) in zip(COAST,COAST[1:]):
        if x0 <= x <= x1:
            return mix((low,high),(low1,high1),(x-x0)/(x1-x0))
    raise ValueError(f'Outside the cape at x={x}')


@dataclass
class Group:
    points: list = field(default_factory=list)
    normals: list = field(default_factory=list)
    indices: list = field(default_factory=list)
    known: dict = field(default_factory=dict)

    def vertex(self, point, normal):
        # Grid snapping shares contour edges; finer road coordinates retain narrow lines.
        p = (round(point[0]*32)/32, round(point[1]*1024)/1024, round(point[2]*32)/32)
        n = tuple(round(a,2) for a in normal)
        if (p,n) not in self.known:
            self.known[p,n] = len(self.points)
            self.points.append(p)
            self.normals.append(n)
        return self.known[p,n]

    def polygon(self, points, normals=None):
        ids = [self.vertex(p,n) for p,n in zip(points,normals or [UP]*len(points))]
        for a,b in zip(ids[1:],ids[2:]):
            i,j,k = ids[0],a,b
            x,y,z = (self.points[t] for t in (i,j,k))
            area = (y[2]-x[2])*(z[0]-x[0])-(y[0]-x[0])*(z[2]-x[2])
            if abs(area) < 1e-7:
                continue
            self.indices.extend((i,j,k) if area>0 else (i,k,j))


def clip(poly, value, below):
    """Clip a ground polygon in scalar-field space; neighbouring cells share the crossing."""
    result = []
    for a,b in zip(poly,poly[1:]+poly[:1]):
        keep_a, keep_b = (a[2] <= value, b[2] <= value) if below else (a[2] >= value, b[2] >= value)
        if keep_a: result.append(a)
        if keep_a != keep_b: result.append(mix(a,b,(value-a[2])/(b[2]-a[2])))
    return result


def ground(mesh):
    rows = []
    for x,lo,hi in COAST:
        row = []
        for k in range(5):
            z = lo+(hi-lo)*k/4
            # Long, irregular bands of pasture follow the prevailing wind. Contours
            # are baked into planar geometry, so they need neither textures nor decals.
            h = math.sin(x*.012+z*.016)+.62*math.sin(z*.031-x*.008)+.39*math.cos(x*.036+z*.006)
            row.append((x,z,h))
        rows.append(row)
    bounds = [-3, -.62, .56, 3]
    for a,b in zip(rows,rows[1:]):
        for k in range(len(a)-1):
            for tri in ([a[k],a[k+1],b[k]], [a[k+1],b[k+1],b[k]]):
                for lo,hi,color in zip(bounds,bounds[1:],('Grassy hollows','Heath','Rough pasture')):
                    patch = clip(clip(tri,lo,False),hi,True)
                    if len(patch)>=3:
                        mesh[color].polygon([(x,0,z) for x,z,_ in patch])

    # Open bank line: the east cut meets the city directly, with no wall across the seam.
    coast = [(x,hi) for x,lo,hi in reversed(COAST)]+[(x,lo) for x,lo,hi in COAST]
    strips, norms, rocky = [], [], []
    for i,(x,z) in enumerate(coast):
        x0,z0 = coast[max(0,i-1)]
        x1,z1 = coast[min(len(coast)-1,i+1)]
        nx,nz = unit(z1-z0, x0-x1)
        # A narrow dark shelf around bluffs, longer pale sweeps in the dunes.
        bluff = (.5+.5*math.sin(x*.013+z*.019))**2
        reach = 12+(1-bluff)*15 + 2*math.sin(x*.077)
        rings = [(0,0),(.22,-.36-.30*bluff),(.58,-1.85-.6*bluff),(1,-5.7)]
        strips.append([(min(-178,x+nx*reach*r),y,z+nz*reach*r) for r,y in rings])
        normals = []
        for slope in (.10,.19,.28,.32):
            length=math.sqrt(1+slope*slope)
            normals.append((nx*slope/length,1/length,nz*slope/length))
        norms.append(normals)
        rocky.append(bluff)
    for i in range(len(coast)-1):
        rock = (rocky[i]+rocky[i+1])*.5 > .65
        tones = ('Scree','Weathered rock','Weathered rock') if rock else ('Dry grass','Dunes','Wet sand')
        for j,tone in enumerate(tones):
            corners = [(i,j),(i+1,j),(i+1,j+1),(i,j+1)]
            mesh[tone].polygon([strips[n][r] for n,r in corners],[norms[n][r] for n,r in corners])


def section(path, i, offsets, height):
    # Intersect the two offset lines. The chosen curve samples make short mitres.
    prev, curr, nxt = path[max(i-1,0)],path[i],path[min(i+1,len(path)-1)]
    back = unit(curr[0]-prev[0],curr[1]-prev[1]) if i else unit(nxt[0]-curr[0],nxt[1]-curr[1])
    ahead = unit(nxt[0]-curr[0],nxt[1]-curr[1]) if i+1<len(path) else back
    tx,tz = unit(back[0]+ahead[0],back[1]+ahead[1])
    factor = 1/(tx*back[0]+tz*back[1])
    return [(curr[0]-tz*width*factor,height,curr[1]+tx*width*factor) for width in offsets]


def road(mesh, connector, loop):
    # Trim the three arms where they meet, then fill their shared six-sided junction.
    # This leaves no double layers of asphalt or painted lines across the intersection.
    def away(p):
        return mix(JUNCTION,p,17/math.dist(JUNCTION,p))
    arms = [connector[:-1]+[away(connector[-2])], [away(loop[1]),*loop[1:-1],away(loop[-2])]]
    junction_edges = []
    arm_index = 0
    for path in arms:
        offsets = [-5.10,-4.5,-4.09,-3.97,3.97,4.09,4.5,5.10]
        cross = [section(path,i,offsets,.039) for i in range(len(path))]
        for a,b in zip(cross,cross[1:]):
            for j,tone in enumerate(('Gravel verge','Asphalt','Edge paint','Asphalt','Edge paint','Asphalt','Gravel verge')):
                mesh[tone].polygon([a[j],a[j+1],b[j+1],b[j]])
        ends = [cross[-1]] if path is arms[0] else [cross[0],cross[-1]]
        for end in ends:
            junction_edges.extend([(arm_index,end[1],end[0]),(arm_index,end[-2],end[-1])])
            arm_index += 1
        # Carry the dashed phase across arc samples. Split at each bend instead of
        # placing little independent mesh instances along the route.
        distance = 0
        for a,b in zip(path,path[1:]):
            length = math.dist(a,b)
            tx,tz = unit(b[0]-a[0],b[1]-a[1])
            start = math.floor(distance/17)*17
            while start < distance+length:
                lo,hi = max(start,distance),min(start+4.5,distance+length)
                if hi>lo:
                    line = []
                    for t,k in ((lo,-1),(lo,1),(hi,1),(hi,-1)):
                        along = t-distance
                        line.append((a[0]+tx*along-tz*.088*k,.055,a[1]+tz*along+tx*.088*k))
                    mesh['Lane paint'].polygon(line)
                start += 17
            distance += length
    junction_edges.sort(key=lambda edge: math.atan2(edge[1][2]-JUNCTION[1],edge[1][0]-JUNCTION[0]))
    # Endpoints on different arms outline the exposed gravel outside the junction.
    for (arm,a,oa),(other,b,ob) in zip(junction_edges,junction_edges[1:]+junction_edges[:1]):
        if arm != other:
            mesh['Gravel verge'].polygon([oa,ob,b,a])
        mesh['Asphalt'].polygon([(JUNCTION[0],.039,JUNCTION[1]),a,b])



def glb(groups):
    data, views, accs, mats, prims = bytearray(), [], [], [], []
    def accessor(values,fmt,component,kind,bounds=False):
        data.extend(bytes(-len(data)%4))
        offset = len(data)
        pack = struct.Struct('<'+fmt).pack
        for v in values: data.extend(pack(*v))
        views.append({'buffer':0,'byteOffset':offset,'byteLength':len(data)-offset})
        acc = {'bufferView':len(views)-1,'componentType':component,'count':len(values),'type':kind}
        if bounds: acc.update(min=list(map(min,zip(*values))),max=list(map(max,zip(*values))))
        accs.append(acc)
        return len(accs)-1
    for name,color in COLORS.items():
        group=groups[name]
        assert group.indices
        mats.append({'name':name,'pbrMetallicRoughness':{'baseColorFactor':[*color,1],'roughnessFactor':1,'metallicFactor':0}})
        prims.append({'material':len(mats)-1,'attributes':{
            'POSITION':accessor(group.points,'3f',5126,'VEC3',True),'NORMAL':accessor(group.normals,'3f',5126,'VEC3')},
            'indices':accessor([(i,) for i in group.indices],'H',5123,'SCALAR')})
    doc={'asset':{'version':'2.0','generator':'Port Meridian west cape'},'scene':0,'scenes':[{'nodes':[0]}],
         'nodes':[{'mesh':0}],'meshes':[{'primitives':prims}],'buffers':[{'byteLength':len(data)}],
         'materials':mats,'bufferViews':views,'accessors':accs}
    desc=json.dumps(doc,separators=(',',':')).encode()
    desc+=b' '*(-len(desc)%4)
    data.extend(bytes(-len(data)%4))
    chunks=struct.pack('<I4s',len(desc),b'JSON')+desc+struct.pack('<I4s',len(data),b'BIN\0')+data
    return struct.pack('<4sII',b'glTF',2,12+len(chunks))+chunks


def luau(paths):
    def row(values):
        return '{'+','.join(f'{n:g}' for n in values)+'}'
    lines=['-- Generated by scripts/bake_west_cape.py. Coordinates are world-space metres.',
           'return {','    coast = {',*('        '+row(r)+',' for r in COAST),'    },','    roads = {']
    for path in paths: lines.extend(['        {',*('            '+row(r)+',' for r in path),'        },'])
    return ('\n'.join([*lines,'    },','}'])+'\n').encode()


def bake(check):
    paths=[CONNECTOR,rounded_loop()]
    assert COAST[-1] == (-178,-178,178) and all(a[0]<b[0] for a,b in zip(COAST,COAST[1:]))
    for path in paths:
        for a,b in zip(path,path[1:]):
            for k in range(11):
                x,z=mix(a,b,k/10)
                lo,hi=bank_bounds(x)
                assert lo+5.1<=z<=hi-5.1
    assert paths[0][-1]==paths[1][0]==paths[1][-1]
    mesh=defaultdict(Group)
    ground(mesh)
    road(mesh,*paths)
    geometry=glb(mesh)
    wire=len(gzip.compress(geometry,compresslevel=6,mtime=0))
    assert wire<=20000, f'Coast download exceeds budget: {wire}'
    outputs={ROOT/'assets/west-cape.glb':geometry, ROOT/'west-cape-data.module.luau':luau(paths)}
    for file,content in outputs.items():
        if check:
            if file.read_bytes()!=content: raise ValueError(f'Run {Path(__file__).name} to update {file.name}')
        else: file.write_bytes(content)
    print(f'{len(COAST)} coast rows, {len(paths[1])} loop points, {sum(len(g.indices)//3 for g in mesh.values())} triangles')
    print(f'{len(geometry)} GLB bytes, {wire} gzip bytes; {len(outputs[ROOT/"west-cape-data.module.luau"])} Luau bytes')


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check',action='store_true')
    bake(parser.parse_args().check)
