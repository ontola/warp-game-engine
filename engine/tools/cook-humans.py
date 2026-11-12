"""Cook the Warp Humans close mesh from the CC0 hm08 data and system pack.
Pass the base OBJ, directory of rig/target files and extracted system pack,
and destination directory. Requires Python, NumPy and Pillow.
"""
import argparse
import json
import re
import struct
from pathlib import Path

import numpy as np
from PIL import Image


def obj(path):
    positions, texcoords, faces = [], [], []
    group = ""
    for line in path.read_text(encoding="utf8", errors="replace").splitlines():
        w = line.split()
        if not w:
            continue
        if w[0] == "v":
            positions.append(list(map(float, w[1:4])))
        elif w[0] == "vt":
            u,v = map(float,w[1:3])
            texcoords.append((u,1-v))
        elif w[0] == "g":
            group = w[1]
        elif w[0] == "f":
            corners = [tuple(int(i)-1 for i in x.split("/")[:2]) for x in w[1:]]
            faces.extend((group, (corners[0], corners[j], corners[j+1])) for j in range(1,len(corners)-1))
    return np.array(positions), np.array(texcoords), faces


def fit(path, base):
    lines=path.read_text().splitlines()
    scales=np.ones(3)
    for axis,letter in enumerate("xyz"):
        for line in lines:
            if line.startswith(letter+"_scale "):
                _,a,b,length=line.split()
                scales[axis]=abs(base[int(a),axis]-base[int(b),axis])/float(length)
    refs=[]; weights=[]; offsets=[]
    for line in lines[next(i for i,l in enumerate(lines) if l.startswith("verts "))+1:]:
        w=line.split()
        if not w: continue
        if not w[0].isdigit():
            if refs: break
            continue
        if len(w)==1:
            refs.append([int(w[0])]*3);weights.append([1.,0.,0.]);offsets.append([0.]*3)
        else:
            refs.append(list(map(int,w[:3])));weights.append(list(map(float,w[3:6])));offsets.append(list(map(float,w[6:9])))
    return np.array(refs),np.array(weights),np.array(offsets)*scales


def hidden(path):
    text=path.read_text().split("delete_verts")
    result=set()
    if len(text)==2:
        for a,b in re.findall(r"(\d+)(?:\s*-\s*(\d+))?",text[1]):
            result.update(range(int(a),int(b or a)+1))
    return result


def curl_tile(tile):
    # A seamless fibre field for the close coiled groom. Preserve the source
    # highlight calibration (27/255), so every seeded hair tint still applies.
    from PIL import ImageDraw
    rng=np.random.default_rng(8108)
    size=tile//2
    field=Image.new("L",(size,size),11)
    draw=ImageDraw.Draw(field)
    for _ in range(5200):
        x,y=rng.uniform(0,size,2)
        rx,ry=rng.uniform(1.2,4.2,2)
        start=rng.integers(360).item()
        light=int(rng.integers(4,35))
        for dx in (-size,0,size):
            for dy in (-size,0,size):
                box=(x+dx-rx,y+dy-ry,x+dx+rx,y+dy+ry)
                if box[2]<0 or box[3]<0 or box[0]>=size or box[1]>=size: continue
                draw.arc(box,start,start+235,fill=light,width=1)
    result=Image.new("RGBA",(tile,tile))
    for x in (0,size):
        for y in (0,size):result.paste(field.convert("RGBA"),(x,y))
    return result


