// Port Meridian's surfaces use world-space metres, keeping floor heights,
// sidewalk widths and street markings independent of entity scale.
struct SurfaceIn {
    kind: u32, instance: u32, p: vec3<f32>, local: vec3<f32>, n: vec3<f32>,
    dir: vec3<f32>, bary: vec3<f32>, uv0: vec2<f32>, uv1: vec2<f32>, uv2: vec2<f32>,
    dx: vec3<f32>, dy: vec3<f32>, tint: vec3<f32>, rough: f32, metal: f32,
    scale: f32, time: f32,
};
struct SurfaceOut {
    albedo: vec3<f32>, normal: vec3<f32>, rough: f32, metal: f32, emission: vec3<f32>,
};

fn pm_hash(p: vec2<f32>) -> f32 {
    var a = fract(vec3<f32>(p.x,p.y,p.x)*vec3<f32>(0.1031,0.0973,0.1137));
    a += dot(a,a.yzx+33.33);
    return fract((a.x+a.y)*a.z);
}

fn pm_band(distance: f32, half_width: f32, aa: f32) -> f32 {
    return 1.0-smoothstep(half_width-aa,half_width+aa,abs(distance));
}

fn pm_street(p: vec2<f32>, tint: vec3<f32>, aa: f32) -> vec3<f32> {
    let curb=abs(p-56.0*round(p/56.0));
    let street=min(curb.x,curb.y);
    let along=select(p.x,p.y,curb.x<curb.y);
    let across=select(curb.y,curb.x,curb.x<curb.y);
    let along_dist=max(curb.x,curb.y);
    let west_exit = p.x < -168.0 && abs(p.y) <= 6.0;
    if street>6.0 || (max(abs(p.x),abs(p.y))>174.0 && !west_exit) {
        let grid=abs(fract(p/1.6+0.5)-0.5)*1.6;
        let seam=pm_band(min(grid.x,grid.y),0.024,aa);
        let slab=pm_hash(floor(p/1.6));
        let walk=vec3<f32>(0.45,0.43,0.37)*(0.94+slab*0.09);
        return mix(walk,vec3<f32>(0.24,0.27,0.25),seam*0.55);
    }
    let aggregate=pm_hash(floor(p*9.0))*(1.0-smoothstep(0.03,0.17,aa));
    var asphalt=tint*(0.70+aggregate*0.12);
    // A shallow gutter, a painted curb, and worn tyre lines establish scale.
    asphalt *= 1.0-0.14*pm_band(abs(across-2.85)-0.76,0.22,aa);
    let gutter=pm_band(across-5.81,0.16,aa);
    asphalt=mix(asphalt,vec3<f32>(0.28,0.27,0.24),gutter);
    if along_dist>8.5 {
        let arterial=min(abs(p.x),abs(p.y))<6.0;
        var stripe=pm_band(across,0.08,aa)*step(0.40,fract(along/7.0));
        if arterial { stripe=pm_band(across-0.14,0.055,aa); }
        asphalt=mix(asphalt,vec3<f32>(0.73,0.51,0.20),stripe);
        let bay=pm_band(across-4.65,0.065,aa);
        asphalt=mix(asphalt,vec3<f32>(0.55,0.54,0.46),bay*0.8);
    }
    if along_dist>6.3 && along_dist<8.2 {
        let zebra=pm_band((fract(across/1.25+0.5)-0.5)*1.25,0.31,aa);
        asphalt=mix(asphalt,vec3<f32>(0.73,0.72,0.65),zebra*(1.0-smoothstep(4.9,5.7,across)));
    }
    return asphalt;
}

// Cast iron lies flush with the road. Use the same 56 m centre lines and
// 6 m kerbs as pm_street, keeping the inlets upstream of the crossings.
fn pm_road_iron(p: vec2<f32>, aa: f32, road: SurfaceOut) -> SurfaceOut {
    if aa >= .30 || max(abs(p.x),abs(p.y)) > 173.0 { return road; }
    let lines=round(p/56.0);
    let offset=p-56.0*lines;
    let vertical=abs(offset.x)<abs(offset.y);
    let across=select(offset.y,offset.x,vertical);
    let along=select(p.x,p.y,vertical);
    let side=abs(across);
    let inlet=abs(side-5.65)<.25+aa;
    let cover=abs(side-1.65)<.42+aa;
    if !inlet && !cover { return road; }
    let line=select(lines.y,lines.x,vertical);
    let block=floor(along/56.0);
    let along_block=along-56.0*block;
    let street_phase=line*.618+block*.382+select(.27,0.0,vertical);
    let fade=1.0-smoothstep(.12,.30,aa);
    var mask=0.0;
    var iron=vec3<f32>(.088,.088,.075);
    if inlet {
        let phase=fract(street_phase+select(.53,0.0,across>0.0));
        let center=select(15.4+6.0*phase,36.0+5.0*phase,phase>.5);
        let q=vec2<f32>(along_block-center,side-5.65);
        if abs(q.x)>.46+aa { return road; }
        let corner=abs(q)-vec2<f32>(.395,.195);
        let edge=length(max(corner,vec2<f32>(0.0)))+min(max(corner.x,corner.y),0.0)-.045;
        mask=1.0-smoothstep(-aa,aa,edge);
        let rim=pm_band(edge+.033,.017,aa);
        // Two rows of slots separated by a cast spine, inside a raised rim.
        let tooth=(fract(q.x/.094+.5)-.5)*.094;
        let detail=1.0-smoothstep(.022,.075,aa);
        let slots=mix(.39,pm_band(tooth,.019,aa),detail)
            *pm_band(q.x,.365,aa)*pm_band(q.y,.149,aa)
            *(1.0-pm_band(q.y,.019,aa)*detail);
        iron=mix(iron,vec3<f32>(.013,.016,.017),slots);
        iron=mix(iron,vec3<f32>(.19,.17,.135),rim);
    } else {
        // An access cover in one lane on alternating blocks. Its tread fades
        // to an area average as the camera moves away.
        if fract((line+block)*.5+select(.5,0.0,vertical))>.25 { return road; }
        let phase=fract(street_phase);
        let center=vec2<f32>(27.0+6.0*phase,select(1.65,-1.65,phase>.5));
        let q=vec2<f32>(along_block,across)-center;
        if max(abs(q.x),abs(q.y))>.40+aa { return road; }
        let r=length(q);
        mask=1.0-smoothstep(.39-aa,.39+aa,r);
        let ring=pm_band(r-.34,.014,aa);
        let diagonal=(fract(vec2<f32>(q.x+q.y,q.x-q.y)/.092+.5)-.5)*.092;
        let tread=pm_band(min(abs(diagonal.x),abs(diagonal.y)),.009,aa);
        let face=1.0-smoothstep(.30-aa,.30+aa,r);
        iron+=vec3<f32>(.053)*mix(.35,tread,1.0-smoothstep(.018,.06,aa))*face;
        iron=mix(iron,vec3<f32>(.023,.025,.022),ring);
        let sockets=length(abs(q)-vec2<f32>(.24,.10));
        iron*=1.0-.8*(1.0-smoothstep(.020-aa,.020+aa,sockets));
    }
    mask*=fade;
    var out=road;
    out.albedo=mix(road.albedo,iron,mask);
    out.rough=mix(road.rough,.61,mask);
    out.metal=mix(road.metal,.48,mask);
    return out;
}

// Each parcel keeps its window and display rhythm when the street-level
// shell streams into separate wall sections or a room is revisited.
fn pm_parcel(s: SurfaceIn) -> f32 {
    let lot=floor(s.p.xz/56.0);
    let half_lot=select(0.0,.37,s.kind==2u && s.p.x>(lot.x+.5)*56.0);
    return floor(pm_hash(lot+vec2<f32>(half_lot,0.0))*19.0);
}

