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
    if street>6.0 || max(abs(p.x),abs(p.y))>174.0 {
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

// Each parcel keeps its window and display rhythm when the street-level
// shell streams into separate wall sections or a room is revisited.
fn pm_parcel(s: SurfaceIn) -> f32 {
    let lot=floor(s.p.xz/56.0);
    let half_lot=select(0.0,.37,s.kind==2u && s.p.x>(lot.x+.5)*56.0);
    return floor(pm_hash(lot+vec2<f32>(half_lot,0.0))*19.0);
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
        return out;
    }
    let hotel=s.kind==2u;
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
    let sill=pm_band((frac_cell.y-0.15)*pitch.y,0.095,aa);
    out.albedo=mix(s.tint*(0.92+sill*0.20),glass,window);
    out.rough=mix(0.77,0.28,window);
    out.metal=0.18*window;
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
fn pm_masonry(s: SurfaceIn, aa: f32) -> SurfaceOut {
    var out=SurfaceOut(s.tint,s.n,.75,0.0,vec3<f32>(0.0));
    if abs(s.n.y)>.5 { return out; }
    let t=select(s.p.z,s.p.x,abs(s.n.z)>.5);
    let row=floor(s.p.y/.51);
    let h=pm_band((fract(s.p.y/.51+.5)-.5)*.51,.009,aa);
    let joint=pm_band((fract(t/1.72+row*.5+.5)-.5)*1.72,.009,aa);
    let trim=pm_band(s.p.y-3.98,.072,aa)+pm_band(s.p.y-2.61,.039,aa);
    out.albedo*=.94-.15*max(h,joint)+.13*trim;
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
            hit=pm_furniture_hit(hit,origin,inv,vec3<f32>(desk_x,.955,desk_z),vec3<f32>(desk_w*.5,.055,.56),vec3<f32>(.61,.55,.42));
            hit=pm_furniture_hit(hit,origin,inv,vec3<f32>(desk_x,1.40,desk_z),vec3<f32>(.365,.215,.045),vec3<f32>(.043,.10,.10));
            let sofa=vec3<f32>(size.x*select(.29,.34,hotel),.39,size.z*select(.23,.21,hotel));
            hit=pm_furniture_hit(hit,origin,inv,sofa,vec3<f32>(.515,.37,select(2.20,1.40,hotel)),fabric);
            let back=select(vec2<f32>(-.26,-.25),vec2<f32>(.265,-.32),hotel)*size.xz;
            hit=pm_furniture_hit(hit,origin,inv,vec3<f32>(back.x,.62,back.y),select(vec3<f32>(2.20,.34,.825),vec3<f32>(1.11,.24,1.56),hotel),select(wood,vec3<f32>(.70,.65,.51),hotel));
        }
    }
    let shaded_wall=c*(.52+.48*exp(-.095*travel));
    let shaded_hit=hit.color*(.52+.48*exp(-.095*hit.distance));
    return mix(shaded_wall,shaded_hit,detail_weight);
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
    var out=SurfaceOut(interior*.064,s.n,.87,0.0,interior*lamplight);
    if glazed {
        let cosine=clamp(abs(dot(s.dir,s.n)),0.0,1.0);
        // Coated shopfront glass. Combine both faces of a thin pane,
        // including internal reflections: R = 2F/(1+F), T = 1-R.
        let fresnel=.16+.84*pow(1.0-cosine,5.0);
        let reflection=lod*(2.0*fresnel)/(1.0+fresnel);
        let ray=reflect(s.dir,s.n);
        let reflected=pm_reflected_city(s,ray,aa,pm_window_environment(s,ray,aa));
        let transmission=1.0-reflection;
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
    // Kinds 20–22 are the original, unparameterized ground floors.
    let encoded=select(s.kind-20u,s.kind-64u,s.kind>=64u);
    let archetype=encoded%3u;
    let variant=encoded/3u;
    let warehouse=archetype==2u;
    let hotel=archetype==1u;
    var out=pm_masonry(s,aa);
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

fn game_surface(s: SurfaceIn) -> SurfaceOut {
    let aa=clamp(max(length(s.dx),length(s.dy))*0.56,0.012,0.5);
    var out=SurfaceOut(s.tint,s.n,s.rough,s.metal,vec3<f32>(0.0));
    if s.kind==29u { return pm_masonry(s,aa); }
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
        if abs(s.n.y)>=.5 { out.albedo=pm_street(s.p.xz,s.tint,aa); }
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
