-- Stage recipes translated from SpaceRacer's MIT scenery.gd, city_layout.gd,
-- city_accents.gd, forest.gd, cell.gd, desert.gd and ocean.gd.
-- Geometry kernels, spatial queries and PCG stay native; composition lives here.
local V=vector.create
local Z=V(0,0,0)
local U=V(0,1,0)
local TAU=math.pi*2
local function R(a,b) return a+(b-a)*random() end
local function put(mesh,mat,p,s,r,solid) emit(mesh,mat,p,s,r or Z,solid or false) end
local function box(mat,p,s,solid) put("box",mat,p,s,Z,solid) end
local function part(mat,c,x,y,z,w,h,d) box(mat,c+V(x,y,z),V(w,h,d),true) end
local function line(mat,a,b,w) beam(mat,a,b,w) end
local function roof(b)
    local w,d,h=b.w,b.d,b.h
    local c=b.c+U*h
    if b.kind==0 then w*=.78;d*=.82
    elseif b.kind==3 then c+=U*h*.04;w*=.55;d*=.55
    elseif b.kind==7 then c+=V(-w*.29,0,0);w*=.42
    elseif b.kind==6 then c+=V(w*.21,0,-d*.12);w*=.48;d*=.65
    elseif b.kind==2 then w*=.54;d*=.54
    elseif b.kind==5 then w*=.42;d*=.55
    elseif b.kind==1 then c+=V(-w*.255,0,0);w*=.36;d*=.72
    elseif b.kind==4 then c+=V(w*.12,-h*.01,0);w*=.72;d*=.74 end
    local top=b.top;local m=b.mat
    if b.roof==0 then part("metal",c,w*.17,4,-d*.18,w*.26,8,d*.22)
    elseif b.roof==1 then part(m,c,w*.11,top*.5,d*.08,w*.65,top,d*.64);c+=U*top
    elseif b.roof==2 then part(m,c,-w*.21,top*.5,0,w*.34,top,d*.62);part("metal",c,w*.21,top*.27,d*.15,w*.32,top*.54,d*.36)
    elseif b.roof==3 then part("metal",c,0,8,0,w*.55,16,d*.48);c+=U*16
    elseif b.roof==4 then part(m,c,0,top*.35,0,w*.74,top*.7,d*.74);part("metal",c,0,top*.84,0,w*.38,top*.28,d*.38);c+=U*top*.98
    else part("metal",c,0,4,0,w*.4,8,d*.4) end
    if b.roof==1 or b.roof==4 then part(b.neon,c,0,1,d*.32,w*.65,1.2,1.4) end
    if b.roof>=3 then
        local mast=b.antenna
        part("metal",c,0,mast*.5,0,1.8,mast,1.8)
        part("beacon",c,0,mast+1,0,2.6,2.6,2.6)
        for _,level in { .45,.72 } do part("metal",c,0,mast*level,0,math.min(w*.6,16),1.2,1.2) end
        if b.roof==3 then for _,side in {-1,1} do part("metal",c,side*w*.22,mast*.32,0,1.2,mast*.64,1.2) end end
    end