// Keep courses in world metres so the solid shell and streamed wall panels meet.
fn pm_wall_stone(s: SurfaceIn, p: vec2<f32>, brick: bool, aa: f32) -> SurfaceOut {
    var out=SurfaceOut(s.tint,s.n,.83,0.0,vec3<f32>(0.0));
    let unit=select(vec2<f32>(1.32,.49),vec2<f32>(.39,.135),brick);
    let row=floor(p.y/unit.y);
    let bond=vec2<f32>(p.x/unit.x+fract(row*.5),p.y/unit.y);
    let within=fract(bond);
    let edge=min(within,1.0-within)*unit;
    let seam=min(edge.x,edge.y);
    let joint=select(.012,.006,brick);
    let resolvable=1.0-smoothstep(unit.y*.20,unit.y*.72,aa);
    if resolvable<=0.0 { return out; }
    let mortar=(1.0-smoothstep(max(0.0,joint-aa),joint+aa,seam))*resolvable;
    let grout=mix(s.tint*.65,vec3<f32>(.35,.30,.25),select(.25,.65,brick));
    if SURFACE_LOW_DETAIL {
        out.albedo=mix(s.tint,grout,mortar);
        return out;
    }
    let block=pm_hash(floor(bond));
    let fired=select(vec3<f32>(.075,.065,.054),vec3<f32>(.22,.15,.11),brick);
    let face=s.tint*(1.0+(block-.50)*fired*resolvable);
    out.albedo=mix(face,grout,mortar);
    out.rough=mix(.76+.12*block,.98,mortar);
    // A sloped shoulder on each side of the recessed mortar responds to the actual sun.
    let bevel=select(.037,.018,brick);
    let slope=(vec2<f32>(1.0)-smoothstep(vec2<f32>(joint),vec2<f32>(bevel+aa),edge))*smoothstep(vec2<f32>(0.0),vec2<f32>(joint),edge);
    let direction=sign(vec2<f32>(.5)-within);
    let relief=slope*direction*(resolvable*(1.0-smoothstep(.014,.085,aa))*.41);
    let tangent=select(vec3<f32>(0,0,1),vec3<f32>(1,0,0),abs(s.n.z)>.5);
    out.normal=normalize(s.n-tangent*relief.x-vec3<f32>(0,relief.y,0));
    return out;
}

fn pm_facade(s: SurfaceIn, aa: f32) -> SurfaceOut {
    var out=SurfaceOut(s.tint,s.n,0.77,0.0,vec3<f32>(0.0));
    if abs(s.n.y)>0.6 {
        let seam=pm_band(fract(s.p.x/1.3)*1.3,0.035,aa);
        out.albedo=s.tint*vec3<f32>(0.69,0.72,0.74)*(1.0-seam*0.15);
        return out;
    }
    let t=select(s.p.z,s.p.x,abs(s.n.z)>0.5);
    let p=vec2<f32>(t,s.p.y);
    let offset=select(pm_parcel(s),f32(s.instance%19u),s.kind==8u)*2.37;
    if s.kind==3u || s.kind==8u {
        let seam=pm_band(fract((p.x+offset)/0.47)-0.5,0.12,min(0.4,aa/0.47));
        out.albedo=s.tint*(0.77+0.23*seam);
        if s.kind==3u {
            let bay=fract((p.x+1.2)/7.2);
            if p.y<3.5 && bay>0.16 && bay<0.84 {
                let slat=pm_band(fract(p.y/.27)-0.5,0.07,min(0.35,aa/.27));
                out.albedo=mix(s.tint*0.48,vec3<f32>(0.25,0.28,0.27),slat);
            }
        }
        out.rough=0.62;
        if !SURFACE_LOW_DETAIL && aa<.18 {
            let fold=fract((p.x+offset)/.47)-.5;
            let flange=(1.0-smoothstep(.0,.095,abs(abs(fold)-.15)))*sign(fold);
            let tangent=select(vec3<f32>(0,0,1),vec3<f32>(1,0,0),abs(s.n.z)>.5);
            out.normal=normalize(s.n+tangent*flange*.46*(1.0-smoothstep(.035,.18,aa)));
            let beam=pm_band(fract((p.y+.07)/2.54)*2.54-.07,.048,aa);
            out.albedo*=1.0-.19*beam;
            out.metal=.21;
        }
        return out;
    }
    let hotel=s.kind==2u;
    out=pm_wall_stone(s,p,hotel,aa);
    let pitch=select(vec2<f32>(2.3,3.1),vec2<f32>(2.85,3.1),hotel);
    let grid=vec2<f32>((t+offset)/pitch.x,p.y/pitch.y);
    let cell=floor(grid);
    let frac_cell=fract(grid);
    // Offices have ribbon windows; the old quarter has deeper, narrower reveals.
    let widths=select(vec2<f32>(0.465,0.36),vec2<f32>(0.302,0.30),hotel);
    let window=pm_band((frac_cell.x-0.5)*pitch.x,widths.x*pitch.x,aa)*
        pm_band((frac_cell.y-0.53)*pitch.y,widths.y*pitch.y,aa);
    let room=pm_hash(cell+vec2<f32>(offset,9.0));
    let cool=vec3<f32>(0.045,0.115,0.145);
    let sky=vec3<f32>(0.22,0.365,0.39);
    let glass=mix(cool,sky,0.18+0.52*frac_cell.y+room*0.23);
    let windowLocal=(frac_cell-vec2<f32>(.5,.53))*pitch;
    let halfWindow=widths*pitch;
    let sill=pm_band(windowLocal.y+halfWindow.y+.060,.085,aa)*
        pm_band(windowLocal.x,halfWindow.x+.16,aa);
    let casing=pm_band(windowLocal.x,halfWindow.x+.072,aa)*
        pm_band(windowLocal.y,halfWindow.y+.072,aa);
    let trim=mix(s.tint,vec3<f32>(.66,.58,.44),select(.22,.50,hotel));
    var wall=mix(out.albedo,trim,casing);
    wall=mix(wall,trim*1.18,sill);
    var pane=glass;
    var metal=.18;
    let masonryNormal=out.normal;
    if !SURFACE_LOW_DETAIL && aa<.24 {
        // Recess the panes behind an outer stone reveal. The offset grows at grazing angles.
        let tangent=select(vec3<f32>(0,0,1),vec3<f32>(1,0,0),abs(s.n.z)>.5);
        let revealDepth=select(.065,.14,hotel);
        let eyeShift=vec2<f32>(dot(s.dir,tangent),s.dir.y)*revealDepth/max(.25,abs(dot(s.dir,s.n)));
        let behind=windowLocal+clamp(eyeShift,vec2<f32>(-.32),vec2<f32>(.32));
        let inset=pm_band(behind.x,halfWindow.x-.042,aa)*pm_band(behind.y,halfWindow.y-.045,aa);
        let bar=max(pm_band(behind.x,.025,aa),pm_band(behind.y-halfWindow.y*.48,.028,aa));
        pane=mix(trim*.48,mix(glass,vec3<f32>(.055,.077,.082),bar),inset);
        let apron=pm_band(windowLocal.y+halfWindow.y+.159,.040,aa)*pm_band(windowLocal.x,halfWindow.x+.105,aa);
        wall*=1.0-.22*apron*(1.0-smoothstep(.07,.24,aa));
        metal*=inset;
    }
    out.albedo=mix(wall,pane,window);
    out.normal=normalize(mix(masonryNormal,s.n,max(window,max(casing,sill))));
    out.rough=mix(out.rough,0.28,window);
    out.metal=metal*window;
    // Occupied rooms emit a modest warm light, naturally visible after dusk.
    let blind=select(1.0,0.51,frac_cell.y>0.34+0.48*room);
    let curtain=pm_band(frac_cell.x-.21,.08,aa/pitch.x)*step(.53,room);
    out.albedo=mix(out.albedo,s.tint*.58,curtain*window*.6);
    out.emission=vec3<f32>(0.94,0.61,0.31)*select(0.0,14.0,room>0.73)*window*blind*(1.0-curtain*.54);
    if p.y<2.45 {
        let shop=fract((t+offset)/6.8);
        let pane=pm_band((shop-.5)*6.8,2.85,aa);
        let frame=pm_band((shop-.5)*6.8,0.06,aa);
        out.albedo=mix(s.tint*0.82,glass*0.73,pane*(1.0-frame));
        out.rough=mix(0.7,0.3,pane);
        out.emission*=0.30;
    }
    return out;
}

// Seven rows per four-letter sign, 5x7 letters separated by one column.
const PM_SIGN_ROWS=array<u32,42>(
    7923615u, 4527172u, 4527172u, 7935876u, 4265220u, 4265092u, 4252740u,
    3740561u, 4527185u, 4527178u, 4528068u, 5575748u, 4789316u, 3466308u,
    4253582u, 4211793u, 4211793u, 4211793u, 4211793u, 4211793u, 8185742u,
    8185759u, 1066064u, 1066064u, 1066078u, 1066064u, 1066064u, 1107871u,
    3991519u, 4264976u, 4264976u, 4323230u, 4264976u, 4264976u, 4002847u,
    4515743u, 7148612u, 5575748u, 5633924u, 4527364u, 4527236u, 4527172u
);

