// Rebuild the original scenery with "node scenery.mjs". Coordinates are metres.
import { writeFileSync } from "node:fs";

let seed = 9417;
function random() { seed = (Math.imul(seed,1664525) + 1013904223) >>> 0; return seed / 2**32; }
const range = (a,b) => a + (b-a)*random();
const pick = colors => colors[Math.floor(random()*colors.length)];
const groups = new Map();
const smoothNormals = new Map();
const pointKey = p => p.map(n=>Math.round(n*1e5)).join(",");
function tri(color, a,b,c) {
    if (!groups.has(color)) groups.set(color, []);
    groups.get(color).push(...a,...b,...c);
}
function quad(color,a,b,c,d) { tri(color,a,b,c); tri(color,a,c,d); }
function height(x,z) {
    const hills = Math.max(0, Math.min(1,(-z-16)/40));
    return z*.055 + hills*(2.6*Math.sin(x*.063+z*.03)+1.8*Math.cos(z*.12));
}
function ground(x,z,dy=0) { return [x,height(x,z)+dy,z]; }
function mound(color,x,y,z,w,h,d,segments=13,rings=7) {
    const point = (i,j) => {
        const lat = Math.PI*(j/rings-.5), lon = Math.PI*2*i/segments;
        const point = [x+w*Math.cos(lat)*Math.cos(lon), y+h*Math.sin(lat), z+d*Math.cos(lat)*Math.sin(lon)];
        const n = [Math.cos(lat)*Math.cos(lon)/w, Math.sin(lat)/h, Math.cos(lat)*Math.sin(lon)/d];
        const len = Math.hypot(...n);
        smoothNormals.set(pointKey(point), n.map(value=>value/len));
        return point;
    };
    for(let j=0;j<rings;j++) for(let i=0;i<segments;i++) {
        const a=point(i,j),b=point(i,j+1),c=point(i+1,j+1),d=point(i+1,j);
        if(j+1<rings) tri(color,a,b,c);
        if(j>0) tri(color,a,c,d);
    }
}
const grass = "#9fbb68";
for(let x=-100;x<100;x+=4) for(let z=-100;z<16;z+=4)
    quad(grass,ground(x,z),ground(x,z+4),ground(x+4,z+4),ground(x+4,z));