end
local function architecture(b)
    local c,w,d,h,m=b.c,b.w,b.d,b.h,b.mat
    part(m,c,0,h*.07,0,w*.98,h*.14,d*.98)
    if b.kind==0 then part(m,c,0,h*.54,0,w*.78,h*.92,d*.82)
    elseif b.kind==1 then for _,side in {-1,1} do part(m,c,side*w*.255,h*.55,0,w*.36,h*.9,d*.72) end
    elseif b.kind==2 then for tier=0,2 do local s=1-tier*.23;part(m,c,0,h*(.2+tier*.3),0,w*s,h*.4,d*s);part(b.neon,c,0,h*(.4+tier*.3),0,w*s-.5,1.8,d*s-.5) end
    elseif b.kind==3 then put("octagon",m,c+U*h*.48,V(w*.92,h*.96,d*.92),Z,true);put("octagon",m,c+U*h*.98,V(w*.55,h*.12,d*.55),Z,true)
    elseif b.kind==4 then for tier=0,2 do part(m,c,w*(tier%2==0 and .12 or -.12),h*(.22+tier*.28),0,w*.72,h*.42,d*(.82-tier*.04)) end
    elseif b.kind==5 then part(m,c,0,h*.23,0,w,h*.46,d);part(m,c,0,h*.65,0,w*.42,h*.7,d*.55);part(b.neon,c,w*.22,h*.64,0,2,h*.65,3)
    elseif b.kind==6 then part(m,c,0,h*.32,0,w,h*.64,d);part(m,c,w*.21,h*.82,-d*.12,w*.48,h*.36,d*.65)
    else part(m,c,-w*.29,h*.5,0,w*.42,h,d);part(m,c,w*.205,h*.32,d*.29,w*.57,h*.64,d*.42) end
    roof(b)
    if b.landmark then
        for _,side in {-1,1} do for _,z in {-1,1} do part("metal",c,side*w*.39,h*.54,z*d*.41,1.2,h*.92,1.2) end end
        for j=1,6 do part("metal",c,0,h*j/7,0,w*.79,2.5,d*.83) end
    end
end
local function facade(b)
    local c,s=b.c,V(b.w,b.h,b.d)
    if b.kind==0 then c+=U*b.h*.54;s*=V(.78,.92,.82)
    elseif b.kind==1 then c+=V(-b.w*.255,b.h*.55,0);s*=V(.36,.9,.72)
    elseif b.kind==2 then c+=U*b.h*.8;s*=V(.54,.4,.54)
    elseif b.kind==4 then c+=V(b.w*.12,b.h*.78,0);s*=V(.72,.42,.74)
    elseif b.kind==5 then c+=U*b.h*.65;s*=V(.42,.7,.55)
    elseif b.kind==6 then c+=V(b.w*.21,b.h*.82,-b.d*.12);s*=V(.48,.36,.65)
    elseif b.kind==7 then c+=V(-b.w*.29,b.h*.5,0);s*=V(.42,1,1)
    else return nil end
    return c,s
