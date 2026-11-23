"""Shared elevation triangles for the Mariner Wood crest and the switchback trail."""
import math


X, Z, SPACING, COLUMNS, ROWS = -1024, -160, 32, 20, 11


def generate():
    """Quantise metres once, then use the same samples in the GLB and in Warp."""
    samples = []
    for j in range(ROWS):
        for i in range(COLUMNS):
            x, z = X+i*SPACING, Z+j*SPACING
            edge = min(i, j, COLUMNS-1-i, ROWS-1-j)
            fade = min(1, edge/2)
            fade = fade*fade*(3-2*fade)
            hills = 0
            for cx, cz, wx, wz, h in [(-706,-64,109,62,37),(-865,66,82,77,42),
                                       (-561,42,67,55,15),(-950,-62,65,50,12)]:
                hills += h * math.exp(-((x-cx)/wx)**2 - ((z-cz)/wz)**2)
            samples.append(round(hills*fade*4)/4)
    # Limit the rise between samples in every compass direction. The resulting
    # grade is suitable for a road car on gravel, even at the short hairpins.
    graded = []
    for i in range(len(samples)):
        dst = min(h+8*math.hypot(i%COLUMNS-j%COLUMNS,i//COLUMNS-j//COLUMNS)
                  for j,h in enumerate(samples))
        graded.append(round(dst*4)/4)
    return graded


HEIGHTS = generate()


def cell(x,z):
    u, v = (x-X)/SPACING, (z-Z)/SPACING
    i, j = math.floor(u), math.floor(v)
    if not (0<=i<COLUMNS-1 and 0<=j<ROWS-1):
        return None
    return j*COLUMNS+i, u-i, v-j


def height(x,z):
    local = cell(x,z)
    if local is None:
        return 0.
    k, fx, fz = local
    a,b,c,d = HEIGHTS[k],HEIGHTS[k+1],HEIGHTS[k+COLUMNS],HEIGHTS[k+COLUMNS+1]
    if fx+fz<=1:
        return a+(b-a)*fx+(c-a)*fz
    return d+(c-d)*(1-fx)+(b-d)*(1-fz)


def smooth_normal(x,z):
    # Soft rolling ground lighting across the exact, coarser collision triangles.
    nx = (height(x-6,z)-height(x+6,z))/12
    nz = (height(x,z-6)-height(x,z+6))/12
    length = math.sqrt(nx*nx+1+nz*nz)
    return (nx/length,1/length,nz/length)


def split(poly, nx, nz, d):
    """Partition a convex polygon against a vertical plane with a shared edge."""
    dist = [nx*p[0]+nz*p[2]-d for p in poly]
    if min(dist)>=-1e-7 or max(dist)<=1e-7:
        return [poly]
    low, high = [], []
    for i,a in enumerate(poly):
        j=(i+1)%len(poly)
        da,db=dist[i],dist[j]
        if da<=1e-7: low.append(a)
        if da>=-1e-7: high.append(a)
        if (da < -1e-7 and db > 1e-7) or (da > 1e-7 and db < -1e-7):
            t=da/(da-db)
            p=tuple(x+(y-x)*t for x,y in zip(a,poly[j]))
            low.append(p)
            high.append(p)
    return [p for p in (low,high) if len(p)>=3]


def lift(poly):
    """Split roads, paint and terrain at the same cell edges and diagonals.

    Each resulting triangle lies on the collision surface with its original
    paint height. The flat coastline outside this inland patch stays untouched.
    """
    xmin,xmax=min(p[0] for p in poly),max(p[0] for p in poly)
    zmin,zmax=min(p[2] for p in poly),max(p[2] for p in poly)
    if xmax<=X or xmin>=X+(COLUMNS-1)*SPACING or zmax<=Z or zmin>=Z+(ROWS-1)*SPACING:
        yield poly, None
        return
    parts = [poly]
    for axis,minp,maxp,base,count in [(0,xmin,xmax,X,COLUMNS),(2,zmin,zmax,Z,ROWS)]:
        for i in range(max(0,math.ceil((minp-base)/SPACING)),min(count-1,math.floor((maxp-base)/SPACING))+1):
            parts = [q for p in parts for q in split(p,int(axis==0),int(axis==2),base+i*SPACING)]
    for part in parts:
        x=sum(p[0] for p in part)/len(part)
        z=sum(p[2] for p in part)/len(part)
        local=cell(x,z)
        if local is None:
            yield part, None
            continue
        k=local[0]
        diagonal=X+Z+(k%COLUMNS+k//COLUMNS+1)*SPACING
        for patch in split(part,1,1,diagonal):
            yield [(px,y+height(px,pz),pz) for px,y,pz in patch],[smooth_normal(px,pz) for px,_,pz in patch]