fn pm_shop(s: SurfaceIn, aa: f32) -> SurfaceOut {
    var out=SurfaceOut(s.tint,s.n,.71,0.0,vec3<f32>(0.0));
    if abs(s.n.y)>.6 { out.albedo*=.67; return out; }
    if s.p.y>2.87 {
        let trim=pm_band(s.p.y-3.94,.09,aa)+pm_band(s.p.y-2.97,.065,aa);
        out.albedo*=.86+trim*.30;
        return out;
    }
    let horizontal=select(s.p.z,s.p.x,abs(s.n.z)>.5);
    let parcel=pm_parcel(s);
    let t=horizontal+parcel*.35;
    let bay=floor(t/4.7);
    let u=fract(t/4.7);
    let h=s.p.y;
    let room=pm_hash(vec2<f32>(bay,parcel));
    let opening=pm_band((u-.5)*4.7,2.02,aa)*pm_band(h-1.50,1.12,aa);
    if opening<.002 { return out; }
    let eye=select(s.dir.z,s.dir.x,abs(s.n.z)>.5);
    let back=fract(u+.10*eye);
    let transom=smoothstep(2.18,2.25,h);
    let frosted=mix(vec3<f32>(.11,.23,.25),vec3<f32>(.44,.48,.41),transom);
    let warm=mix(vec3<f32>(.22,.12,.073),vec3<f32>(.52,.31,.16),room);
    var interior=mix(warm,frosted,.45+transom*.35);
    // A hanging poster, a display shelf, a ledge and the inner door frame.
    let poster=pm_band((back-.72)*4.7,.47,aa)*pm_band(h-1.55,.49,aa);
    let poster_ink=pm_band(h-1.73,.12,aa)+pm_band(h-1.38,.035,aa);
    let accent=mix(vec3<f32>(.66,.34,.19),vec3<f32>(.18,.44,.39),step(.52,room));
    interior=mix(interior,mix(accent,vec3<f32>(.72,.68,.52),poster_ink),poster);
    let shelf=pm_band(h-.79,.06,aa)*step(.29,back);
    let bottle=pm_band((fract(back*10.0)-.5)*.47,.11,aa)*pm_band(h-.96,.135,aa)*step(.32,back);
    interior=mix(interior,vec3<f32>(.14,.17,.13),shelf);
    interior=mix(interior,accent*.77,bottle);
    let frame=max(pm_band((u-.31)*4.7,.043,aa),pm_band(h-2.20,.044,aa));
    let handle=pm_band((u-.37)*4.7,.032,aa)*pm_band(h-1.12,.21,aa);
    out.albedo=mix(s.tint,mix(interior,vec3<f32>(.22,.25,.23),max(handle,frame)),opening);
    let lightstrip=pm_band(h-2.49,.041,aa)*pm_band((u-.64)*4.7,.78,aa);
    let occupied=.52+.48*step(.22,room);
    out.emission=opening*(1.0-frame)*(interior*(4.0+8.0*occupied)+vec3<f32>(33.0,26.0,15.0)*lightstrip);
    out.rough=mix(.72,.34,opening);
    return out;
}

// The distant podium uses the same sills, stiles and opening extents as
// room.module.luau. Up close, separate optical panes replace the distant glass.
fn pm_masonry(s: SurfaceIn, aa: f32, brick: bool) -> SurfaceOut {
    if abs(s.n.y)>.5 { return SurfaceOut(s.tint,s.n,.75,0.0,vec3<f32>(0.0)); }
    let t=select(s.p.z,s.p.x,abs(s.n.z)>.5);
    let plinth=1.0-smoothstep(.37-aa,.37+aa,s.p.y);
    var out=pm_wall_stone(s,vec2<f32>(t,s.p.y),brick && plinth<.5,aa);
    let limestone=mix(s.tint,vec3<f32>(.54,.48,.40),.32);
    out.albedo=mix(out.albedo,out.albedo*.68,plinth);
    let ledge=max(pm_band(s.p.y-3.98,.085,aa),pm_band(s.p.y-2.63,.058,aa));
    out.albedo=mix(out.albedo,limestone*1.14,ledge);
    if !SURFACE_LOW_DETAIL {
        let drip=max(pm_band(s.p.y-3.865,.028,aa),pm_band(s.p.y-2.54,.021,aa));
        out.albedo*=1.0-.25*drip;
        out.normal=normalize(mix(out.normal,s.n+vec3<f32>(0,.22,0),ledge));
    }
    return out;
}

// Analytic ground-floor LOD. Only visible openings trace a ray. Coordinates
// are in the same room space as room.module.luau, mirrored once per lot.
struct PmRoomSample {
    distance: f32,
    color: vec3<f32>,
};

fn pm_furniture_hit(current: PmRoomSample, origin: vec3<f32>, inv: vec3<f32>,
                    center: vec3<f32>, half_size: vec3<f32>, color: vec3<f32>) -> PmRoomSample {
    let a=(center-half_size-origin)*inv;
    let b=(center+half_size-origin)*inv;
    let entry=min(a,b);
    let exit_t=max(a,b);
    let t=max(max(entry.x,entry.y),entry.z);
    if t<=0.0 || t>=current.distance || t>min(min(exit_t.x,exit_t.y),exit_t.z) { return current; }
    let facing=select(select(.65,.82,entry.x>entry.z),1.04,entry.y>=max(entry.x,entry.z));
    return PmRoomSample(t,color*facing);
}

// A vertical card at each activity anchor shares the furniture depth test.
// Only rays within a person's footprint evaluate the rounded silhouette.
fn pm_limb(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, radius: f32) -> f32 {
    let v=b-a;
    return length(p-a-v*clamp(dot(p-a,v)/max(dot(v,v),.0001),0.0,1.0))-radius;
}

fn pm_room_person(hit: PmRoomSample, origin: vec3<f32>, dir: vec3<f32>, across: vec2<f32>,
                  travel_axis: vec2<f32>, feet: vec2<f32>, facing: vec2<f32>, seated: bool,
                  identity: vec2<f32>, aa: f32, fade: f32) -> PmRoomSample {
    let delta=feet-origin.xz;
    let travel=dot(delta,travel_axis);
    if travel<=0.0 || travel>=hit.distance { return hit; }
    let at=vec2<f32>(-dot(delta,across),origin.y+dir.y*travel);
    if abs(at.x)>.68+aa || at.y<0.0 || at.y>1.99+aa { return hit; }

    let seed=pm_hash(identity);
    let p=at/(.96+.10*seed);
    let side=dot(facing,across);
    let width=.20-.064*abs(side);
    let hip=select(.90,.52,seated);
    let shoulder=vec2<f32>(side*.025,hip+.49);
    let head=shoulder+vec2<f32>(side*.025,.275);
    let knee=select(0.0,.38*side,seated);
    let offset=mix(knee,0.0,clamp((p.y-.42)/(hip-.42),0.0,1.0));
    let leg=pm_limb(vec2<f32>(abs(p.x-offset),p.y),vec2<f32>(width*.60,.08),vec2<f32>(width*.47,hip),.082);
    let torso=pm_limb(p,vec2<f32>(0.0,hip+.028),shoulder,width*.88);
    let arm=pm_limb(vec2<f32>(abs(p.x-shoulder.x),p.y),vec2<f32>(width,.04+hip),
                    vec2<f32>(width*.94,shoulder.y-.015),.066);
    let face=length((p-head)*vec2<f32>(1.13,.87))-.132;
    let outline=min(min(torso,arm),min(face,leg));
    let coverage=1.0-smoothstep(-aa,aa,outline);
    if coverage<=0.0 { return hit; }

    let shirts=array<vec3<f32>,6>(vec3<f32>(.11,.24,.33),vec3<f32>(.56,.21,.12),
        vec3<f32>(.12,.29,.20),vec3<f32>(.52,.48,.37),vec3<f32>(.30,.16,.25),vec3<f32>(.17,.19,.23));
    let shirt=shirts[u32(seed*6.0)];
    let skin=mix(vec3<f32>(.23,.11,.062),vec3<f32>(.70,.41,.26),fract(seed*7.13));
    var color=mix(vec3<f32>(.070,.086,.11),vec3<f32>(.20,.17,.13),fract(seed*5.61));
    if torso<=leg || arm<=leg {
        color=select(shirt,skin,arm<torso && p.y<hip+.22);
    }
    if face<min(min(torso,arm),leg) {
        color=select(skin,skin*.24,p.y>head.y+.056);
    }
    let shading=(.52+.48*exp(-.095*travel))*(.88-.12*clamp((p.x-shoulder.x)/width,-1.0,1.0));
    return PmRoomSample(travel,mix(hit.color,color*shading,coverage*fade));
}

