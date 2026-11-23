"""Bake Port Meridian's western land, joined road and matching Luau coordinates.
Run with Python 3 from any directory. --check verifies the checked-in pair.
"""
import argparse
from collections import defaultdict
from dataclasses import dataclass, field
import gzip
import math
from pathlib import Path
from mesh_glb import glb
import cape_relief as relief
from cape_landcover import ground_material, LAND_COLORS

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
    **LAND_COLORS,
    'Forest gravel': (.215,.16,.103),
    'Edge paint': (.557,.556,.438), 'Lane paint': (.544,.445,.248),
}


def unit(x, z):
    length = math.hypot(x,z)
    return x/length, z/length


def mix(a, b, t):
    return tuple(x+(y-x)*t for x,y in zip(a,b))


def rounded_path(path, radius):
    route = [path[0]]
    for before, corner, after in zip(path, path[1:], path[2:]):
        distance = min(radius, math.dist(before,corner)*.18, math.dist(corner,after)*.18)
        entry = mix(corner,before,distance/math.dist(corner,before))
        exit_point = mix(corner,after,distance/math.dist(corner,after))
        for k in range(4):
            t = k/3
            point = mix(mix(entry,corner,t),mix(corner,exit_point,t),t)
            route.append(tuple(round(v,2) for v in point))
    return route+[path[-1]]


def on_road(path,x,z):
    def closest(a,b):
        dx,dz=b[0]-a[0],b[1]-a[1]
        t=max(0,min(1,((x-a[0])*dx+(z-a[1])*dz)/(dx*dx+dz*dz)))
        return mix(a,b,t)
    return min((closest(a,b) for a,b in zip(path,path[1:])),key=lambda p:math.dist(p,(x,z)))


def rally(loop):
    # Two trailheads join the paved loop. Offset bends climb the eastern shoulder
    # of Mariner Wood, switch back along the spine and descend the northern ridge.
    start=on_road(loop,-565,-94)
    end=on_road(loop,-821,144)
    return rounded_path([start,(-585,-68),(-622,-69),(-650,-42),(-711,-33),
        (-738,-52),(-697,-80),(-676,-104),(-696,-113),(-753,-94),
        (-789,-65),(-806,-23),(-855,-8),(-907,24),(-935,53),
        (-918,68),(-868,40),(-832,43),(-803,65),(-831,82),
        (-879,106),(-875,125),end],19)


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
    uvs: list = field(default_factory=list)

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
        if normals is not None:
            self.flat_polygon(points,normals)
        else:
            for patch,norms in relief.lift(points):
                self.flat_polygon(patch,norms)

    def flat_polygon(self, points, normals=None):
        ids = [self.vertex(p,n) for p,n in zip(points,normals or [UP]*len(points))]
        for a,b in zip(ids[1:],ids[2:]):
            i,j,k = ids[0],a,b
            x,y,z = (self.points[t] for t in (i,j,k))
            area = (y[2]-x[2])*(z[0]-x[0])-(y[0]-x[0])*(z[2]-x[2])
            if abs(area) < 1e-7:
                continue
            self.indices.extend((i,j,k) if area>0 else (i,k,j))


def ground(mesh):
    rows = [[(x,0,lo+(hi-lo)*j/6) for j in range(7)] for x,lo,hi in COAST]
    for a,b in zip(rows,rows[1:]):
        for k in range(len(a)-1):
            for tri in ([a[k],a[k+1],b[k]], [a[k+1],b[k+1],b[k]]):
                for patch,normals in relief.lift(tri):
                    x,y,z = (sum(p[k] for p in patch)/len(patch) for k in range(3))
                    nx,ny,nz = relief.smooth_normal(x,z)
                    material = ground_material(x,z,y,math.hypot(nx,nz)/ny)
                    mesh[material].flat_polygon(patch,normals)

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