def groom_edges(fitted,mat,atlas):
    # Overlap tapered locks along the actual outline of each fitted groom.
    # Root and tip share their nearest donor's morphs, so they stay attached.
    xyz=fitted*.06+[0,.49,-.021]
    angle=np.arctan2(xyz[:,0],xyz[:,2]-.002)
    rng=np.random.default_rng(mat)
    verts=[]; uvs=[]; roots=[]; tris=[]
    for theta in np.linspace(-2.75,2.75,99):
        nearby=np.where(np.abs(angle-theta)<.075)[0]
        if not len(nearby): continue
        i=nearby[np.argmin(xyz[nearby,1])]
        outward=np.array([np.sin(theta),0,np.cos(theta)])
        across=np.array([np.cos(theta),0,-np.sin(theta)])
        root=xyz[i]+outward*.0007
        width=rng.uniform(.0008,.0025)
        fall=rng.uniform(.0015,.004)
        tip=root+[0,-fall,0]+across*width*rng.uniform(-1.3,1.3)
        first=len(verts)
        for point,tex in [(root+[0,.0045,0]-width*across,(.4,.4)),(root+[0,.0045,0]+width*across,(.6,.4)),(tip,(.51,.58))]:
            verts.append((point-[0,.49,-.021])/.06);uvs.append(tex);roots.append(i)
        tris.append(("",[(first,first),(first+2,first+2),(first+1,first+1)]))
    # A recessed crown blocks the view through stacked cutout cards.
    # Sample an opaque patch of the same groom so this joins its draw.
    tile=atlas.height//2
    atlas_x,atlas_y=(mat-17)%4*tile,(mat-17)//4*tile
    tex=np.asarray(atlas.crop((atlas_x,atlas_y,atlas_x+tile,atlas_y+tile)))
    patches=[(u,v) for u in range(20,tile-20,12) for v in range(20,tile-20,12)
             if tex[v-10:v+11,u-10:u+11,3].min()>245]
    u,v=min(patches,key=lambda q:abs(tex[q[1],q[0],:3].mean()-{20:25,22:18}[mat]))
    start=len(verts)
    sides,rings=(72,18) if mat==22 else (40,12)
    top=xyz[:,1].max()
    radius=np.hypot(xyz[:,0],xyz[:,2]-.002)
    for k in range(sides+1):
        theta=k*2*np.pi/sides-np.pi
        alignment=np.cos(angle-theta)
        sector=np.flatnonzero(alignment>np.cos(.17))
        if len(sector)<12: sector=np.argsort(alignment)[-12:]
        bottom=xyz[sector,1].min()+.005
        for ring in range(rings):
            t=ring/rings
            y=bottom+(top-bottom)*t
            near=sector[np.argsort(abs(xyz[sector,1]-y))[:8]]
            r=max(0.,np.quantile(radius[near],.65)-.003)
            if mat==22:
                # Close the curl mass under the actual outer silhouette.
                r=(np.quantile(radius[near],.75)+.001)*(1.+.015*np.sin(9*theta+11*t))
            pt=np.array([r*np.sin(theta),y,.002+r*np.cos(theta)])
            verts.append((pt-[0,.49,-.021])/.06)
            roots.append(near[np.argmin(np.linalg.norm(xyz[near]-pt,axis=1))])
            if mat==22: uvs.append((k/sides,t))
            else: uvs.append(((u+5*np.sin(theta))/tile,(v+7*t)/tile))
    if mat==22:
        crown=np.array(verts[start:]).reshape(sides+1,rings,3)[:-1].copy()
        for _ in range(2):
            crown=.5*crown+.25*(np.roll(crown,1,axis=0)+np.roll(crown,-1,axis=0))
            crown[:,1:-1]=.6*crown[:,1:-1]+.2*(crown[:,:-2]+crown[:,2:])
        verts[start:]=list(np.concatenate((crown,crown[:1])).reshape(-1,3))
    pole=len(verts)
    verts.append((np.array([0,top-.002,.002])-[0,.49,-.021])/.06)
    roots.append(np.argmax(xyz[:,1]));uvs.append((u/tile,v/tile))
    for k in range(sides):
        a=start+k*rings; b=start+(k+1)*rings
        for j in range(rings-1):
            for tri in [(a+j,b+j,b+j+1),(a+j,b+j+1,a+j+1)]:
                tris.append(("",[(q,q) for q in tri]))
        tris.append(("",[(q,q) for q in (a+rings-1,b+rings-1,pole)]))
    return np.array(verts),np.array(uvs),tris,roots


