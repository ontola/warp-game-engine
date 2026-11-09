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


def cook(base_path,src,pack,dest):
    base,uv,faces=obj(base_path)
    weights=np.zeros((len(base),4))
    totals=np.zeros(len(base))
    for bone,values in json.loads((src/"default_weights.mhw").read_text())["weights"].items():
        fore=bone.startswith(("lowerarm","wrist","hand","finger","metacarpal"))
        shin=bone.startswith(("lowerleg","foot","toe"))
        mask=[fore or bone.startswith("upperarm"),fore,shin or bone.startswith("upperleg"),shin]
        for i,w in values:
            weights[i]+=np.array(mask)*w
            totals[i]+=w
    weights/=np.maximum(totals,1e-9)[:,None]
    # Four signed blend shapes with independent seed weights.
    groups=[
        [("nose-scale-horiz-incr",1.15),("nose-point-down",0.85)],
        [("mouth-scale-horiz-incr",1.15),("mouth-upperlip-volume-incr",0.85)],
        [("eyebrows-angle-up",1.3)],
        [("caucasian-male-young",0.5),("caucasian-female-young",-0.5)],
    ]
    shapes=np.zeros((len(base),4,3))
    for channel,targets in enumerate(groups):
        for name,strength in targets:
            for line in (src/(name+".target")).read_text().splitlines():
                w=line.split()
                if w and w[0].isdigit():shapes[int(w[0]),channel]+=np.array(list(map(float,w[1:4])))*strength
    # Macro shape is restricted to the head; clothes retain the fitted body shape.
    shapes[:,3]*=np.clip((base[:,1]-5.45)/0.9,0,1)[:,None]
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
    for idx,name in enumerate(["middleage_caucasian_male","young_caucasian_female","young_african_male"]):
        img=Image.open(next((pack/"skins"/name).glob("*diffuse*.png")))
        put(idx,img.crop(tuple(np.r_[lo*img.size,hi*img.size].round().astype(int))))
    for idx,name in enumerate(["short01","bob02","afro01"],3):
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
    def add(pos,texcoord,triangles,skin,morph,material):
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
                    seen[key]=len(vertices)//38
                    xyz=pos[i]*.06+[0,.49,-.021]
                    uvp=texcoord[t] if mat & 31 !=11 else (texcoord[t]-lo)/(hi-lo)
                    signed=lambda x:np.round(np.clip(x,-1,1)*32767).astype(int)
                    blends=np.round(np.clip(morph[i]*.06/.055,-1,1)*127).astype(int)
                    vertices.extend(struct.pack("<3h3h2H4B16bH",*signed(xyz/1.2),*signed(normals[i]),
                        *np.round(np.clip(uvp,0,1)*65535).astype(int),*np.round(np.clip(skin[i],0,1)*255).astype(int),
                        *np.pad(blends,((0,0),(0,1))).ravel(),mat))
                indices.append(seen[key])
    # Bits 5..7 select fitted outfits. Zero marks geometry shared by all.
    def body_material(g,f):
        if "eye" in g: return 6
        visible=sum(1<<k for k,mask in enumerate(masks) if not all(i in mask for i,t in f))
        if not visible: return None
        face=min(base[i,1] for i,t in f)>5.35 and all(np.all((uv[t]>=lo)&(uv[t]<=hi)) for i,t in f)
        return (11 if face else 4) | ((visible if visible!=7 else 0)<<5)
    body=[(g,f) for g,f in faces if g in ("helper-l-eye","helper-r-eye","body")]
    add(base,uv,body,weights,shapes,body_material)
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
    for folder,name,mat in pieces:
        path=pack/folder/name
        _,uv,faces=obj(path/(name+".obj"))
        refs,w,offset=fit(path/(name+".mhclo"),base)
        fitted=(base[refs]*w[:,:,None]).sum(1)+offset
        skin=(weights[refs]*w[:,:,None]).sum(1)
        morph=(shapes[refs]*w[:,:,None,None]).sum(1)
        material=garment_material(fitted,faces,outfits.index(name)) if mat==12 else lambda g,f:mat
        add(fitted,uv,faces,skin,morph,material)
    dest.mkdir(exist_ok=True,parents=True)
    assert len(vertices)%38==0 and len(vertices)//38 <= 65536
    model_tmp=dest/"person.bin.tmp"
    atlas_tmp=dest/"appearance.png.tmp"
    model_tmp.write_bytes(struct.pack("<II",len(vertices)//38,len(indices))+vertices+np.array(indices,dtype="<u2").tobytes())
    atlas.save(atlas_tmp,format="PNG",optimize=True)
    model_tmp.replace(dest/"person.bin")
    atlas_tmp.replace(dest/"appearance.png")
    print(len(vertices)//38,"vertices and",len(indices)//3,"triangles")

if __name__=="__main__":
    p=argparse.ArgumentParser(description=__doc__)
    for arg in ("base","sources","pack","destination"):p.add_argument(arg,type=Path)
    a=p.parse_args();cook(a.base,a.sources,a.pack,a.destination)