fn pm_room_view(s: SurfaceIn, size: vec3<f32>, archetype: u32, variant: u32, aa: f32) -> vec3<f32> {
    let mirror=select(1.0,-1.0,(variant&1u)!=0u);
    let axis=vec3<f32>(mirror,1.0,1.0);
    let origin=vec3<f32>(s.local.x*size.x,s.p.y,s.local.z*size.z)*axis;
    let dir=s.dir*axis;
    // Clamping grazing components keeps parallel slab tests finite on WebGPU.
    let inv=select(vec3<f32>(1.0),vec3<f32>(-1.0),dir<vec3<f32>(0.0))/max(abs(dir),vec3<f32>(.00001));
    let lo=vec3<f32>(-.5*size.x+.23,.024,-.5*size.z+.23);
    let hi=vec3<f32>(.5*size.x-.23,size.y-.17,.5*size.z-.23);
    let exits=max((lo-origin)*inv,(hi-origin)*inv);
    let travel=max(0.0,min(min(exits.x,exits.y),exits.z));
    let p=origin+dir*travel;
    let hotel=archetype==1u;
    let warehouse=archetype==2u;
    var wall=select(vec3<f32>(.51,.49,.40),vec3<f32>(.58,.42,.31),hotel);
    if warehouse { wall=vec3<f32>(.36,.42,.40); }
    var c=wall*select(.70,1.0,exits.z<exits.x);
    if exits.y<=min(exits.x,exits.z) {
        if dir.y<0.0 {
            let edge=abs(fract(p.xz/.85+.5)-.5)*.85;
            let joint=pm_band(min(edge.x,edge.y),.012,aa);
            c=select(vec3<f32>(.22,.27,.26),vec3<f32>(.53,.45,.34),hotel)*(1.0-.18*joint);
            if warehouse { c=vec3<f32>(.33,.35,.33); }
        } else {
            c=vec3<f32>(.47,.45,.40);
            let panel=abs(abs(p.xz)-size.xz*vec2<f32>(.23,.22));
            if panel.x<.75 && panel.y<.30 { c=vec3<f32>(2.3,2.05,1.7); }
        }
    } else if p.z<lo.z+.015 && abs(p.x)<select(2.4,1.3,hotel) && abs(p.y-2.1)<.78 {
        c=select(vec3<f32>(.11,.29,.25),vec3<f32>(.49,.22,.12),hotel);
    }
    var hit=PmRoomSample(travel,c);
    let wood=select(vec3<f32>(.39,.19,.081),vec3<f32>(.225,.107,.057),variant>=6u);
    let upholstery=array<vec3<f32>,3>(vec3<f32>(.070,.26,.24),vec3<f32>(.49,.16,.085),vec3<f32>(.099,.16,.25));
    let fabric=upholstery[(variant/2u)%3u];
    // Furniture converges to the room shell as its silhouettes become subpixel.
    let detail_weight=1.0-smoothstep(.20,.36,aa);
    if detail_weight>0.0 {
        if warehouse {
            for (var side=-1; side<=1; side+=2) {
                let x=f32(side)*size.x*.29;
                let z=-size.z*.05;
                if SURFACE_LOW_DETAIL {
                    hit=pm_furniture_hit(hit,origin,inv,vec3<f32>(x,1.20,z),vec3<f32>(.93,1.20,size.z*.24),vec3<f32>(.40,.27,.14));
                    continue;
                }
                hit=pm_furniture_hit(hit,origin,inv,vec3<f32>(x,1.87,z),vec3<f32>(1.15,.085,size.z*.30),vec3<f32>(.43,.16,.085));
                for (var bay=-1; bay<=1; bay+=2) {
                    let crate_z=z+f32(bay)*size.z*.145;
                    hit=pm_furniture_hit(hit,origin,inv,vec3<f32>(x,1.4,crate_z),vec3<f32>(.85,1.12,size.z*.073),vec3<f32>(.46,.31,.16));
                }
            }
        } else {
            let desk_x=-size.x*select(.26,.285,hotel);
            let desk_z=size.z*select(.235,.23,hotel);
            let desk_w=select(4.8,size.x*.26,hotel);
            hit=pm_furniture_hit(hit,origin,inv,vec3<f32>(desk_x,.46,desk_z),vec3<f32>(desk_w*.5-.11,.44,.47),wood);
            if !SURFACE_LOW_DETAIL {
                hit=pm_furniture_hit(hit,origin,inv,vec3<f32>(desk_x,1.40,desk_z),vec3<f32>(.365,.215,.045),vec3<f32>(.043,.10,.10));
            }
            hit=pm_furniture_hit(hit,origin,inv,vec3<f32>(desk_x,.955,desk_z),vec3<f32>(desk_w*.5,.055,.56),vec3<f32>(.61,.55,.42));
            let sofa=vec3<f32>(size.x*select(.29,.34,hotel),.39,size.z*select(.23,.21,hotel));
            hit=pm_furniture_hit(hit,origin,inv,sofa,vec3<f32>(.515,.37,select(2.20,1.40,hotel)),fabric);
            let back=select(vec2<f32>(-.26,-.25),vec2<f32>(.265,-.32),hotel)*size.xz;
            hit=pm_furniture_hit(hit,origin,inv,vec3<f32>(back.x,.62,back.y),select(vec3<f32>(2.20,.34,.825),vec3<f32>(1.11,.24,1.56),hotel),select(wood,vec3<f32>(.70,.65,.51),hotel));
        }
    }
    let shaded_wall=c*(.52+.48*exp(-.095*travel));
    let shaded_hit=hit.color*(.52+.48*exp(-.095*hit.distance));
    let background=mix(shaded_wall,shaded_hit,detail_weight);
    // A person remains readable after individual furniture edges become subpixel.
    let fade=1.0-smoothstep(.22,.46,aa);
    let planar=dot(dir.xz,dir.xz);
    let absent=u32(round(clamp(s.metal,0.0,1.0)*7.0));
    if fade<=0.0 || planar<.001 || absent==7u { return background; }
    let across=vec2<f32>(-dir.z,dir.x)*inverseSqrt(planar);
    let travel_axis=dir.xz/planar;
    let lot=round(s.p.xz-s.local.xz*size.xz);
    var people=PmRoomSample(hit.distance,background);
    // The same three fixed activities as the streamed room. The bit mask is
    // shared with population.module.luau and follows its per-lot absences.
    for (var i=0u; i<3u; i++) {
        if (absent & (1u<<i))!=0u { continue; }
        var feet=vec2<f32>(0.0);
        var facing=vec2<f32>(0.0,1.0);
        var seated=false;
        if warehouse {
            feet=vec2<f32>(-size.x*.29+select(1.84,3.0,i==1u),size.z*select(.37,.41,i==1u));
            facing=vec2<f32>(-1.0,0.0);
            if i==2u { feet=vec2<f32>(2.3,-size.z*.41); facing=vec2<f32>(0.0,1.0); }
        } else {
            let desk=size.xz*select(vec2<f32>(-.26,.235),vec2<f32>(-.285,.23),hotel);
            feet=desk+vec2<f32>(0.0,select(-.93,1.0,i==1u));
            if i==1u { facing.y=-1.0; }
            if i==2u {
                feet=size.xz*select(vec2<f32>(.29,.23),vec2<f32>(.34,.21),hotel)+vec2<f32>(-.12,select(0.0,.55,hotel));
                facing=vec2<f32>(-1.0,0.0);
                seated=true;
            }
        }
        people=pm_room_person(people,origin,dir,across,travel_axis,feet,facing,seated,
                              lot+vec2<f32>(f32(i)*17.0,6.0),aa,fade);
    }
    return people.color;
}

// Coarse reflected silhouettes of the 56 m lots in world.module.luau.
// Only glass calls this. The road and sky radiance are supplied by the caller.
struct PmCityHit { travel: f32, normal: vec3<f32>, kind: u32 };