def rooted_bob(scalp,shapes,faces):
    # Fit each root to a body triangle. The two guide layers have independent
    # roots and different lengths. Their slots reuse the original bob field.
    ids=np.array([[v for v,_ in f] for g,f in faces if g=="body"])
    ids=ids[np.all(scalp[ids,1]>.89,axis=1)]
    triangles=scalp[ids]
    origin=np.array([0.,.937,.001])
    e1=triangles[:,1]-triangles[:,0]
    e2=triangles[:,2]-triangles[:,0]
    offset=origin-triangles[:,0]
    cross=np.cross(offset,e1)
    def attach(target):
        direction=target-origin
        h=np.cross(direction,e2)
        det=np.sum(e1*h,axis=1)
        inv=np.divide(1.,det,out=np.zeros_like(det),where=abs(det)>1e-10)
        u=np.sum(offset*h,axis=1)*inv
        v=(cross@direction)*inv
        t=np.sum(e2*cross,axis=1)*inv
        hits=np.flatnonzero((u>=0)&(v>=0)&(u+v<=1)&(t>0))
        if not len(hits): raise ValueError("Bob guide misses scalp")
        i=hits[np.argmax(t[hits])]
        w=np.array([1.-u[i]-v[i],u[i],v[i]])
        return np.vstack((w@triangles[i],np.einsum("i,ijk->jk",w,shapes[ids[i]])))
    guides=np.empty((64,16,5,3))
    # Each sheet fans from the off-centre part over the skull toward one
    # temple, then down toward the jaw. The sheets meet at the nape.
    for k in range(64):
        side=-1 if k<32 else 1
        theta=np.pi*(k%32)/31
        front=max(0.,np.cos(theta))
        phi=side*(theta+(.46 if side>0 else .31)*np.exp(-(theta/.78)**2))
        radial=np.array([np.sin(phi),0.,np.cos(phi)])
        for layer in range(2):
            root_dir=np.array([.014+.0012*np.sin(3*theta)+side*(.00015+.0009*layer),
                               .048-.027*abs(np.cos(theta))**3,
                               .074*np.cos(theta)-.004+.002*layer])
            root_dir/=np.linalg.norm(root_dir)
            root=attach(origin+.08*root_dir)
            shoulder_dir=radial+np.array([0.,.09+.18*front**3,0.])
            shoulder=attach(origin+.08*shoulder_dir)
            radius_x=.063-.005*layer
            radius_z=.070-.0045*layer
            shoulder_pos=np.array([radius_x*np.sin(phi),.936+.015*front**3,
                                   -.001+radius_z*np.cos(phi)])
            if side<0: shoulder_pos[1]-=.012*front**4
            outward=shoulder_pos-shoulder[0]
            bend=.62
            tip_y=.841+.083*front**5+.012*(1.-np.cos(theta))*.5+.0035*layer
            tip_y+=.0027*np.sin(7.*theta+.8*side)+.0018*np.sin(13.*theta)
            for j in range(8):
                t=j/7
                if t<=bend:
                    w=t/bend
                    ray=(1.-w)*root_dir+w*shoulder_dir
                    ray/=np.linalg.norm(ray)
                    point=attach(origin+.08*ray)
                    smooth=w*w*(3.-2.*w)
                    point[0]+=smooth*outward+ray*(.0003+(.009-.005*layer)*np.sin(np.pi*w))
                else:
                    w=(t-bend)/(1.-bend)
                    end_angle=phi+side*(.48*front**5*w+.045*np.sin(4*theta+1.1)*np.sin(np.pi*w))
                    fold=.0035*np.sin(np.pi*w)-.004*w*w
                    point=shoulder.copy()
                    point[0]=[(radius_x+fold)*np.sin(end_angle),
                              (1.-w)*shoulder_pos[1]+w*tip_y,
                              -.001+(radius_z+fold)*np.cos(end_angle)]
                # Large waves carry neighbouring locks together. Vary the phase
                # along the part to keep the ends from forming one regular comb.
                roll=6.6*t+.85*np.sin(3*theta)+.42*np.sin(8*theta)+.6*side
                lift=t*t*(3.-2.*t)
                across=np.array([np.cos(phi),0.,-np.sin(phi)])
                point[0]+=across*side*.0085*lift*np.sin(roll)
                point[0]+=radial*lift*(.0050*np.cos(roll)+.0028*np.cos(9*theta+2.9*t))
                point[0,1]+=.0038*lift*np.cos(5*theta+3*t)
                guides[k,layer*8+j]=point
    return guides