const pathX = z => 4.4+3.4*Math.sin(z*.082);
for(let z=-72;z<12;z++) {
    const edge=(z,sign)=>ground(pathX(z)+sign*(.94+.16*Math.sin(z*.8)), z,.065);
    quad("#d2b884",edge(z,-1),edge(z+1,-1),edge(z+1,1),edge(z,1));
}
// The closest marks stand up in little fans so they can occlude the walker.
for(let i=0;i<2500;i++) {
    const x=range(-22,22), z=range(-26,7);
    if(Math.abs(x-pathX(z))<1.1) continue;
    const y=height(x,z), size=range(.14,.39);
    for(let k=0;k<3;k++) {
        const dx=range(-.12,.12), bx=x+dx, bz=z+range(-.06,.06);
        tri(pick(["#7c9d56","#b6cc74","#cbd489"]),[bx-.026,y,bz],[bx+.031,y,bz],[bx+dx+.055,y+size*range(.6,1.3),bz-.03]);
    }
    if(i%17===0) mound(pick(["#f5de8e","#ffefcd","#cf946f"]),x,y+size*.9,z,.06,.035,.045,5,3);
}
// A few tall foreground stems cross the cloak in the opening view.
for(const [x,z,h] of [[-.23,.50,.65],[.31,.46,.54],[-.43,.72,.49],[.22,.83,.40]]) {
    for(let n=0;n<4;n++){
        const dx=range(-.13,.13),y=height(x,z);
        tri("#729553",[x+dx-.018,y,z],[x+dx+.023,y,z],[x+dx*1.7+.05,y+h*range(.78,1),z-.12]);
    }
}
function house(x,z,w) {
    const y=height(x,z)-.4,h=w*.72, d=w*.63;
    const p=(a,b,c)=>[x+a*w,y+b*h,z+c*d];
    quad("#f2d6a2",p(-.5,0,1),p(.5,0,1),p(.5,1,1),p(-.5,1,1));
    quad("#d4b18c",p(.5,0,1),p(.5,0,-1),p(.5,1,-1),p(.5,1,1));
    quad("#e1c69a",p(-.5,0,-1),p(-.5,0,1),p(-.5,1,1),p(-.5,1,-1));
    tri("#ebd4b0",p(-.5,1,1),p(.5,1,1),p(0,1.58,1));
    quad("#bd724f",p(-.6,.98,1.1),p(0,1.6,1.1),p(0,1.6,-1.1),p(-.6,.98,-1.1));
    quad("#945a49",p(0,1.6,1.1),p(.6,.98,1.1),p(.6,.98,-1.1),p(0,1.6,-1.1));
    quad("#84694d",p(-.06,0,1.012),p(.08,0,1.012),p(.08,.57,1.012),p(-.06,.57,1.012));
    for(const wx of [-.29,.24]) {
        for(const sx of [wx-.045,wx+.135]) quad("#818268",p(sx,.37,1.013),p(sx+.04,.37,1.013),p(sx+.04,.68,1.013),p(sx,.68,1.013));
    }
    for(let k=0;k<4;k++){
        const u=.11+.1*k, v=1.6-u*.62/.6;
        quad("#ac6649",p(-u,v+.008,.85),p(-u-.01,v-.005,.85),p(-u-.01,v-.005,-.8),p(-u,v+.008,-.8));
    }
    const chimney=(a,b,c)=>p(.22+a,1.35+b,.1+c);
    quad("#b89a80",chimney(0,0,0),chimney(.10,0,0),chimney(.1,.40,0),chimney(0,.40,0));
    quad("#dbb799",chimney(.10,0,0),chimney(.1,0,-.14),chimney(.1,.40,-.14),chimney(.1,.4,0));
    for(const wx of [-.29,.24]) quad("#566566",p(wx,.38,1.01),p(wx+.13,.38,1.01),p(wx+.13,.66,1.01),p(wx,.66,1.01));
}
for(let i=0;i<16;i++) house(range(-17,14),range(-48,-28),range(2.1,3.7));
function tree(x,z,scale) {
    const y=height(x,z), crown=pick(["#67895f","#72925d","#849957"]);
    mound("#806746",x,y+scale*1.3,z,scale*.16,scale*1.4,scale*.16,7,4);
    for(let k=0;k<6;k++) {
        const a=k*2.4, reach=k===0?0:.83;
        mound(crown,x+scale*reach*Math.cos(a),y+scale*(k===0?3.35:2.6+range(-.22,.32)),
            z+scale*reach*Math.sin(a),scale*1.13,scale*range(.8,1.15),scale*.98,10,5);
    }
}
for(let i=0;i<45;i++) tree(range(-35,37),range(-65,-18),range(.55,1.3));
tree(-6.9,-3,1.6); tree(10,-9,1.35);
for(const [z,base,relief,color] of [[-143,11,11,"#abbec1"],[-113,5,12,"#829eaa"],[-87,0,9,"#738f83"]]) {
    const ridge = x => base+relief*(.51+.24*Math.cos(x*.095)+.21*Math.sin(x*.19)+.08*Math.cos(x*.47));
    for(let x=-170;x<170;x+=2.5) {
        const y=ridge(x), next=ridge(x+2.5);
        quad(color,[x,-12,z+9],[x+2.5,-12,z+9],[x+2.5,next,z],[x,y,z]);
        if(next>y && z < -90) tri("#acbdbe",[x,y,z+.06],[x+2.5,next,z+.06],[x-1.3,y-1.6,z+1.7]);
    }
}
for(const [x,y,z,s] of [[-32,22,-98,1],[22,25,-107,1.35],[58,21,-93,.74],[-65,19,-90,.65]]) {
    for(let k=0;k<11;k++) {
        const t=k/10, wave=Math.sin(t*Math.PI);
        mound("#f8edce",x+s*(t-.5)*31,y+s*(wave*2+range(-1,1)),z+range(-2,2),
            s*(3+wave*2),s*(1.1+wave*4),s*3.2,13,7);
    }
}

save("meadow.glb");
groups.clear();
smoothNormals.clear();

// Each ring is [height, radius X, radius Z, centre X, centre Z]. A form
// function can pleat fabric, curl the hat brim, or cant a shoulder strap.
function rings(color, rows, form=(p)=>p, count=36) {
    const vertex=(row,i)=>{
        const [y,rx,rz,x=0,z=0]=rows[row], a=i/count*2*Math.PI;
        return form([x+rx*Math.cos(a),y,z+rz*Math.sin(a)],a,row);
    };
    for(let j=0;j<rows.length-1;j++) for(let i=0;i<count;i++)
        quad(color,vertex(j,i),vertex(j+1,i),vertex(j+1,i+1),vertex(j,i+1));
}
const skin="#d69b73", hair="#573b34", cloak="#347888", hem="#72a9a2", scarf="#bc593b",
    straw="#efc579", band="#905638", bag="#925c37", flap="#b3804e", buckle="#d6ac64";