fn pm_city_box(hit: PmCityHit, origin: vec3<f32>, inv: vec3<f32>, center: vec2<f32>,
               half_size: vec2<f32>, height: f32, kind: u32) -> PmCityHit {
    let lo=vec3<f32>(center.x-half_size.x,0.0,center.y-half_size.y);
    let hi=vec3<f32>(center.x+half_size.x,height,center.y+half_size.y);
    let a=(lo-origin)*inv;
    let b=(hi-origin)*inv;
    let entry=min(a,b);
    let leaves=max(a,b);
    let t=max(max(entry.x,entry.y),entry.z);
    if t<=0.0 || t>=hit.travel || t>min(min(leaves.x,leaves.y),leaves.z) { return hit; }
    var normal=vec3<f32>(0.0,0.0,-sign(inv.z));
    if entry.x>=entry.z { normal=vec3<f32>(-sign(inv.x),0.0,0.0); }
    if entry.y>=max(entry.x,entry.z) { normal=vec3<f32>(0.0,-sign(inv.y),0.0); }
    return PmCityHit(t,normal,kind);
}

fn pm_reflected_city(s: SurfaceIn, ray: vec3<f32>, aa: f32, background: vec3<f32>) -> vec3<f32> {
    let origin=s.p+s.n*.045;
    let signs=select(vec3<f32>(-1.0),vec3<f32>(1.0),ray>=vec3<f32>(0.0));
    let inv=signs/max(abs(ray),vec3<f32>(.00001));
    var cell=floor(origin.xz/56.0);
    let source=cell;
    let stride=signs.xz;
    var next=(56.0*(cell+max(stride,vec2<f32>(0.0)))-origin.xz)*inv.xz;
    let limit=min(138.0,select(138.0,max(0.0,-origin.y*inv.y),ray.y<-.00001));
    var hit=PmCityHit(limit,vec3<f32>(0.0),0u);
    for (var i=0u; i<4u; i++) {
        if any(cell<vec2<f32>(-3.0)) || any(cell>vec2<f32>(2.0)) { break; }
        let center=(cell+.5)*56.0;
        let own=all(cell==source);
        let pad=(cell.x==2.0 && cell.y==0.0) || (cell.x==-3.0 && cell.y==-2.0);
        let park=(cell.x==-2.0 && cell.y==1.0) || (cell.x==1.0 && cell.y==-2.0);
        if !pad && !park {
            if all(cell==vec2<f32>(0.0)) {
                hit=pm_city_box(hit,origin,inv,center,vec2<f32>(1.45),7.4,4u);
            } else if cell.x==2.0 {
                if !own { hit=pm_city_box(hit,origin,inv,center-vec2<f32>(5.0,3.0),vec2<f32>(11.0,11.5),7.2,3u); }
            } else if abs(cell.x)<=1.0 && (cell.y==-1.0 || cell.y==0.0) {
                let height=select(38.0,57.2,all(cell==vec2<f32>(-1.0)));
                if !own { hit=pm_city_box(hit,origin,inv,center,vec2<f32>(13.0),height,1u); }
            } else {
                // Two narrow hotels share a lot. A side pane can see its neighbour.
                for (var side=-1; side<=1; side+=2) {
                    let offset=f32(side)*9.0;
                    if own && (origin.x-center.x)*offset>=0.0 { continue; }
                    let hotel=center+vec2<f32>(offset,0.0);
                    let height=select(9.7,12.8,side>0);
                    hit=pm_city_box(hit,origin,inv,hotel,vec2<f32>(6.9,11.5),height,2u);
                }
            }
        }
        if hit.kind!=0u || min(next.x,next.y)>=limit { break; }
        let advance=vec2<bool>(next.x<=next.y,next.y<=next.x);
        cell+=select(vec2<f32>(0.0),stride,advance);
        next+=select(vec2<f32>(0.0),abs(inv.xz)*56.0,advance);
    }
    if hit.kind==0u { return background; }

    let pos=origin+ray*hit.travel;
    let normal=hit.normal;
    let footprint=aa*(1.0+hit.travel/max(distance(view.cam.xyz,s.p),1.0))/max(abs(dot(normal,ray)),.10);
    var paint=vec3<f32>(.22,.30,.32);
    if hit.kind==2u { paint=vec3<f32>(.58,.39,.29); }
    if hit.kind==3u { paint=vec3<f32>(.34,.37,.32); }
    if hit.kind==4u { paint=vec3<f32>(.58,.54,.42); }
    var lamps=vec3<f32>(0.0);
    if hit.kind<=2u && normal.y==0.0 {
        let along=select(pos.z,pos.x,abs(normal.z)>.5);
        let pitch=vec2<f32>(select(2.3,2.85,hit.kind==2u),3.1);
        let grid=vec2<f32>(along,pos.y)/pitch;
        let edge=(fract(grid)-.5)*pitch;
        let width=select(.45,.30,hit.kind==2u);
        let blur=clamp(footprint,.03,.6);
        let panes=pm_band(edge.x,pitch.x*width,blur)*pm_band(edge.y,1.0,blur);
        let resolved=1.0-smoothstep(.35,1.3,footprint);
        let mean=2.0*width*2.0/pitch.y;
        let lit=step(.73,pm_hash(floor(grid)+cell*13.0));
        paint=mix(paint,vec3<f32>(.06,.12,.14),mix(mean,panes,resolved));
        lamps=vec3<f32>(.94,.61,.31)*14.0*light.anim.y*mix(.27*mean,lit*panes,resolved);
    }
    let daylight=sky_irradiance(normal)*.78+light.sun_color.rgb*light.sun_dir.w*max(dot(normal,light.sun_dir.xyz),0.0);
    let radiance=paint*daylight/PI+lamps;
    return mix(radiance,background,smoothstep(100.0,138.0,hit.travel));
}

// One reflected block on the low profile: no city traversal, sky texture or pavement samples.
fn pm_low_reflection(s: SurfaceIn, ray: vec3<f32>) -> vec3<f32> {
    let up=vec3<f32>(0.0,1.0,0.0);
    let sky=sky_irradiance(up)/PI;
    let sun=light.sun_color.rgb*light.sun_dir.w/PI;
    let ground=.18*(sky+sun*max(light.sun_dir.y,0.0));
    let env=mix(ground,sky*(.85+.7*max(ray.y,0.0)),smoothstep(-.04,.10,ray.y));
    let cell=floor(s.p.xz/56.0)+round(s.n.xz);
    if any(cell<vec2<f32>(-3.0)) || any(cell>vec2<f32>(2.0)) { return env; }
    if all(cell==vec2<f32>(-2,1)) || all(cell==vec2<f32>(1,-2)) || all(cell==vec2<f32>(2,0)) ||
       all(cell==vec2<f32>(-3,-2)) || all(cell==vec2<f32>(0)) { return env; }
    let center=(cell+.5)*56.0;
    let office=abs(cell.x)<=1.0 && (cell.y==-1.0 || cell.y==0.0);
    let height=select(select(11.0,7.2,cell.x==2.0),38.0,office);
    let inv=select(vec3<f32>(-1.0),vec3<f32>(1.0),ray>=vec3<f32>(0.0))/max(abs(ray),vec3<f32>(.00001));
    let limit=select(100.0,min(100.0,-s.p.y*inv.y),ray.y<-.00001);
    let hit=pm_city_box(PmCityHit(limit,vec3<f32>(0.0),0u),s.p+s.n*.045,inv,center,
                        vec2<f32>(select(16.0,13.0,office),12.0),height,1u);
    if hit.kind==0u { return env; }
    return vec3<f32>(.22,.25,.24)*(sky*.69+sun*max(dot(hit.normal,light.sun_dir.xyz),0.0));
}

// Directional radiance for the optical layer of the distant storefront.
fn pm_window_environment(s: SurfaceIn, ray: vec3<f32>, aa: f32) -> vec3<f32> {
    let up=vec3<f32>(0.0,1.0,0.0);
    let light_on_ground=(sky_irradiance(up)+light.sun_color.rgb*light.sun_dir.w*max(light.sun_dir.y,0.0))/PI;
    var ground=vec3<f32>(.19,.19,.17);
    if ray.y<-.004 {
        let travel=s.p.y/-ray.y;
        if travel<140.0 {
            let pos=s.p.xz+ray.xz*travel;
            // The hit footprint grows quickly as the mirror ray skims the pavement.
            let footprint=aa*(1.0+travel/max(distance(view.cam.xyz,s.p),1.0))/max(-ray.y,.015);
            let paving=pm_street(pos,vec3<f32>(.120,.133,.144),clamp(footprint,.02,.5));
            let resolved=(1.0-smoothstep(.35,1.8,footprint))*(1.0-smoothstep(95.0,140.0,travel));
            ground=mix(ground,paving,resolved);
        }
    }
    let horizon=smoothstep(-.04,.035,ray.y);
    if horizon<=0.0 { return ground*light_on_ground; }
    let sky=min(sky_radiance(ray),vec3<f32>(gi.params.z));
    return mix(ground*light_on_ground,sky,horizon);
}