def strand_field(fitted,morph):
    # A periodic guide surface, ordered from hairline to crown. The GPU
    # interpolates a different path over it for each individual hair.
    xyz=fitted*.06+[0,.49,-.021]
    angles=np.arctan2(xyz[:,0],xyz[:,2]-.002)
    radius=np.hypot(xyz[:,0],xyz[:,2]-.002)
    field=np.zeros((64,16,5,3))
    for k in range(64):
        angle=k*2*np.pi/64-np.pi
        rank=np.cos(angles-angle)
        sector=np.flatnonzero(rank>np.cos(.15))
        if len(sector)<12: sector=np.argsort(rank)[-12:]
        bottom=np.quantile(xyz[sector,1],.025)
        top=xyz[:,1].max()
        for j in range(16):
            t=j/15
            y=bottom+(top-bottom)*np.sin(t*np.pi/2)
            near=sector[np.argsort(abs(xyz[sector,1]-y))[:10]]
            r=np.quantile(radius[near],.85)*min(1.,(1.-t)*7.)
            pt=np.array([r*np.sin(angle),y,.002+r*np.cos(angle)])
            donor=near[np.argmin(np.linalg.norm(xyz[near]-pt,axis=1))]
            field[k,j,0]=pt
            field[k,j,1:]=morph[donor]*.06
    for _ in range(2):
        field=.5*field+.25*(np.roll(field,1,axis=0)+np.roll(field,-1,axis=0))
    return pack_guides(field)


def pack_guides(field):
    packed=bytearray()
    for sample in field.reshape(-1,5,3):
        pos=np.rint((sample[0]-[0,.9,0])*(32767/.25)).astype(int)
        packed.extend(struct.pack("<4h",*pos,0))
        for d in sample[1:]:
            packed.extend(struct.pack("<4b",*np.rint(np.clip(d/.055,-1,1)*127).astype(int),0))
    return packed


def bob_undercoat(guides,tile):
    # The lower-density representation shares the strand guides and stops
    # above their tips. Keep both sides of the part open at the forehead.
    points=[]; tex=[]; tris=[]
    rings=22
    for col in range(64):
        control=.4*guides[col,:8]+.6*guides[col,8:]
        for row in range(rings):
            t=row/(rings-1)*.92
            v=t*7; j=min(int(v),6); f=v-j
            p0,p1,p2,p3=control[[max(0,j-1),j,j+1,min(j+2,7)]]
            b=.5*(p2-p0); e=.5*(p3-p1)
            points.append(p1+f*(b+f*(3*(p2-p1)-2*b-e+f*(b+e-2*(p2-p1)))))
            tex.append(((1+509*(col%32)/31)/512,(1+509*row/(rings-1))/512))
            if col%32 and row:
                q=col*rings+row
                for ids in [(q-rings-1,q-rings,q),(q-rings-1,q,q-1)]:
                    tri=ids if col>=32 else ids[::-1]
                    tris.append(("",[(i,i) for i in tri]))
    field=np.asarray(points)
    # A dark, longitudinal undercoat supplies density between real fibres.
    # Its tip mask tapers over different distances across neighbouring locks.
    y,x=np.mgrid[0:tile,0:tile]/tile
    phase=x+.008*np.sin(7*y+12*x)+.0025*np.sin(12*y+28*x)
    tone=76+15*np.cos(2*np.pi*(phase*17))+10*np.cos(2*np.pi*(phase*67+.2*np.sin(y*8)))
    tone+=14*np.sin(2*np.pi*(phase*173+.12*np.sin(y*6+37*x)))
    tone*=.80+.20*np.sin(np.pi*y)
    tips=.935+.036*np.cos(2*np.pi*phase*14)+.022*np.sin(2*np.pi*phase*103)
    # Let rendered fibres own the border of the part and the face-framing
    # locks. The colour falloff continues along the same comb direction.
    border=.017+.010*np.sin(9*y)+.005*np.sin(38*y)
    edge=np.clip((x-border)*27,0,1)
    grain=np.sin(2*np.pi*phase*158)+.48*np.sin(2*np.pi*phase*73)
    alpha=np.clip((tips-y)*52,0,1)*np.clip(2.3*edge-0.55+.48*grain,0,1)*255
    rgba=np.uint8(np.stack((tone,tone,tone,alpha),axis=-1).clip(0,255))
    return field,np.asarray(tex),tris,Image.fromarray(rgba)