mound(skin,0,1.64,-.01,.155,.205,.145,20,12);
mound(hair,0,1.69,.045,.164,.155,.135,20,10);
for(let k=0;k<7;k++) {
    const a=k*.45+.2;
    mound(hair,.145*Math.cos(a),1.60+.012*Math.sin(k),.032+.11*Math.sin(a),.045,.10,.04,9,6);
}
const folds=(p,a,row)=>{
    const f=(.044*Math.cos(9*a)+.018*Math.sin(5*a))*Math.max(0,1-row*.26);
    return [p[0]+f*Math.cos(a),p[1]+(.024*Math.sin(5*a)+.016*Math.cos(9*a))*Math.max(0,1-row*.23),p[2]+f*Math.sin(a)];
};
const capeRows=[[.51,.44,.32],[.54,.45,.33],[.76,.39,.30],[1.23,.30,.22],[1.39,.26,.19],[1.47,.15,.14]];
rings(cloak,capeRows,folds);
rings(hem,[[.54,.452,.332],[.56,.45,.331]],folds);
rings(scarf,[[1.39,.22,.18],[1.45,.22,.17],[1.51,.17,.15]]);
rings(scarf,[[1.08,.045,.025,.26,.22],[1.22,.058,.03,.22,.22],[1.43,.055,.02,.14,.16]]);
const curl=(p,a,row)=>[p[0],p[1]+(.028*Math.cos(2*a)+.031*Math.sin(a))*Math.max(0,1-row*.23),p[2]];
rings(straw,[[1.82,.40,.35],[1.84,.43,.37],[1.86,.39,.34],[1.89,.24,.21],[1.97,.22,.19],[2.025,.16,.14],[2.03,.001,.001]],curl);
rings(band,[[1.88,.248,.218],[1.913,.24,.211]],curl);
for(const side of [-1,1]) {
    rings(bag,[[.86,.025,.025,side*.18,.27],[1.19,.028,.025,side*.16,.25],[1.39,.032,.025,side*.16,.16]],undefined,9);
}
rings(bag,[[.81,.13,.084,-.02,.29],[.85,.20,.12,-.02,.29],[1.15,.21,.11,-.02,.28],[1.21,.16,.085,-.02,.27],[1.22,.001,.001,-.02,.27]]);
mound(flap,-.02,1.115,.376,.20,.095,.035,18,8);
mound(buckle,-.02,1.06,.408,.026,.033,.009,10,5);
save("traveller.glb",new Map([[skin,1],[hair,2],[cloak,3],[hem,3],[scarf,4],[straw,5],[band,5],[bag,6],[flap,6],[buckle,6]]));

// glTF has one primitive per paint colour. Compute flat normals for the facets.
function save(file, tags=new Map()) {
    const bin=[], views=[], accessors=[], materials=[], primitives=[];
    let length=0;
    function floats(values,min,max) {
        const data=Buffer.from(new Float32Array(values).buffer);
        const bufferView=views.push({buffer:0,byteOffset:length,byteLength:data.length})-1;
        bin.push(data); length+=data.length;
        return accessors.push({bufferView,componentType:5126,count:values.length/3,type:"VEC3",min,max})-1;
    }
    for(const [color,positions] of groups) {
        const normals=[],min=[Infinity,Infinity,Infinity],max=[-Infinity,-Infinity,-Infinity];
        for(let i=0;i<positions.length;i+=9) {
            const [ax,ay,az,bx,by,bz,cx,cy,cz]=positions.slice(i,i+9);
            const ux=bx-ax,uy=by-ay,uz=bz-az,vx=cx-ax,vy=cy-ay,vz=cz-az;
            const nx=uy*vz-uz*vy,ny=uz*vx-ux*vz,nz=ux*vy-uy*vx,mag=Math.hypot(nx,ny,nz)||1;
            for(let k=0;k<3;k++) {
                const at=positions.slice(i+3*k,i+3*k+3);
                normals.push(...(smoothNormals.get(pointKey(at)) ?? [nx/mag,ny/mag,nz/mag]));
                for(let j=0;j<3;j++){const v=positions[i+3*k+j];min[j]=Math.min(min[j],v);max[j]=Math.max(max[j],v);}
            }
        }
        const rgb=color.match(/[0-9a-f]{2}/g).map(hex=>((parseInt(hex,16)/255+.055)/1.055)**2.4);
        const material=materials.push({name:color,extras:tags.has(color)?{warpSurface:tags.get(color)}:undefined,doubleSided:true,pbrMetallicRoughness:{baseColorFactor:[...rgb,1],metallicFactor:0,roughnessFactor:1}})-1;
        primitives.push({attributes:{POSITION:floats(positions,min,max),NORMAL:floats(normals)},material});
    }
    const doc={asset:{version:"2.0"},scene:0,scenes:[{nodes:[0]}],nodes:[{mesh:0}],meshes:[{primitives}],materials,
        buffers:[{byteLength:length}],bufferViews:views,accessors};
    let json=Buffer.from(JSON.stringify(doc));
    json=Buffer.concat([json,Buffer.alloc((4-json.length%4)%4,32)]);
    const header=Buffer.alloc(20), chunk=Buffer.alloc(8);
    header.writeUInt32LE(0x46546c67,0); header.writeUInt32LE(2,4); header.writeUInt32LE(28+json.length+length,8);
    header.writeUInt32LE(json.length,12); header.writeUInt32LE(0x4e4f534a,16);
    chunk.writeUInt32LE(length,0); chunk.writeUInt32LE(0x004e4942,4);
    writeFileSync(new URL(file,import.meta.url),Buffer.concat([header,json,chunk,...bin]));
}