fn pm_opening(s: SurfaceIn, size: vec3<f32>, archetype: u32, variant: u32,
              across: f32, width: f32, divisions: f32, glazed: bool, transom: bool, aa: f32) -> SurfaceOut {
    let interior=pm_room_view(s,size,archetype,variant,aa);
    let lod=smoothstep(2.0,11.0,distance(view.cam.xyz,s.p));
    // Warm ceiling lighting reaches the room even when the street is dark.
    // Its radiance is in nits, shared by the door and the view through glass.
    let lamplight=vec3<f32>(.43)+lod*vec3<f32>(4.8,4.0,3.1);
    // Diffuse light entering the opening scales with the actual outdoor sky.
    let street_bounce=light.sun_color.rgb*light.sun_dir.w*max(light.sun_dir.y,0.0)*.067;
    let daylight=lod*(.24*sky_irradiance(s.n)+street_bounce)/(PI*max(light.anim.y,.001));
    var out=SurfaceOut(interior*.064,s.n,.87,0.0,interior*(lamplight+daylight));
    if glazed {
        let cosine=clamp(abs(dot(s.dir,s.n)),0.0,1.0);
        // Coated shopfront glass. Combine both faces of a thin pane,
        // including internal reflections: R = 2F/(1+F) before absorption.
        let fresnel=.06+.94*pow(1.0-cosine,5.0);
        let reflection=lod*(2.0*fresnel)/(1.0+fresnel);
        let ray=reflect(s.dir,s.n);
        var reflected: vec3<f32>;
        if SURFACE_LOW_DETAIL { reflected=pm_low_reflection(s,ray); }
        else { reflected=pm_reflected_city(s,ray,aa,pm_window_environment(s,ray,aa)); }
        // The smoke tint absorbs light coming through the pane.
        let transmission=.80*(1.0-reflection);
        out.albedo*=transmission;
        out.emission=out.emission*transmission+reflected*reflection/max(light.anim.y,.001);
        // The opaque material models the room behind the pane. Its rough
        // lobe keeps the full renderer from adding another polished surface.
        out.rough=.87;
        let cell=width/divisions;
        let stile=pm_band((fract(across/cell+.5)-.5)*cell,.034,aa);
        let rail=select(0.0,pm_band(s.p.y-2.17,.034,aa),transom);
        let frame=max(stile,rail);
        out.albedo=mix(out.albedo,vec3<f32>(.052,.083,.089),frame);
        out.emission*=1.0-frame;
        out.rough=mix(out.rough,.68,frame);
    }
    return out;
}

fn pm_ground_floor(s: SurfaceIn, aa: f32) -> SurfaceOut {
    // Kinds 20â€“22 are the original, unparameterized ground floors.
    let encoded=select(s.kind-20u,s.kind-64u,s.kind>=64u);
    let archetype=encoded%3u;
    let variant=encoded/3u;
    let warehouse=archetype==2u;
    let hotel=archetype==1u;
    var out=pm_masonry(s,aa,hotel);
    if warehouse {
        var face=s;
        face.kind=3u;
        out=pm_facade(face,aa);
    }
    if abs(s.n.y)>.6 || s.n.z<-.8 { return out; }
    let inst=instance_at(s.instance);
    let size=abs(vec3<f32>(inst.r0.x,inst.r1.y,inst.r2.z));
    if abs(s.n.x)>.8 {
        let span=select(select(7.4,5.6,hotel),7.2,warehouse);
        let sill=select(select(.72,.85,hotel),2.45,warehouse);
        let u=s.local.z*size.z-select(1.4,0.0,warehouse)+span*.5;
        if u>=0.0 && u<=span && s.p.y>=sill && s.p.y<=3.58 {
            return pm_opening(s,size,archetype,variant,u,span,2.0,true,false,aa);
        }
        return out;
    }
    if s.p.y>2.50 { return out; }
    let x=abs(s.local.x)*size.x;
    if x<=1.40 {
        return pm_opening(s,size,archetype,variant,x,2.8,1.0,false,false,aa);
    }
    let inner=select(1.76,2.24,warehouse);
    let outer=select(.5*size.x-.66,4.78,warehouse);
    let sill=select(select(.47,.60,hotel),1.02,warehouse);
    if s.p.y<sill || x>outer { return out; }
    if x<inner { out.albedo=vec3<f32>(.052,.083,.089); return out; }
    return pm_opening(s,size,archetype,variant,x-inner,outer-inner,select(3.0,2.0,hotel||warehouse),true,true,aa);
}

fn pm_interior(s: SurfaceIn, aa: f32) -> SurfaceOut {
    var out=SurfaceOut(s.tint,s.n,.76,0.0,vec3<f32>(0.0));
    let horizontal=select(s.p.x,s.p.z,abs(s.n.x)>.5);
    if s.kind==24u {
        let p=s.p.xz/.85;
        let edges=abs(fract(p+.5)-.5)*.85;
        let grout=pm_band(min(edges.x,edges.y),.015,aa);
        let stone=pm_hash(floor(p));
        out.albedo=mix(s.tint*(.87+.17*stone),s.tint*.52,grout);
        out.rough=.58;
    } else if s.kind==25u {
        let cross=select(s.p.y,s.p.z,abs(s.n.y)>.5);
        let grain=sin(cross*78.0+.75*sin(horizontal*2.4)+.45*sin(cross*13.0));
        let joint=pm_band((fract(cross/.19+.5)-.5)*.19,.005,aa);
        out.albedo=s.tint*(.94+.07*grain*(1.0-smoothstep(.01,.07,aa))-.16*joint);
        out.rough=.58;
    } else if s.kind==26u {
        if s.p.y<.94 {
            let stile=pm_band((fract(horizontal/.72+.5)-.5)*.72,.019,aa);
            let rail=pm_band(s.p.y-.88,.055,aa);
            let base=1.0-smoothstep(.11-aa,.11+aa,s.p.y);
            out.albedo=mix(vec3<f32>(.30,.19,.12),s.tint,.22)*(.84+.26*max(stile,max(rail,base)));
        }
    } else if s.kind==27u {
        let coord=select(s.local.x,s.local.z,abs(s.n.x)>.5);
        let cross=select(s.local.y,s.local.z,abs(s.n.y)>.5);
        let edge_aa=clamp(aa*.6,.004,.1);
        let slat=pm_band((fract(cross*4.0+.5)-.5)*.25,.009,edge_aa);
        let strap=pm_band(abs(coord)-.33,.027,edge_aa);
        out.albedo=mix(s.tint*(.96-.24*slat),vec3<f32>(.15,.21,.21),strap);
        if abs(s.n.y)<.5 && abs(coord-.08)<.13 && abs(cross-.10)<.10 {
            let code=pm_band((fract(coord*58.0)-.5)/58.0,.004,edge_aa);
            out.albedo=mix(vec3<f32>(.83,.78,.60),vec3<f32>(.25,.27,.24),
                code*(1.0-smoothstep(.045,.055,abs(cross-.10))));
        }
    } else {
        // Framed travel print: sunset, a little skyline and the ferry wake.
        let u=select(s.local.x,s.local.z,abs(s.n.x)>.5);
        let uv=vec2<f32>(u,s.local.y);
        let frame=max(abs(u),abs(uv.y));
        out.albedo=vec3<f32>(.20,.14,.10);
        if abs(s.n.y)<.5 && frame<.476 {
            out.albedo=vec3<f32>(.88,.80,.64);
            if abs(u)<.427 && abs(uv.y)<.397 {
                let sky=mix(vec3<f32>(.91,.61,.38),vec3<f32>(.36,.63,.62),uv.y+.53);
                let sun=1.0-smoothstep(.093,.101,length(uv-vec2<f32>(.18,.16)));
                out.albedo=mix(sky,vec3<f32>(.99,.80,.43),sun);
                let towers=pm_hash(vec2<f32>(floor(u*15.0),7.0))*.17-.08;
                if uv.y<towers { out.albedo=vec3<f32>(.18,.33,.34); }
                if uv.y<-.12 {
                    let wake=pm_band(uv.y+.21+.028*sin(u*12.0),.010,.006);
                    out.albedo=mix(vec3<f32>(.26,.48,.48),vec3<f32>(.72,.67,.47),wake);
                }
            }
        }
    }
    return out;
}