end
local function city(ctx)
    seed(ctx.seed+170)
    local buildings={}
    local function add(c,w,d,h,kind,landmark)
        if not free(c+U*(h+180)*.5,V(w,h+180,d)*.5) then return false end
        occupy(c+U*(h+180)*.5,V(w,h+180,d)*.5+V(1,0,1))
        local b={c=c,w=w,d=d,h=h,kind=kind,roof=integer(0,5),top=R(22,55),antenna=R(35,80),mat="facade"..integer(0,31),neon="neon"..integer(0,5),landmark=landmark}
        table.insert(buildings,b);architecture(b);return true
    end
    for i,u in {.018,.04,.06,.078,.10,.12} do
        local n=sample(u);local direction=vector.normalize(V(n.r.x,0,n.r.z))*(i%2==1 and 1 or -1)
        for _,offset in {100,140,180} do
            local c=n.p+direction*(n.w+offset);c=V(math.round(c.x/120)*120,-180,math.round(c.z/120)*120)
            if add(c,88+((i-1)%3)*3,82+((i-1)%2)*10,math.max(450+(i-1)*65,n.p.y+440),0,true) then break end
        end
    end
    for x=-18,18 do for z=-18,18 do
        if random()<.035 then continue end
        local center=V(x*120,-180,z*120);local pattern=random();local plots={}
        if pattern<.16 then plots={{0,0,94,94}}
        elseif pattern<.5 then for _,side in {-1,1} do table.insert(plots,(x+z)%2==0 and {side*25,0,44,94} or {0,side*25,94,44}) end
        else for _,side in {-1,1} do for _,en in {-1,1} do table.insert(plots,{side*25,en*25,44,44}) end end end
        local before=#buildings
        for _,plot in plots do
            local kind=integer(0,7);local w,d=plot[3],plot[4];local h=#plots>1 and R(150,530) or R(280,740)
            if kind==6 then h*=.7 elseif kind==3 then w=math.min(w,R(26,36));d=math.min(d,w) end
            if random()<.08 then h=R(650,920) end
            local c=center+V(plot[1],0,plot[2]);local oh=h
            for i=0,14 do local p=sample(i*.01).p;if vector.magnitude(V(p.x-c.x,0,p.z-c.z))<360 then h=math.max(h,p.y+180+oh*.55) end end
            add(c,w,d,h,kind,false)
        end
        if #buildings>before and #plots>1 then local h=R(12,24);if clear(center+U*h*.5,V(48,h*.5,48)) then box("facade12",center+U*h*.5,V(96,h,96),true) end end
    end end
    -- Select screens near the racing route first; generation order is a city grid.
    for _,b in buildings do local n=nearest(b.c);b.distance=vector.magnitude(V(n.p.x-b.c.x,0,n.p.z-b.c.z)) end
    table.sort(buildings,function(a,b) if a.landmark~=b.landmark then return a.landmark end;return a.distance<b.distance end)
    local signs=0
    -- Facade-mounted source artwork on the two road-facing sides, not random walls.
    for index,b in buildings do
        local c,s=facade(b)
        if c==nil then continue end
        local n=nearest(c);local distance=vector.magnitude(V(n.p.x-c.x,0,n.p.z-c.z))
        if distance>850 then continue end
        if index%4~=3 or b.landmark then
            for _,side in {-1,1} do
                for tier=0,2 do
                    local y=c.y+s.y*(-.3+tier*.3)
                    for _,zs in {-1,1} do box(b.neon,V(c.x+side*(s.x*.5-1.2),y,c.z+zs*(s.z*.5+.18)),V(1.5,s.y*.23,.48)) end
                end
                box(b.neon,c+V(0,s.y*.5-.9,side*(s.z*.5+.18)),V(s.x-.8,1.5,.48))
            end
        end
        if distance>750 or signs>=640 then continue end
        for face=0,1 do
            local normal=face==0 and V(n.p.x>=c.x and 1 or -1,0,0) or V(0,0,n.p.z>=c.z and 1 or -1)
            local width=math.min((face==0 and s.z or s.x)*.84,72)
            local height=math.min(width*2,s.y*.72,s.y-12.1,160);width=height*.5
            local pos=c+normal*((face==0 and s.x or s.z)*.5+.55)
            pos=V(pos.x,math.clamp(n.p.y+65,c.y-s.y*.5+height*.5+6,c.y+s.y*.5-height*.5-6),pos.z)
            local yaw=math.atan2(normal.x,normal.z);local rot=V(0,yaw,0)
            if width>10 then
                put("box","metal",pos,V(width+1,height+1,1),rot)
                put("poster"..((index*7+face*5+ctx.seed)%6),"poster",pos+normal*.55,V(width,height,1),rot)
                signs+=1
                if signs%8==0 or b.landmark then lamp(pos+normal*5,V(.55,.72,1),10000,150) end
            end
            if b.landmark and face==0 then
                local p=pos+U*(height*.5+55)
                put("poster"..(6+index%4),"poster",p,V(width,math.min(180,width*2.5),1),rot)
            end
        end
    end
    -- Separate street lanes and stacked air corridors from the source layout.
    seed(ctx.seed+9407)
    for row=-17,17 do for level=0,3 do for _,dir in {-1,1} do
        local y=level==0 and -178 or (-35+(level-1)*145)
        local z=row*120+60+dir*8
        local start=nil;local length=0
        local function flush()
            if not start or length<240 then return end
            local spacing=level==0 and R(24,38) or R(60,85);local speed=level==0 and R(15,25) or R(42,78)
            for k=0,math.floor(length/spacing)-1 do
                moving("traffic"..integer(0,3),"traffic"..integer(0,5),start,V(1,1,1),V(0,dir*math.pi*.5,0),"traffic",k*spacing,speed*dir,length)
            end
        end
        for block=-9,9 do
            local p=V(block*240,y,z)
            if block<9 and free(p+V(120,0,0),V(129,5,5)) then if not start then start=p end;length+=240
            else flush();start=nil;length=0 end
        end
    end end end