def cook(base_path,src,pack,dest):
    base,uv,faces=obj(base_path)
    # Narrow the neutral palpebral opening with the paired hm08 eyelid targets.
    for side in ("l","r"):
        for section in (1,2,3):
            delta=np.loadtxt(src/f"{side}-eye-height{section}-decr.target")
            base[delta[:,0].astype(int)] += 0.85*delta[:,1:4]
    # Include every facial joint below the skull (jaw, lips, lids and eyes).
    rig=json.loads((src/"default.mhskel").read_text())["bones"]
    def follows_head(bone):
        while bone and bone!="head":
            bone=rig[bone]["parent"]
        return bone=="head"

    weights=np.zeros((len(base),5))
    totals=np.zeros(len(base))
    for bone,values in json.loads((src/"default_weights.mhw").read_text())["weights"].items():
        fore=bone.startswith(("lowerarm","wrist","hand","finger","metacarpal"))
        shin=bone.startswith(("lowerleg","foot","toe"))
        mask=[fore or bone.startswith("upperarm"),fore,shin or bone.startswith("upperleg"),shin,follows_head(bone)]
        for i,w in values:
            weights[i]+=np.array(mask)*w
            totals[i]+=w
    weights/=np.maximum(totals,1e-9)[:,None]
    # Register whole authored heads at the eyeballs before fitting clothes,
    # brows and hair. Macro targets also encode the height of the entire body.
    eye_vertices=np.unique([i for g,tri in faces if g in ("helper-l-eye","helper-r-eye") for i,_ in tri])
    neck=np.clip((base[:,1]-5.45)/.66,0.,1.)
    neck=(neck*neck*(3.-2.*neck))[:,None]
    def head(name):
        rows=np.loadtxt(src/(name+"-young.target"))
        delta=np.zeros_like(base)
        delta[rows[:,0].astype(int)]=rows[:,1:4]
        return (delta-delta[eye_vertices].mean(axis=0))*neck

    # Four adult references, blended as whole faces at runtime. The modest
    # shared component leaves room for variation at each corner of the blend.
    references=np.stack([head(name) for name in (
        "caucasian-male","asian-male","caucasian-female","african-female")],axis=1)
    neutral=references.mean(axis=1)
    base+=neutral
    shapes=.85*(references-neutral[:,None,:])
    # A sparse fifth field closes the actual lid rings around the eyeball.
    closure=np.zeros_like(base)
    for side in ("left","right"):
        delta=np.loadtxt(src/f"eye-{side}-closure.target")
        closure[delta[:,0].astype(int)] += delta[:,1:4]*0.85
    outfits=["male_casualsuit01","male_casualsuit06","male_casualsuit05"]; shoes="shoes02"
    outfit=outfits[0]
    shoe_mask=hidden(pack/"clothes"/shoes/(shoes+".mhclo"))
    masks=[shoe_mask | hidden(pack/"clothes"/name/(name+".mhclo")) for name in outfits]
    tile=512
    atlas=Image.new("RGBA",(tile*4,tile*2))
    # hm08 stores its face island sideways on the right of the skin map.
    # Eye-mouth interiors and detached UV islands use the plain skin fallback.
    lo=np.array([0.635,0.115]);hi=np.array([1.,0.89])
    def put(idx,img):
        atlas.paste(img.convert("RGBA").resize((tile,tile),Image.Resampling.LANCZOS),((idx%4)*tile,(idx//4)*tile))
    # Bake the shinier T-zone and lips into unused face alpha. RGB stays sRGB.
    from PIL import ImageDraw, ImageFilter
    face_mask=Image.new("L",(tile,tile),128)
    brush=ImageDraw.Draw(face_mask)
    for group,tri in faces:
        if group != "body": continue
        ids,tex=zip(*tri)
        coords=(uv[list(tex)]-lo)/(hi-lo)
        if np.any(coords < 0) or np.any(coords > 1): continue
        x,y,z=base[list(ids)].mean(0)*.06+[0,.49,-.021]
        if z < .04: continue
        def area(height,width,radius):
            return np.exp(-((x/width)**2+((y-height)/radius)**2))
        sheen=max(.82*area(.907,.012,.021),.95*area(.885,.016,.005),.60*area(.950,.030,.020))
        brush.polygon([tuple(pt) for pt in coords*(tile-1)],fill=128+round(127*sheen))
    face_mask=face_mask.filter(ImageFilter.GaussianBlur(1.5))
    for idx,name in enumerate(["middleage_caucasian_male","young_caucasian_female","young_african_male"]):
        img=Image.open(next((pack/"skins"/name).glob("*diffuse*.png")))
        face=img.crop(tuple(np.r_[lo*img.size,hi*img.size].round().astype(int))).convert("RGBA").resize((tile,tile),Image.Resampling.LANCZOS)
        face.putalpha(face_mask)
        atlas.paste(face,(idx*tile,0))
    for idx,name in [(3,"short01"),(5,"afro01")]:
        put(idx,Image.open(next((pack/"hair"/name).glob("*diffuse*.png"))))
    # Three grayscale fabric maps and sixteen print glyphs occupy the old outfit tile.
    # Geometry, colour swatches and text can vary independently without another texture.
    cloth=[]
    for name in outfits:
        image=Image.open(pack/"clothes"/name/(name+"_diffuse.png")).convert("L").resize((tile,tile),Image.Resampling.LANCZOS)
        light=np.asarray(image,dtype=float)
        ref=np.percentile(light[light>12],74)
        cloth.append(Image.fromarray(np.uint8(np.clip(0.17+0.76*np.sqrt(light/ref),0,1)*255)))
    from PIL import ImageDraw, ImageFont
    prints=Image.new("L",(tile,tile))
    draw=ImageDraw.Draw(prints)
    names=[str(i) for i in range(10)]+["WARP","HOME","AWAY","GG","WEST","EAST"]
    for i,label in enumerate(names):
        x=i%4*128; y=i//4*128
        font=ImageFont.load_default(size=78 if len(label)==1 else (42 if len(label)<4 else 32))
        draw.text((x+64,y+64),label,font=font,fill=255,anchor="mm",stroke_width=1)
        if i>=10 and i%2==0:
            draw.rectangle((x+18,y+37,x+110,y+39),fill=215)
            draw.rectangle((x+18,y+88,x+110,y+90),fill=215)
    put(6,Image.merge("RGBA",(*cloth,prints)))
    put(7,Image.open(pack/"eyebrows/eyebrow001/eyebrow001.png"))
    vertices=bytearray();indices=[]
    targets=[bytearray() for _ in range(5)]
    def add(pos,texcoord,triangles,skin,morph,material,blink=None):
        normals=np.zeros_like(pos)
        for _,f in triangles:
            a,b,c=[i for i,_ in f]; v=np.cross(pos[b]-pos[a],pos[c]-pos[a])
            for i in (a,b,c):normals[i]+=v
        normals/=np.maximum(np.linalg.norm(normals,axis=1)[:,None],1e-12)
        seen={}
        for g,f in triangles:
            mat=material(g,f)
            if mat is None:continue
            for i,t in f:
                key=i,t,mat
                if key not in seen:
                    seen[key]=len(vertices)//22
                    xyz=pos[i]*.06+[0,.49,-.021]
                    uvp=texcoord[t] if mat & 31 !=11 else (texcoord[t]-lo)/(hi-lo)
                    signed=lambda x:np.round(np.clip(x,-1,1)*32767).astype(int)
                    blends=np.round(np.clip(morph[i]*.06/.055,-1,1)*127).astype(int)
                    for target,delta in zip(targets,blends):
                        if np.any(delta):
                            target.extend(struct.pack("<H3b",seen[key],*delta))
                    if blink is not None and np.any(blink[i]):
                        d=np.round(np.clip(blink[i]*.06/.012,-1,1)*127).astype(int)
                        if np.any(d): targets[4].extend(struct.pack("<H3b",seen[key],*d))
                    vertices.extend(struct.pack("<3h3h2H4BH",*signed(xyz/1.2),*signed(normals[i]),
                        *np.round(np.clip(uvp,0,1)*65535).astype(int),*np.round(np.clip(skin[i,:4],0,1)*255).astype(int),
                        mat | (round(np.clip(skin[i,4],0,1)*255)<<8)))
                indices.append(seen[key])
    # Bits 5..7 select fitted outfits. Zero marks geometry shared by all.
    def body_material(g,f):
        if "eye" in g: return 6
        visible=sum(1<<k for k,mask in enumerate(masks) if not all(i in mask for i,t in f))
        if not visible: return None
        face=min(base[i,1] for i,t in f)>5.35 and all(np.all((uv[t]>=lo)&(uv[t]<=hi)) for i,t in f)
        return (11 if face else 4) | ((visible if visible!=7 else 0)<<5)
    body=[(g,f) for g,f in faces if g in ("helper-l-eye","helper-r-eye","body")]
    add(base,uv,body,weights,shapes,body_material,closure)
    # The independent T-shirt and jacket share the same pair of jeans.
    # Keep their lower garment only once. Connected pieces avoid recolouring a long hem.
    def garment_material(pos,triangles,style):
        parents=list(range(len(pos)))
        def root(i):
            while parents[i]!=i:
                parents[i]=parents[parents[i]]; i=parents[i]
            return i
        for _,f in triangles:
            a,b,c=[i for i,t in f]; parents[root(b)]=root(a); parents[root(c)]=root(a)
        legs={root(i) for i in range(len(pos)) if pos[i,1]<-3.}
        def material(g,f):
            trouser=(sum(pos[i,1] for i,t in f)/3.<0.50 if style==0 else root(f[0][0]) in legs)
            if trouser and style==2: return None
            mask=6 if trouser and style==1 else 1<<style
            return (14 if trouser else 12) | (mask<<5)
        return material
    pieces=[("clothes",name,12) for name in outfits]+[("clothes",shoes,3),("eyebrows","eyebrow001",13),("hair","short01",20),("hair","bob02",21),("hair","afro01",22)]
    strand_fields=bytearray()
    for folder,name,mat in pieces:
        if mat==21:
            guides=rooted_bob(base*.06+[0,.49,-.021],shapes*.06,body)
            strand_fields.extend(pack_guides(guides))
            field,coord,tris,texture=bob_undercoat(guides,tile)
            head_skin=np.zeros((len(field),5));head_skin[:,4]=1.
            add((field[:,0]-[0,.49,-.021])/.06,coord,tris,head_skin,field[:,1:]/.06,lambda g,f:21)
            put(4,texture)
            continue
        path=pack/folder/name
        _,uv,faces=obj(path/(name+".obj"))
        refs,w,offset=fit(path/(name+".mhclo"),base)
        fitted=(base[refs]*w[:,:,None]).sum(1)+offset
        skin=(weights[refs]*w[:,:,None]).sum(1)
        morph=(shapes[refs]*w[:,:,None,None]).sum(1)
        material=garment_material(fitted,faces,outfits.index(name)) if mat==12 else lambda g,f:mat
        # The original coiled groom has stacked transparent sheets crossing the
        # face in the match renderer. Use its fitted outline to build a curl mass.
        if mat!=22: add(fitted,uv,faces,skin,morph,material)
        if mat >= 20:
            strand_fields.extend(strand_field(fitted,morph))
            locks,lock_uv,triangles,roots=groom_edges(fitted,mat,atlas)
            add(locks,lock_uv,triangles,skin[roots],morph[roots],material)
    put(5,curl_tile(tile))
    dest.mkdir(exist_ok=True,parents=True)
    assert len(vertices)%22==0 and len(vertices)//22 <= 65536
    model_tmp=dest/"person.bin.tmp"
    atlas_tmp=dest/"appearance.png.tmp"
    model_tmp.write_bytes(struct.pack("<4s7I",b"WH04",len(vertices)//22,len(indices),*(len(t)//5 for t in targets))
                          +vertices+np.array(indices,dtype="<u2").tobytes()+b"".join(targets))
    atlas.save(atlas_tmp,format="PNG",optimize=True)
    model_tmp.replace(dest/"person.bin")
    atlas_tmp.replace(dest/"appearance.png")
    (dest/"strand-guides.bin").write_bytes(strand_fields)
    print(len(vertices)//22,"vertices and",len(indices)//3,"triangles")

if __name__=="__main__":
    p=argparse.ArgumentParser(description=__doc__)
    for arg in ("base","sources","pack","destination"):p.add_argument(arg,type=Path)
    a=p.parse_args();cook(a.base,a.sources,a.pack,a.destination)