// Smooth patches of ash cling to the chassis while it moves and loose panels tumble.
fn pm_scorch_noise(p: vec2<f32>) -> f32 {
    let cell=floor(p);
    let f=fract(p);
    let w=f*f*(3.0-2.0*f);
    return mix(mix(pm_hash(cell),pm_hash(cell+vec2<f32>(1,0)),w.x),
               mix(pm_hash(cell+vec2<f32>(0,1)),pm_hash(cell+vec2<f32>(1,1)),w.x),w.y);
}

fn pm_scorch(s: SurfaceIn, aa: f32) -> SurfaceOut {
    let p=vec2<f32>(s.local.z+.53*s.local.x,s.local.y+.81*s.local.x);
    let patches=pm_scorch_noise(p*3.2);
    let flecks=(pm_scorch_noise(p*18.0)-.5)*(1.0-smoothstep(.022,.10,aa));
    let ash=smoothstep(.23,.83,patches+.43*flecks);
    let color=s.tint*mix(.44,1.47,ash);
    return SurfaceOut(color,s.n,mix(.97,s.rough,ash),s.metal*ash,vec3<f32>(0));
}

// The collider and the baked fallback share these broad land-cover regions.
// Surface shading blends through triangulation edges and resolves finer growth
// only where the screen footprint can show it.
// Small coastal fields on either side of the grain elevator. The grass margins
// leave space at the ends of the rows for machinery to turn.
fn pm_cape_fields(p: vec2<f32>, grass: vec3<f32>, aa: f32) -> vec3<f32> {
    if p.x < -460.0 || p.x > -193.0 || abs(p.y)>150.0 { return grass; }
    let x=-p.x-193.0;
    let z=abs(p.y)-14.0-.085*x;
    if z<0.0 { return grass; }
    let cross=z+3.0*sin(x*.028);
    let bed=vec2<f32>((x+select(21.0,0.0,p.y>0.0))/68.0,cross/43.0);
    let within=fract(bed);
    let toEdge=min(within,1.0-within)*vec2<f32>(68.0,43.0);
    let outer=min(min(x,267.0-x),min(z,120.0-z));
    let cover=smoothstep(1.4,3.4,min(outer,min(toEdge.x,toEdge.y)));
    let crop=u32(floor(bed.x)+2.0*floor(bed.y)+select(1.0,0.0,p.y>0.0))%4u;
    let palette=array<vec3<f32>,4>(vec3<f32>(.32,.25,.115),vec3<f32>(.139,.099,.069),
        vec3<f32>(.128,.194,.072),vec3<f32>(.255,.215,.114));
    var soil=palette[crop];
    if !SURFACE_LOW_DETAIL {
        let planting=select(x+.16*cross,cross-.12*x,(crop&1u)==0u);
        let furrow=cos(planting*8.4);
        let planted=smoothstep(3.2,6.4,min(toEdge.x,toEdge.y));
        let coarse=cos(planting*.82+.23*sin(.07*x));
        soil*=1.0+.12*furrow*(1.0-smoothstep(.06,.26,aa))*planted+.06*coarse;
    }
    return mix(grass,soil,cover);
}

fn pm_cape_ground(s: SurfaceIn, aa: f32) -> SurfaceOut {
    let p=s.p.xz;
    let weather=sin(.015*p.x+.022*p.y)+.53*cos(.034*p.y-.010*p.x);
    let grade=length(s.n.xz)/max(.25,s.n.y);
    let damp=1.0-smoothstep(-.82,-.30,weather);
    let meadow=max(smoothstep(.81,1.25,weather),
        smoothstep(-585.0,-515.0,p.x)*smoothstep(-.42,.12,weather));
    let first=(p-vec2<f32>(-690,-99))/vec2<f32>(150,100);
    let second=(p-vec2<f32>(-900,108))/vec2<f32>(95,83);
    let woodland=1.0-smoothstep(.76,1.19,min(dot(first,first),dot(second,second))-.10*weather);
    let heath=max(smoothstep(13.0,19.0,s.p.y-2.0*weather),smoothstep(.17,.25,grade));
    let granite=max(smoothstep(.265,.35,grade-.045*weather),
        smoothstep(18.0,23.0,s.p.y-2.5*weather)*smoothstep(.09,.17,grade));
    var color=mix(vec3<f32>(.145,.205,.099),vec3<f32>(.10,.17,.078),damp);
    color=mix(color,vec3<f32>(.265,.256,.119),meadow);
    color=mix(color,vec3<f32>(.097,.13,.066),woodland);
    color=mix(color,vec3<f32>(.194,.159,.110),heath);
    color=mix(color,vec3<f32>(.297,.290,.255),granite);
    if !SURFACE_LOW_DETAIL {
        let tufts=pm_scorch_noise(p*.37)*.7+pm_scorch_noise(p*1.29)*.3;
        let striae=sin(dot(p,vec2<f32>(1.34,.59))+4.0*tufts);
        let swath=sin(.65*p.y+.074*p.x+1.25*sin(.033*p.x));
        color*=1.0+.30*(tufts-.5)+.082*meadow*swath+.12*granite*striae;
        let resolved=1.0-smoothstep(.028,.20,aa);
        if resolved>0.0 {
            let blade=pm_scorch_noise(p*vec2<f32>(6.4,3.7))-.5;
            color+=resolved*blade*mix(vec3<f32>(.073,.083,.021),vec3<f32>(.050,.048,.044),granite);
        }
    }
    return SurfaceOut(pm_cape_fields(p,color,aa),s.n,mix(.91,.81,granite),0.0,vec3<f32>(0.0));
}

// Stony trail faces use metre-scale course coordinates in the untextured mesh.
// The same wheel paths follow the complete switchback in either direction.
fn pm_gravel_chip(p: vec2<f32>, scale: f32, aa: f32) -> f32 {
    let uv=p*scale;
    let pick=pm_hash(floor(uv));
    let center=.28+.44*vec2<f32>(pick,fract(pick*17.31));
    let q=abs(fract(uv)-center);
    let radius=.12+.17*fract(pick*41.53);
    let distance=max(q.x+.42*q.y,.34*q.x+q.y);
    let edge=clamp(aa*scale,.04,.44);
    let coverage=1.0-smoothstep(radius-edge,radius+edge,distance);
    return coverage*(pick-.43);
}

fn pm_cape_gravel(s: SurfaceIn, aa: f32) -> SurfaceOut {
    let uv=s.uv0*s.bary.x+s.uv1*s.bary.y+s.uv2*s.bary.z;
    let across=(uv.x-.5)*8.0;
    let along=uv.y*1024.0;
    let p=s.p.xz;
    let footprint=max(length(s.dx),length(s.dy));
    let variation=pm_scorch_noise(p*.28);
    let wander=.17*sin(along*.083)+.09*sin(along*.31);
    let tracks=abs(abs(across-wander)-.83);
    let traffic=smoothstep(.10,.77,variation+.22*sin(along*.22));
    let compact=(1.0-smoothstep(.13,.46+aa,tracks))*traffic
        *(1.0-smoothstep(.34,1.3,footprint));
    let edge=2.56+.26*sin(along*.19)+.29*(variation-.5);
    let fringe=smoothstep(edge-.25-aa,edge+.23+aa,abs(across));
    let gravel=mix(vec3<f32>(.147,.121,.088),vec3<f32>(.294,.238,.168),variation);
    var color=mix(gravel,vec3<f32>(.155,.117,.080),compact*.72);
    var rough=.94-.12*compact;
    if !SURFACE_LOW_DETAIL {
        let resolved=1.0-smoothstep(.036,.145,footprint);
        let coarse=1.0-smoothstep(.17,.46,footprint);
        if coarse>0.0 {
            let grit=pm_scorch_noise(p*1.8)-.5;
            color+=vec3<f32>(.103,.087,.066)*grit*coarse;
        }
        if resolved>0.0 {
            let chips=pm_gravel_chip(p,6.3,aa)+.5*pm_gravel_chip(p.yx+12.3,11.1,aa);
            color+=vec3<f32>(.23,.22,.19)*chips*resolved*(1.0-.52*compact);
        }
    }
    if fringe>0.0 {
        let land=pm_cape_ground(s,aa);
        color=mix(color,land.albedo,fringe);
        rough=mix(rough,land.rough,fringe);
    }
    return SurfaceOut(color,s.n,rough,0.,vec3<f32>(0));
}