end
local function forest(ctx)
    seed(ctx.seed+80441)
    for x=-20,20 do for z=-20,20 do
        if random()<.12 then continue end
        local age=random();local h=age>.32 and R(340,650) or (age>.1 and R(190,340) or R(80,190))
        local variant=random()<.7 and 0 or 1
        local p=V(x*125+R(-62,62),0,z*125+R(-62,62));p=V(p.x,ground(p.x,p.z),p.z)
        if p.y<ctx.water+4 then continue end
        local rw=h*.022
        for _,o in {V(rw,0,0),V(-rw,0,0),V(0,0,rw),V(0,0,-rw)} do p=V(p.x,math.min(p.y,ground(p.x+o.x,p.z+o.z)),p.z) end
        if p.y<ctx.water+2 then continue end
        local rotation=V(0,R(0,TAU),0);local size=V(R(.92,1.08),1,R(.92,1.08))*h
        if not tree_clear(variant,p,size,rotation) then continue end
        put("tree"..variant,"tree",p-U*.5,size,rotation,true)
        for plant=1,integer(3,8) do
            local angle=plant*TAU/5+random();local spot=p+V(math.cos(angle),0,math.sin(angle))*h*.065
            spot=V(spot.x,ground(spot.x,spot.z)-.15,spot.z);local s=R(2,18)
            if spot.y>ctx.water+1.5 and clear(spot+U*s*.7,V(s*1.4,s*.7,s*1.4)) then put("fern","fern",spot,V(R(.65,1.3),R(.55,1.4),R(.65,1.3))*s,V(0,angle,0)) end
        end
    end end
    -- Understory follows the route and shoreline rather than a uniform grid.
    -- Shared rock/fern templates keep this inexpensive; the ribbon stays clear.
    seed(ctx.seed+29173)
    for i=0,279 do
        local n=sample(i/280)
        local side=i%2==0 and -1 or 1
        local p=n.p+n.r*side*(n.w+R(35,155))
        p=V(p.x,ground(p.x,p.z)-.3,p.z)
        if p.y<ctx.water-2 then continue end
        local size=R(5,18)
        local scale=V(size*R(1.2,1.8),size*.65,size)
        local rotation=V(0,R(0,TAU),0)
        if mesh_clear("rock2",p,scale,rotation) then put("rock2","moss",p,scale,rotation,true) end
        for k=1,4 do
            local a=R(0,TAU);local q=p+V(math.cos(a),0,math.sin(a))*R(size*.6,size*1.8)
            q=V(q.x,ground(q.x,q.z)-.2,q.z)
            local h=R(4,10)
            if q.y>ctx.water+1 and clear(q+U*h*.5,V(h,h*.5,h)) then put("fern","fern",q,V(h,h*.7,h),V(0,a,0)) end
        end
    end