def gravel(mesh,path):
    # A single ribbon carries signed lateral distance and chainage. The surface
    # shader feathers its shoulders into the heath and resolves individual stones.
    group = mesh['Forest gravel']
    cross = [section(path,i,[-3.24,3.24],.025) for i in range(len(path))]
    segments = []
    length = 0
    for i,(a,b) in enumerate(zip(path,path[1:])):
        group.polygon([cross[i][0],cross[i][1],cross[i+1][1],cross[i+1][0]])
        tx,tz=unit(b[0]-a[0],b[1]-a[1])
        distance=math.dist(a,b)
        segments.append((a[0],a[1],tx,tz,distance,length))
        length+=distance
    for x,_,z in group.points:
        best=math.inf
        for ax,az,tx,tz,distance,start in segments:
            u=max(0,min(distance,(x-ax)*tx+(z-az)*tz))
            dx,dz=x-ax-u*tx,z-az-u*tz
            d2=dx*dx+dz*dz
            if d2<best:
                best,across,along=d2,dz*tx-dx*tz,start+u
        group.uvs.append((.5+across/8,along/1024))


def luau(paths):
    def row(values):
        return '{'+','.join(f'{n:g}' for n in values)+'}'
    lines=['-- Generated by scripts/bake_west_cape.py. Coordinates are world-space metres.',
           'return {','    coast = {',*('        '+row(r)+',' for r in COAST),'    },','    roads = {']
    for path in paths: lines.extend(['        {',*('            '+row(r)+',' for r in path),'        },'])
    lines.extend(['    },',f'    terrain = {{x={relief.X},z={relief.Z},spacing={relief.SPACING},columns={relief.COLUMNS},rows={relief.ROWS},heights={{'])
    for j in range(relief.ROWS):
        samples = relief.HEIGHTS[j*relief.COLUMNS:(j+1)*relief.COLUMNS]
        lines.append('        '+','.join(f'{n:g}' for n in samples)+',')
    return ('\n'.join([*lines,'    }},','}'])+'\n').encode()


def bake(check):
    loop=rounded_path(LOOP,23)
    paths=[CONNECTOR,loop,rally(loop)]
    assert COAST[-1] == (-178,-178,178) and all(a[0]<b[0] for a,b in zip(COAST,COAST[1:]))
    for path in paths:
        for a,b in zip(path,path[1:]):
            for k in range(11):
                x,z=mix(a,b,k/10)
                lo,hi=bank_bounds(x)
                assert lo+5.1<=z<=hi-5.1
    assert paths[0][-1]==paths[1][0]==paths[1][-1]
    # Trailheads sit on the paved road and a gravel car can climb either way.
    for x,z in (paths[2][0],paths[2][-1]):
        assert math.dist((x,z),on_road(loop,x,z))<.001
    for a,b in zip(paths[2],paths[2][1:]):
        steps=math.ceil(math.dist(a,b)/2)
        for j in range(steps):
            p,q=mix(a,b,j/steps),mix(a,b,(j+1)/steps)
            assert abs(relief.height(*p)-relief.height(*q)) <= .30*math.dist(p,q)
    mesh=defaultdict(Group)
    ground(mesh)
    road(mesh,CONNECTOR,loop)
    gravel(mesh,paths[2])
    materials = [{'name':name,'pbrMetallicRoughness':{'baseColorFactor':[*color,1],'roughnessFactor':1,'metallicFactor':0}}
                 for name,color in COLORS.items() if mesh[name].indices]
    for material in materials:
        if material['name'] in LAND_COLORS or material['name'] in ('Grassy hollows','Rough pasture'):
            material['extras'] = {'warpSurface':40}
        elif material['name']=='Forest gravel':
            material['extras'] = {'warpSurface':41}
    geometry=glb(mesh,materials,'Port Meridian west cape')
    wire=len(gzip.compress(geometry,compresslevel=6,mtime=0))
    assert wire<=76*1024, f'Coast download exceeds budget: {wire}'
    outputs={ROOT/'assets/west-cape.glb':geometry, ROOT/'west-cape-data.module.luau':luau(paths)}
    for file,content in outputs.items():
        if check:
            if file.read_bytes()!=content: raise ValueError(f'Run {Path(__file__).name} to update {file.name}')
        else: file.write_bytes(content)
    print(f'{len(COAST)} coast rows, {len(loop)} loop points, {len(paths[2])} rally points, {sum(len(g.indices)//3 for g in mesh.values())} triangles')
    print(f'{len(geometry)} GLB bytes, {wire} gzip bytes; {len(outputs[ROOT/"west-cape-data.module.luau"])} Luau bytes')


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check',action='store_true')
    bake(parser.parse_args().check)