fn game_surface(s: SurfaceIn) -> SurfaceOut {
    let aa=clamp(max(length(s.dx),length(s.dy))*0.56,0.012,0.5);
    var out=SurfaceOut(s.tint,s.n,s.rough,s.metal,vec3<f32>(0.0));
    if s.kind==40u { return pm_cape_ground(s,aa); }
    if s.kind==41u { return pm_cape_gravel(s,aa); }
    if s.kind==220u { return pm_scorch(s,aa); }
    if s.kind==221u {
        // White-hot gas at the mouth, cooling to amber along the plume and lateral jets.
        let heat=1.-smoothstep(.08,.86,s.local.z+length(s.local.xy)*.85);
        let core=heat*heat;
        let fire=mix(vec3<f32>(4800.,620.,20.),vec3<f32>(49000.,28000.,6500.),core)
            +vec3<f32>(108000.,92000.,52000.)*core*core*core;
        return SurfaceOut(vec3<f32>(.18,.048,.01),s.n,1.,0.,fire);
    }
    if s.kind==31u {
        // The diffuser follows the street lighting transition at sunset and dawn.
        let dusk=1.0-smoothstep(-.057,.224,light.sun_dir.y);
        out.emission=vec3<f32>(82.0,61.0,39.0)*dusk;
        return out;
    }
    if s.kind==30u {
        // A steel-edged red case with a yellow and charcoal hazard band on all four sides.
        let u=select(s.local.x,s.local.z,abs(s.n.x)>.5);
        let band=pm_band(s.local.y-.08,.12,min(.08,aa));
        let stripe=smoothstep(.42,.58,fract((u+s.local.y*.7)*6.0));
        let paint=mix(vec3<f32>(.028,.035,.036),vec3<f32>(.96,.65,.08),stripe);
        let edge=smoothstep(.435,.475,max(abs(u),abs(s.local.y)));
        let side=mix(s.tint,paint,band);
        out.albedo=mix(side,vec3<f32>(.19,.20,.19),edge);
        out.metal=.32*edge;
        return out;
    }
    if s.kind==29u || s.kind==32u { return pm_masonry(s,aa,s.kind==32u); }
    if (s.kind>=20u && s.kind<=22u) || (s.kind>=64u && s.kind<100u) { return pm_ground_floor(s,aa); }
    if s.kind==23u {
        // Brass end caps on a narrow, warm welcome light.
        let strip=pm_band(s.local.x,.448,min(aa*.32,.08));
        let lamp=select(0.0,strip,s.n.z>.5 || s.n.y<-.5);
        out.albedo=mix(s.tint,vec3<f32>(.85,.72,.43),lamp);
        out.emission=vec3<f32>(8.0,5.2,2.4)*lamp;
        return out;
    }
    if s.kind>=24u && s.kind<=28u { return pm_interior(s,aa); }
    if s.kind==0u {
        out.albedo=vec3<f32>(.21,.25,.25);
        if s.n.y>=.5 {
            out.albedo=pm_street(s.p.xz,s.tint,aa);
            out=pm_road_iron(s.p.xz,aa,out);
        }
        return out;
    }
    if s.kind==1u || s.kind==2u || s.kind==3u || s.kind==8u { return pm_facade(s,aa); }
    if s.kind==5u {
        let p=s.p.xz;
        let footprint=max(length(s.dx.xz),length(s.dy.xz));
        let reach=length(p);
        let ripple=(1.0-smoothstep(.16,2.3,footprint))*(1.0-smoothstep(260.0,1100.0,reach));
        let phase=sin(p.y*.079+p.x*.051);
        let a=sin(dot(p,vec2<f32>(.67,.32))+phase*.84+s.time*.73);
        let b=sin(dot(p,vec2<f32>(-.29,.85))-phase*.6-s.time*.48);
        let haze=smoothstep(370.0,5300.0,reach);
        out.albedo=mix(s.tint,vec3<f32>(.41,.55,.56),haze)*(1.0+ripple*.025*(a+b));
        out.normal=normalize(vec3<f32>(.018*a*ripple,1.0,.024*b*ripple));
        out.rough=.43+haze*.3;
        out.metal=.04;
        return out;
    }
    if s.kind==6u {
        let local=s.local.xz*29.0;
        let ring=pm_band(length(local)-10.6,.24,aa);
        let h=max(pm_band(abs(local.x)-2.35,.39,aa)*pm_band(local.y,3.7,aa),
            pm_band(local.y,.39,aa)*pm_band(local.x,2.35,aa));
        out.albedo=mix(s.tint,vec3<f32>(0.85,0.81,0.61),max(ring,h));
        return out;
    }
    if s.kind==4u || s.kind==7u {
        let p=s.local.xz*37.0;
        let path=pm_band(min(abs(p.x+p.y),abs(p.x-p.y)),1.15,aa);
        let tile=pm_band((fract(s.p.x/1.4+0.5)-0.5)*1.4,.034,aa);
        if s.kind==4u {
            let mown=select(.92,1.03,fract((s.p.x+s.p.z*.35)/4.0)>.5);
            // The clipped turf underneath the blades also carries the lawn at
            // distance. Filter the fine colour variation as it becomes subpixel.
            let cut=vec2<f32>(s.p.x+s.p.z*.35,s.p.z-s.p.x*.35);
            let thatch=(pm_hash(floor(cut*vec2<f32>(42.,18.)))-.5)
                *(1.-smoothstep(.012,.055,aa));
            let turf=s.tint*(mown+.22*thatch);
            out.albedo=mix(turf,vec3<f32>(.57,.49,.34),path);
            out.rough=mix(.96,out.rough,path);
        } else {
            let ring=pm_band(length(p)-9.5,.32,aa);
            out.albedo=mix(s.tint*(1.0-.07*tile),vec3<f32>(.26,.40,.40),max(path,ring)*.65);
        }
        return out;
    }
    if s.kind==9u && abs(s.n.y)<.5 {
        let x=select(s.local.z,s.local.x,abs(s.n.z)>.5);
        let p=vec2<f32>(x*3.15,s.local.y*1.6);
        let dist=length(p);
        let face=1.0-smoothstep(.65-aa,.65+aa,dist);
        let ring=pm_band(dist-.66,.045,aa);
        let spokes=pm_band(sin(atan2(p.x,p.y)*6.0)*dist,0.034,aa)*smoothstep(.48,.54,dist);
        out.albedo=mix(s.tint,vec3<f32>(.90,.84,.64),face);
        out.albedo=mix(out.albedo,vec3<f32>(.08,.16,.17),max(ring,spokes*face));
        out.emission=vec3<f32>(1.2,.9,.45)*face;
        return out;
    }
    if s.kind==16u { return pm_shop(s,aa); }
    if s.kind==17u {
        let p=s.local.xz*1.8;
        let frame=pm_band(max(abs(p.x),abs(p.y))-.84,.046,aa);
        let hole=pm_band((fract((p.x+p.y)*5.0)-.5)*.2,.045,aa)*pm_band((fract((p.x-p.y)*5.0)-.5)*.2,.048,aa);
        out.albedo=mix(s.tint,vec3<f32>(.08,.10,.085),max(hole,.58*frame));
        out.rough=.83;
        return out;
    }
    if s.kind==18u {
        let stripe=pm_band((fract(s.p.z/.94)-.5)*.94,.23,aa);
        out.albedo=mix(s.tint,vec3<f32>(.74,.68,.50),stripe);
        return out;
    }
    if s.kind==19u {
        let p=s.local.zy;
        let ink=pm_band(p.x-.11,.028,aa)*pm_band(p.y,.36,aa)+pm_band(p.x+.08,.016,aa)*pm_band(p.y,.32,aa);
        let stops=pm_band(fract(p.y*5.0)-.5,.11,aa)*pm_band(p.x,.29,aa);
        out.albedo=mix(s.tint,vec3<f32>(.09,.25,.26),max(ink,.67*stops));
        out.emission=out.albedo*2.0;
        return out;
    }
    if s.kind>=10u && s.kind<=15u && abs(s.local.z)>.49 {
        let x=select(-s.local.x,s.local.x,s.local.z>0.0);
        let uv=(vec2<f32>(x,-s.local.y)+0.5)*vec2<f32>(27.0,9.0)-vec2<f32>(2.0,1.0);
        if all(uv>=vec2<f32>(0.0)) && all(uv<vec2<f32>(23.0,7.0)) {
            let bits=PM_SIGN_ROWS[(s.kind-10u)*7u+u32(uv.y)];
            if (bits & (1u << (22u-u32(uv.x))))!=0u {
                out.albedo=vec3<f32>(.96,.85,.57);
                out.emission=vec3<f32>(3.5,2.2,.95);
            }
        }
    }
    return out;
}