end
local function cell(ctx)
    local lowest,highest,reach=math.huge,-math.huge,0
    for i=0,ctx.nodes-1 do local p=node(i).p;lowest=math.min(lowest,p.y);highest=math.max(highest,p.y);reach=math.max(reach,vector.magnitude(V(p.x,0,p.z))) end
    put("membrane","membrane",V(0,(highest+lowest)*.5,0),V((reach+1200)*2,math.max(2800,highest-lowest+2000),(reach+1200)*2))
    seed(ctx.seed+92273)
    local colors={"tissue0","tissue1","tissue2","tissue3"}
    for i=0,ctx.nodes-1,9 do
        local n=node(i);local side=random()>.5 and 1 or -1;local size=R(110,230)
        local kind=i%45==0 and "dna" or (random()<.5 and "protein" or "mitochondrion")
        if kind=="dna" then size=R(330,590) end
        local p=n.p+n.r*side*R(125,370)+U*R(-100,110)
        local rot=V(R(-.45,.45),R(0,TAU),R(-.35,.35))
        local s=V(size,size,size)*(kind=="mitochondrion" and V(.78,.55,1.35) or V(1,1,1))
        if not mesh_clear(kind,p,s,rot) then continue end
        local m=colors[integer(1,4)]
        put(kind,m,p,s,rot,kind=="mitochondrion")
        if kind=="mitochondrion" then put("cristae","fold",p,s,rot) end
    end
    local p=V(0,80,0);local s=V(720,630,720)
    if clear(p,s*.5+V(15,15,15)) then
        put("sphere","nucleus",p,s,Z,true)
        for i=0,33 do local y=1-2*(i+.5)/34;local a=i*2.39996;local n=V(math.sqrt(1-y*y)*math.cos(a),y,math.sqrt(1-y*y)*math.sin(a));pore(p+n*s*.499,n) end
    end
    for i=0,ctx.nodes-1,24 do
        local n=node(i);local size=R(32,52);local c=n.p+n.r*(i%48==0 and 245 or -245)-U*25
        if clear(c+U*(size*1.3),V(100,size*1.3+16,100)) then
            walker(c,size,R(0,TAU))
            local a=c-V(0,8,66);local b=c+V(0,-8,66)
            line("microtubule",a,b,9)
            for bead=0,17 do put("sphere","tissue2",a+(b-a)*bead/17,V(14,11,7)) end
        end
    end
    for i=0,41 do
        local a=i*TAU/42;local c=V(math.cos(a)*R(1500,1950),R(60,600),math.sin(a)*R(1500,1950));local size=R(240,410);local rot=V(R(0,TAU),R(0,TAU),0)
        if mesh_clear("reticulum",c,V(size,size,size),rot) then put("reticulum",i%2==0 and "tissue0" or "tissue2",c,V(size,size,size),rot) end
    end
    for i=1,180 do local c=V(R(-1800,1800),R(-180,650),R(-1800,1800));local size=R(10,34);if clear(c,V(size,size,size)*.5+V(8,8,8)) then put("sphere",colors[i%4+1],c,V(size,size,size),Z,true) end end
