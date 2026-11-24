"""Split scenery triangles against a terrain grid resolved by the game in Luau."""
import math


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


class Terrain:
    def __init__(self, data):
        self.x, self.z, self.spacing = data["x"], data["z"], data["spacing"]
        self.columns, self.rows, self.heights = data["columns"], data["rows"], data["heights"]

    def cell(self,x,z):
        u, v = (x-self.x)/self.spacing, (z-self.z)/self.spacing
        i, j = math.floor(u), math.floor(v)
        if not (0<=i<self.columns-1 and 0<=j<self.rows-1):
            return None
        return j*self.columns+i, u-i, v-j


    def height(self,x,z):
        local = self.cell(x,z)
        if local is None:
            return 0.
        k, fx, fz = local
        a,b,c,d = self.heights[k],self.heights[k+1],self.heights[k+self.columns],self.heights[k+self.columns+1]
        if fx+fz<=1:
            return a+(b-a)*fx+(c-a)*fz
        return d+(c-d)*(1-fx)+(b-d)*(1-fz)


    def smooth_normal(self,x,z):
        # Soft rolling ground lighting across the exact, coarser collision triangles.
        nx = (self.height(x-6,z)-self.height(x+6,z))/12
        nz = (self.height(x,z-6)-self.height(x,z+6))/12
        length = math.sqrt(nx*nx+1+nz*nz)
        return (nx/length,1/length,nz/length)


    def lift(self,poly):
        """Split roads, paint and terrain at the same cell edges and diagonals.

        Each resulting triangle lies on the collision surface with its original
        paint height. The flat coastline outside this inland patch stays untouched.
        """
        xmin,xmax=min(p[0] for p in poly),max(p[0] for p in poly)
        zmin,zmax=min(p[2] for p in poly),max(p[2] for p in poly)
        if xmax<=self.x or xmin>=self.x+(self.columns-1)*self.spacing or zmax<=self.z or zmin>=self.z+(self.rows-1)*self.spacing:
            yield poly, None
            return
        parts = [poly]
        for axis,minp,maxp,base,count in [(0,xmin,xmax,self.x,self.columns),(2,zmin,zmax,self.z,self.rows)]:
            for i in range(max(0,math.ceil((minp-base)/self.spacing)),min(count-1,math.floor((maxp-base)/self.spacing))+1):
                parts = [q for p in parts for q in split(p,int(axis==0),int(axis==2),base+i*self.spacing)]
        for part in parts:
            x=sum(p[0] for p in part)/len(part)
            z=sum(p[2] for p in part)/len(part)
            local=self.cell(x,z)
            if local is None:
                yield part, None
                continue
            k=local[0]
            diagonal=self.x+self.z+(k%self.columns+k//self.columns+1)*self.spacing
            for patch in split(part,1,1,diagonal):
                yield [(px,y+self.height(px,pz),pz) for px,y,pz in patch],[self.smooth_normal(px,pz) for px,_,pz in patch]
