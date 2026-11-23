"""Bake the grain elevator beside Exchange Avenue. Run with --check to verify the mesh."""
import argparse
import gzip
import math
from pathlib import Path

from bake_street import Group, material, unit, cross
from mesh_glb import glb


def grain_yard():
    materials = [
        material("Galvanized sheet", (.40,.46,.45), .51, .42),
        material("Oxide red", (.34,.094,.048), .69, .15),
        material("Painted steel", (.080,.113,.112), .62, .3),
        material("Safety yellow", (.71,.41,.069), .58, .12),
        material("Concrete", (.31,.30,.26), .92, 0),
        material("Crushed stone", (.26,.23,.18), .96, 0),
    ]
    groups = [Group() for _ in materials]
    sheet,red,steel,yellow,concrete,gravel = groups

    def beam(g,a,b,width,depth=None):
        depth = depth or width
        along = unit(tuple(v-u for u,v in zip(a,b)))
        side = unit(cross(along,(0,1,0) if abs(along[1])<.95 else (0,0,1)))
        up = cross(along,side)
        def section(point):
            return [tuple(p+i*s*width/2+j*u*depth/2 for p,s,u in zip(point,side,up))
                    for i,j in ((-1,-1),(1,-1),(1,1),(-1,1))]
        start,end = section(a),section(b)
        g.face(start[::-1])
        g.face(end)
        for i in range(4):
            k=(i+1)%4
            g.face([start[i],start[k],end[k],end[i]])

    def drum(cx,cz,r,h):
        sides=12
        def ring(y,radius):
            return [(cx+radius*math.cos(i*math.tau/sides),y,cz+radius*math.sin(i*math.tau/sides)) for i in range(sides)]
        profile=[(.04,r+.22,concrete),(.37,r+.22,concrete),(.41,r,sheet)]
        for i in range(1,int(h),2):
            profile.extend([(i-.045,r,sheet),(i,r+.07,sheet),(i+.045,r,sheet)])
        profile.extend([(h,r,sheet),(h+.13,r+.18,red),(h+1.65,.38,red),(h+1.96,.38,steel)])
        for (y0,r0,_),(y1,r1,g) in zip(profile,profile[1:]):
            lo,hi=ring(y0,r0),ring(y1,r1)
            normals=[unit((math.cos(i*math.tau/sides),(r0-r1)/(y1-y0),math.sin(i*math.tau/sides))) for i in range(sides)]
            for i in range(sides):
                k=(i+1)%sides
                g.face([lo[i],hi[i],hi[k],lo[k]],[normals[i],normals[i],normals[k],normals[k]])
        steel.face(ring(h+1.96,.38)[::-1])
        # Ladder standoffs keep its shadow distinct from the sheet wall.
        for dx in (-.28,.28):
            beam(yellow,(cx+dx,.5,cz-r-.20),(cx+dx,h+.45,cz-r-.20),.065)
        for i in range(2,int((h+.4)/.40)):
            beam(steel,(cx-.31,i*.40,cz-r-.23),(cx+.31,i*.40,cz-r-.23),.045)

    for cx,h in [(-4.9,8.8),(4.9,7.6)]:
        drum(cx,0,3.65,h)
    # Four-column bucket elevator and the overhead feed spouts.
    for x in (-.94,.94):
        for z in (3.9,5.8): beam(steel,(x,.05,z),(x,14.6,z),.23)
    for y in (1,4,7,10,13):
        for x in (-.94,.94): beam(steel,(x,y,3.9),(x,y+2.6,5.8),.12)
        beam(steel,(-.94,y,3.9),(.94,y+2.6,3.9),.12)
    beam(red,(0,13.6,4.85),(0,15.25,4.85),2.55,2.7)
    for x,h in [(-4.9,10.32),(4.9,9.1)]:
        beam(sheet,(0,13.65,4.75),(x,h,0),.39)
    gravel.face([(-13,.016,-12),(-12,.016,10),(12,.016,10),(13,.016,-12)])
    gravel.face([(-5,.016,-12),(7,.016,-12),(5,.016,-21),(-2,.016,-21)])
    return glb({m['name']:g for m,g in zip(materials,groups)},materials,"Port Meridian grain yard")


if __name__ == "__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check",action="store_true")
    args=parser.parse_args()
    path=Path(__file__).resolve().parents[1]/"assets"/"grain-yard.glb"
    mesh=grain_yard()
    size=len(gzip.compress(mesh,mtime=0))
    assert size<=16*1024, f"Grain yard exceeds 16 KiB compressed: {size}"
    if args.check:
        if path.read_bytes()!=mesh: raise SystemExit("Regenerate grain-yard.glb")
    else: path.write_bytes(mesh)
    print(f"Grain yard: {size} gzip bytes")