end
local function dry(ctx,ocean)
    seed(ctx.seed+51713)
    for x=-24,24 do for z=-24,24 do
        local p=V(x*130+R(-60,60),0,z*130+R(-60,60));if vector.magnitude(p)>3300 then continue end
        p=V(p.x,ground(p.x,p.z),p.z);local roll=random()
        if roll<(ocean and .2 or .5) then
            local variant=roll<.16 and integer(0,1) or integer(2,3)
            local h=variant<2 and R(35,120) or R(6,26);local size=V(h*R(.75,1.25),h,h*R(.8,1.2))
            local rot=V(0,R(0,TAU),0);if mesh_clear("rock"..variant,p,size,rot) then put("rock"..variant,ocean and "reef" or "sandstone",p,size,rot,true) end
        elseif not ocean and roll<.78 then
            for k=1,integer(1,4) do local spot=p+V(R(-45,45),0,R(-45,45));spot=V(spot.x,ground(spot.x,spot.z)-.3,spot.z);local h=R(9,17);if clear(spot+U*h*.5,V(h*.25,h*.5,h*.25)) then put("cactus","cactus",spot,V(h,h,h),V(0,R(0,TAU),0),true) end end
        elseif ocean and roll<.8 then
            local kind=roll<.5 and "kelp" or (roll<.6 and "lantern" or (roll<.7 and "fan" or "sponge"))
            local count=kind=="kelp" and integer(4,8) or integer(1,3)
            for k=1,count do
                local spot=p+V(R(-55,55),0,R(-55,55));spot=V(spot.x,ground(spot.x,spot.z)-.5,spot.z)
                local h=kind=="kelp" and R(40,115) or (kind=="lantern" and R(40,85) or R(18,42));local size=V(h,h,h);local rot=V(0,R(0,TAU),0)
                if mesh_clear(kind,spot,size,rot) then put(kind,kind,spot,size,rot) end
            end
        end
    end end
    -- Source natural arches: a continuous rock tube over a clear opening.
    for i=0,7 do
        local n=sample((i+.35)/8);local p=n.p+n.r*(i%2==0 and 200 or -200);p=V(p.x,ground(p.x,p.z)-3,p.z)
        local size=V(R(150,240),R(110,180),R(45,65))
        local rot=V(0,R(0,TAU),0);if mesh_clear("arch",p,size,rot) then put("arch",ocean and "reef" or "sandstone",p,size,rot,true) end
    end
    if ocean then
        seed(ctx.seed+81919)
        -- Giant animals are landmarks, not obstacles across the racing line.
        -- Reserve the complete tentacle envelope, including their animation.
        for _,fraction in {.035,.27,.53,.78} do
            local best=nil;local score=math.huge
            for attempt=0,79 do
                local n=sample(fraction+(attempt//4)*.006)
                local side=attempt%2==0 and -1 or 1
                local size=R(22,30)
                local p=n.p+n.r*side*(n.w+size*6.4+45+(attempt%4)*12)
                p=V(p.x,ground(p.x,p.z)+size*1.5,p.z)
                -- Prefer low shelves visible from the road, not hidden behind
                -- the tall canyon walls. Keep a fallback for steep seeds.
                local cost=math.abs(p.y+size*1.2-n.p.y)+attempt*.3
                if cost<score and clear(p+U*size*2.5,V(size*6.2,size*3.5,size*6.2)) then
                    local facing=n.p-p
                    best={p=p,size=size,yaw=math.atan2(-facing.x,-facing.z)};score=cost
                end
            end
            if best then octopus(best.p,best.size,best.yaw,R(0,TAU)) end
        end
        for vent=0,15 do
            local n=sample((vent+.4)/16)
            local p=n.p+n.r*(vent%2==0 and 100 or -100)
            p=V(p.x,ground(p.x,p.z)+2,p.z)
            for k=0,19 do
                local r=R(.3,1.1)
                moving("bubble","bubble",p+V(R(-5,5),0,R(-5,5)),V(r,r,r),Z,"bubble",k*6,R(4,7),120)
            end
        end
        for school=0,11 do
            local n=sample((school+R(.1,.9))/12);local p=n.p+n.r*(school%2==0 and 130 or -130)+U*R(10,45)
            if clear(p,V(65,45,65)) then for k=0,59 do moving("fish","fish"..(school%5),p+V(R(-35,35),R(-14,14),R(-35,35)),V(5,5,5),Z,"swim",R(0,TAU),R(.1,.18),R(40,100)) end end
        end
        for swarm=0,6 do local n=sample((swarm+.5)/7);local p=n.p+n.r*230+U*60;for k=1,16 do moving("jelly","jelly",p+V(R(-70,70),R(-40,70),R(-70,70)),V(20,20,20),Z,"drift",R(0,TAU),.2,12) end end
        for i=0,5 do local n=sample((i+.3)/6);moving("manta","manta",n.p+n.r*190+U*70,V(45,45,45),Z,"swim",R(0,TAU),.08,90) end
    end
end
return {build=function(ctx)
    if ctx.biome=="city" then city(ctx)
    elseif ctx.biome=="forest" then forest(ctx)
    elseif ctx.biome=="cell" then cell(ctx)
    else dry(ctx,ctx.biome=="ocean") end
end}

