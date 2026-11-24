"""Export the game's resolved Luau layout as a prebuilt GLB. --check verifies the packaged asset."""
import argparse
from collections import defaultdict
from dataclasses import dataclass, field
import gzip
import math
import json
import shutil
import subprocess
import tempfile
from pathlib import Path
from mesh_glb import glb
from cape_relief import Terrain
from cape_landcover import ground_material, LAND_COLORS

ROOT = Path(__file__).resolve().parents[1]
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


@dataclass
class Group:
    terrain: Terrain
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
            for patch,norms in self.terrain.lift(points):
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


def ground(mesh, layout, terrain):
    coast_rows = layout["coast"]
    rows = [[(x,0,lo+(hi-lo)*j/6) for j in range(7)] for x,lo,hi in coast_rows]
    for a,b in zip(rows,rows[1:]):
        for k in range(len(a)-1):
            for tri in ([a[k],a[k+1],b[k]], [a[k+1],b[k+1],b[k]]):
                for patch,normals in terrain.lift(tri):
                    x,y,z = (sum(p[k] for p in patch)/len(patch) for k in range(3))
                    nx,ny,nz = terrain.smooth_normal(x,z)
                    material = ground_material(x,z,y,math.hypot(nx,nz)/ny,layout)
                    mesh[material].flat_polygon(patch,normals)

    # Open bank line: the east cut meets the city directly, with no wall across the seam.
    coast = [(x,hi) for x,lo,hi in reversed(coast_rows)]+[(x,lo) for x,lo,hi in coast_rows]
    strips, norms, rocky = [], [], []
    for i,(x,z) in enumerate(coast):
        x0,z0 = coast[max(0,i-1)]
        x1,z1 = coast[min(len(coast)-1,i+1)]
        nx,nz = unit(z1-z0, x0-x1)
        # A narrow dark shelf around bluffs, longer pale sweeps in the dunes.
        bluff = (.5+.5*math.sin(x*.013+z*.019))**2
        reach = 12+(1-bluff)*15 + 2*math.sin(x*.077)
        rings = [(0,0),(.22,-.36-.30*bluff),(.58,-1.85-.6*bluff),(1,-5.7)]
        strips.append([(min(coast_rows[-1][0],x+nx*reach*r),y,z+nz*reach*r) for r,y in rings])
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


def road(mesh, connector, loop, width):
    junction = connector[-1]
    half = width/2
    # Trim the three arms where they meet, then fill their shared six-sided junction.
    # This leaves no double layers of asphalt or painted lines across the intersection.
    def away(p):
        return mix(junction,p,17/math.dist(junction,p))
    arms = [connector[:-1]+[away(connector[-2])], [away(loop[1]),*loop[1:-1],away(loop[-2])]]
    junction_edges = []
    arm_index = 0
    for path in arms:
        offsets = [-half-.6,-half,-half+.41,-half+.53,half-.53,half-.41,half,half+.6]
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
    junction_edges.sort(key=lambda edge: math.atan2(edge[1][2]-junction[1],edge[1][0]-junction[0]))
    # Endpoints on different arms outline the exposed gravel outside the junction.
    for (arm,a,oa),(other,b,ob) in zip(junction_edges,junction_edges[1:]+junction_edges[:1]):
        if arm != other:
            mesh['Gravel verge'].polygon([oa,ob,b,a])
        mesh['Asphalt'].polygon([(junction[0],.039,junction[1]),a,b])



def gravel(mesh,path,width):
    # A single ribbon carries signed lateral distance and chainage. The surface
    # shader feathers its shoulders into the heath and resolves individual stones.
    group = mesh['Forest gravel']
    cross = [section(path,i,[-width/2-.64,width/2+.64],.025) for i in range(len(path))]
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


def resolve_layout(warp):
    """Run the game's own Luau compiler, including ordinary language constructs."""
    if warp:
        command = [str(Path(warp).resolve())]
    elif shutil.which("warp"):
        command = ["warp"]
    else:
        command = ["cargo","run","--quiet","--locked","-p","warp","--bin","warp","--"]
    with tempfile.TemporaryDirectory(prefix="warp-cape-") as tmp:
        folder = Path(tmp)
        for name in ("west_cape_layout.module.luau", "west-cape-data.module.luau"):
            shutil.copyfile(ROOT/name, folder/name)
        (folder/"game.toml").write_text('title="Cape cook"\nsteps=3600000\n',encoding="utf-8")
        (folder/"main.luau").write_text(
            'local data=require("west-cape-data")\n'
            'return {init=function() state.layout=data end}\n', encoding="utf-8")
        result=subprocess.run([*command,"check",str(folder),"--ticks","0"],cwd=ROOT.parents[1],
                              text=True,stdout=subprocess.PIPE,check=True)
        payload = result.stdout[result.stdout.index("{"):]
        layout=json.loads(payload)["state"]["layout"]
        if "error" in layout:
            raise ValueError(layout["error"])
        return layout


def bake(check,warp=None):
    layout=resolve_layout(warp)
    terrain=Terrain(layout["terrain"])
    connector,loop,trail=layout["roads"]
    mesh=defaultdict(lambda: Group(terrain))
    ground(mesh,layout,terrain)
    road(mesh,connector,loop,layout["widths"]["asphalt"])
    gravel(mesh,trail,layout["widths"]["gravel"])
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
    outputs={ROOT/'assets/west-cape.glb':geometry, ROOT/'west-cape-bake.module.luau':(
        '-- Generated by scripts/bake_west_cape.py. Identifies the geometry in west-cape.glb.\n'
        f'return {{key="{layout["bakeKey"]}"}}\n').encode()}
    for file,content in outputs.items():
        if check:
            current = file.read_bytes() if file.suffix == ".glb" else file.read_text(encoding="utf-8").encode()
            if current != content: raise ValueError(f'Run {Path(__file__).name} to update {file.name}')
        else: file.write_bytes(content)
    print(f'{len(layout["coast"])} coast rows, {len(loop)} loop points, {len(trail)} rally points, {sum(len(g.indices)//3 for g in mesh.values())} triangles')
    print(f'{len(geometry)} GLB bytes, {wire} gzip bytes; geometry {layout["bakeKey"]}')


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check',action='store_true')
    parser.add_argument('--warp',help='path to a built Warp executable (default: warp on PATH or cargo run)')
    args=parser.parse_args()
    bake(args.check,args.warp)
