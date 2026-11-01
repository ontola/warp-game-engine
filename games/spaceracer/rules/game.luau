-- Game decisions live here. Rust owns the 120 Hz movement integrators and geometry.
-- Inputs are snapshots; RNG samples and simulation time come from the native sim.
local clamp = math.clamp
local function track(c)
    local family = (c.seed - 31) % 6
    local p = {corner_scale=1, width_scale=1, hard_flight=true, stadium=false,
        exposed={{.02,.05},{.285,.335},{.64,.68},{.755,.80}},
        boosts={{.115,.142},{.63,.65}}, loops={.16}, tunnels={{.33,.38}}, corners={},
        sections={{"open",.275,.315},{"halfpipe",.405,.49},{"split",.535,.615},{"tube",.805,.92}},
        jumps={{.070,.152}}}
    if family==1 then
        p.loops={}; p.corner_scale=.6;p.hard_flight=false
        p.tunnels={{.17,.30},{.34,.51},{.55,.72},{.81,.92}}
        p.sections={{"split",.735,.795},{"narrows",.365,.425}}
    elseif family==2 then
        p.loops={};p.corner_scale=.25;p.hard_flight=false
        p.corners={{.235,125,.20},{.495,-130,.20},{.75,120,.19}}
        p.tunnels={{.57,.615}}
        p.sections={{"chicane",.19,.28},{"chicane",.45,.54},{"chicane",.705,.795},{"narrows",.83,.90}}
    elseif family==3 then
        p.loops={.17,.45,.73};p.corner_scale=.45
        p.jumps={{.070,.152},{.325,.410},{.60,.682}};p.tunnels={}
        p.sections={{"open",.285,.31},{"halfpipe",.845,.912}}
    elseif family==4 then
        p.stadium=true;p.loops={};p.hard_flight=false
        p.boosts={{.165,.195},{.235,.27},{.365,.40},{.515,.55},{.58,.615},{.635,.67},{.745,.775},{.85,.885},{.925,.97}}
        p.tunnels={{.62,.71}};p.sections={{"open",.18,.26},{"narrows",.30,.35},{"split",.435,.50},{"narrows",.78,.83}}
    elseif family==5 then
        p.loops={};p.corner_scale=.3;p.width_scale=1.12;p.tunnels={}
        p.sections={{"halfpipe",.17,.27},{"tube",.29,.42},{"halfpipe",.44,.58},{"split",.61,.67},{"tube",.69,.82},{"halfpipe",.84,.91}}
    end
    -- Sampling order stays native/deterministic; all design ranges are editable here.
    p.dimensions={radius={900,1100},lobes={2,4},amplitude={65,155},stretch={1,1.3},
        hills={2,3},climb={105,170},corner_a={.15,.16},corner_b={.18,.195},
        corner_shift={-.055,.055},loop_radius={185,225},road_width={23,6,9}}
    return p
end
local function personality(c)
    local k=(c.seed+c.slot*5)%6+1
    return {pace=({1.02,.98,1,.96,1.03,.99})[k],caution=({.94,1.07,1,1.10,.96,1.02})[k],
        line=({-.18,.20,0,-.08,.12,-.24})[k],strafe=({.37,.23,.31,.26,.42,.34})[k],
        reserve=({30,55,38,50,28,42})[k],boost_chance=({.82,.45,.65,.52,.90,.70})[k],
        boost_cooldown=({2.2,4.4,3.1,4,2.6,3.6})[k],phase=.11+k*.23}
end
local function plan(c)
    return {line=clamp(c.style.line+(c.r1-.5)*.18,-.36,.36),
        pace=c.style.pace+(c.r2-.5)*.06,next=c.clock+2.2+c.r3*4,
        seek_item=c.r4<.58}
end
local function boost_plan(c)
    -- An independent decision clock, including on lap one: crossing the lap
    -- boundary or reaching a straight no longer triggers everyone together.
    return {attempt=c.r1<c.style.boost_chance,next=c.clock+.35+c.r2*1.35}
end
local function ai(c)
    local p,n=c.p,c.node
    if p.airborne then
        return {drive={throttle=.85,brake=clamp((p.speed-235)/100,0,.25),trim=c.air_trim,steer=c.air_steer,strafe=c.air_roll},reset=c.air_reset,fire=c.air_fire}
    end
    local safe=c.boost_speed
    local peak=math.abs(n.curve)
    for _,v in ipairs(c.preview) do
        local bend=math.abs(v.curve);peak=math.max(peak,bend)
        local speed=clamp((c.hard and 2.7 or 1.65)/math.max(.001,bend),c.hard and 160 or 125,c.boost_speed)
        if v.hairpin then speed=clamp(1.5/math.max(.001,bend),75,c.boost_speed) end
        safe=math.min(safe,math.sqrt((speed*c.pace/c.style.caution)^2+300*v.ahead/c.style.caution))
    end
    local brake=clamp((p.speed-safe)/35,0,1)
    if not c.hard and math.abs(n.curve)>.006 and p.speed>145 then brake=math.max(brake,.48) end
    local target=c.line*n.width
    if type(c.pickup)=="table" and peak<.004 and not c.has_item then
        local weight=clamp(1-c.pickup.ahead/350,.2,1)
        target=target+(c.pickup.x-target)*weight
    end
    if math.abs(c.next_curve)>.003 then target-=math.sign(c.next_curve)*n.width*.18 end
    if math.abs(n.curve)>.004 then target+=math.sign(n.curve)*n.width*.22 end
    if n.zone=="repair" and p.energy<70 then target=-n.width*.29 end
    if n.split_gap>.01 then
        local side=n.route~=0 and n.route or (p.slot%2==0 and 1 or -1)
        target=side*(n.split_gap+(n.width-n.split_gap)*.5)
    elseif c.next_route~=0 then target=c.next_route*15 end
    local lateral=clamp((target-p.x)*2.4,-45,45)
    local demand=lateral+n.curve*p.speed^2*(.25+.75*p.slide)/(14-12.2*p.slide)
    local strafe=clamp(demand*c.style.strafe/46,-.75,.75)
    local heading=math.asin(clamp((demand-strafe*46)/math.max(p.speed,60),-.75,.75))
    local steer=clamp((n.curve*p.speed+heading*(3.5-2.7*p.slide)+(heading-p.heading)*3.8+(lateral-p.slip)*.012)/(1.65+2.15*p.slide),-1,1)
    if c.at_jump then brake=0 end
    return {drive={throttle=1,steer=steer,strafe=strafe,brake=brake,boost=p.lap>1 and p.energy>math.max(p.boost_cost+4,c.style.reserve) and peak<.003/c.style.caution and not c.near_jump and brake<.05 and c.boost_ready and p.boost<=0},reset=c.missed,fire=c.has_item and c.fire_tick}
end
local function pickup(c)
    local pool=c.rank==1 and {"sentry","railgun","glidebomb"} or {"missile","warp","warp","sentry","emp","railgun","glidebomb"}
    return pool[1+math.min(#pool-1,math.floor(c.roll*#pool))]
end
local function balance(_)
    return {countdown=3,respawn_seconds=2,race_timeout=240,refill_rate=1,pickup_seconds=2,
        pickup_spacing=1300,warp_seconds=2.8,warp_speed=630,sentry_seconds=8,sentry_interval=.05,
        sentry_damage=.5,rail_damage=50,emp_seconds=3,emp_radius=100,missile_damage=60,missile_radius=55,
        bomb_damage=145,bomb_radius=120,projectile_timeout=12}
end
-- Random is a selection, resolved only when creating a new race. Native code
-- supplies entropy once; simulation and track generation remain deterministic.
local function menu_defaults(c)
    return {seed=c.entropy%99999+1,biome=math.floor(c.entropy/99999)%5,
        difficulty=math.floor(c.entropy/499995)%3}
end
local rows={"RESUME RACE","START NEW RACE","PLAYERS","WORLD","DIFFICULTY","TRACK SEED"}
local function menu(c)
    local first=c.can_resume and 0 or 1
    local row=first+(math.max(first,c.row)-first+c.vertical)%(#rows-first)
    local action="none"
    if c.start then action=c.can_resume and row~=1 and "play" or "new"
    elseif c.accept and row<2 then action=row==0 and "play" or "new" end
    local players,biome,difficulty,seed=c.players,c.biome,c.difficulty,c.seed
    if c.horizontal~=0 then
        if row==2 then players=(players-1+c.horizontal)%4+1
        elseif row==3 then biome=(biome+1+c.horizontal)%6-1
        elseif row==4 then difficulty=(difficulty+1+c.horizontal)%4-1
        elseif row==5 then seed=(seed+c.horizontal)%100000 end
    end
    return {row=row,action=action,label=rows[row+1],players=players,biome=biome,difficulty=difficulty,seed=seed,
        rebuild=c.horizontal~=0 and row>=2}
end
-- Weapon activation policy runs only on a trigger edge. Rust performs the
-- ray/sweep queries and integrates the resulting bodies at 120 Hz.
local function activate(c)
    if c.crashed or c.finished or c.active or c.emp>0 then return {effect="none"} end
    local b=balance(nil)
    if c.item=="GlideBomb" and not c.airborne then return {effect="none"} end
    if c.item=="Emp" then return {effect="emp",duration=b.emp_seconds,range=b.emp_radius} end
    if c.item=="Warp" and c.airborne then return {effect="none"} end
    if c.item=="Warp" then return {effect="active",duration=b.warp_seconds} end
    if c.item=="Sentry" then return {effect="active",duration=b.sentry_seconds} end
    if c.item=="Railgun" then return {effect="ray",cone=.975,range=700,damage=b.rail_damage} end
    return {effect="projectile",cone=c.item=="GlideBomb" and .7 or 1,range=1000,speed=c.speed+40}
end
local function environment(c)
    local e={ground=-150,hills=0,spacing=140,skip=.18,height={110,510},sun=24,sky=3,ambient=34,exposure=5.5}
    if c.biome=="forest" then
        e={ground=-70,hills=28,spacing=62,skip=.08,height={90,170},sun=78000,sky=11000,exposure=13.3}
    elseif c.biome=="cell" then
        e={ground=-110,hills=35,spacing=140,skip=.2,height={45,160},sun=100,sky=90,ambient=180,exposure=8}
    elseif c.biome=="desert" then
        e={ground=-90,hills=42,spacing=180,skip=.35,height={65,170},sun=85000,sky=15000,exposure=14}
    elseif c.biome=="ocean" then
        e={ground=-145,hills=70,spacing=180,skip=.5,height={70,165},sun=2200,sky=1800,exposure=10}
    end
    return e
end
-- Source victory choreography and cruise guidance policy. Geometry/sweeps stay native.
local function victory(c)
    local curve=c.curve
    local t=(c.age%4.8)/4.8
    local shot=math.floor(c.age/4.8)%4
    local offsets={{-13+4*t,6,-21},{11,4.5,21-5*t},{20-4*t,9,-5},{-7,25-6*t,-23}}
    -- Keep racing after the classification is fixed. Look ahead for braking,
    -- and stagger boost decisions per driver rather than a synchronized parade.
    local peak=math.abs(curve)
    local safe=c.stack_speed
    for _,v in ipairs(c.preview) do
        local bend=math.abs(v.curve);peak=math.max(peak,bend)
        local corner=clamp((v.hairpin and 1.45 or 2.15)/math.max(.001,bend),105,c.stack_speed)
        safe=math.min(safe,math.sqrt(corner^2+2*240*v.ahead))
    end
    local boost=c.ready and c.boost<=0 and peak<.0035 and not c.near_jump and safe>c.top_speed*1.12
    local powered=c.boost>0 or boost
    local cruise=c.top_speed*(.96+.04*math.sin(c.age*.23+c.slot*1.7))
    local target=powered and (c.on_pad and c.stack_speed or c.boost_speed) or (c.on_pad and c.boost_speed or cruise)
    target=math.min(target,safe)
    local brake=clamp((c.speed-safe)/65,0,1)
    local steer=clamp(curve*c.speed*.6,-1,1)
    return {speed=target,acceleration=brake>.02 and 260 or c.speed>target and 38 or (powered or c.on_pad) and 230 or 150,
        throttle=brake>.15 and .25 or 1,brake=brake,boost=boost,
        boost_delay=3.5+c.slot*.37+.7*math.sin(c.age*.4+c.slot),
        steer=steer,strafe=clamp(curve*c.speed*.24,-.65,.65),
        trim=clamp(c.crest*c.speed*.25,-.3,.3),
        line=clamp(-c.next_curve*35,-.24,.24)+.035*math.sin(c.age*.6+c.slot),
        heading=clamp(curve*c.speed*(.1+brake*.18),-.22,.22),bank=-steer*.19,
        hop_height=24,offset=offsets[shot+1],fov=({58,52,56,62})[shot+1]}
end
local function missile(c)
    local speed=math.min(500,math.max(80,c.launch_speed)+math.max(0,c.age-.18)*620)
    return {speed=speed,launch_blend=clamp((c.age-.18)/.87,0,1),
        lock=c.target and c.age>.38 and c.range/speed<.75,
        terminal=c.target and c.age>.38 and c.range<180 and math.abs(c.along)<100,
        fuse=3.4,laser_seconds=.75}
end
-- Non-critical weapon policy remains editable without rebuilding the native game.
local function combat(_)
    return {bomb_speed=360,bomb_accel=200,bomb_burn=1.25,bomb_turn=1.15,bomb_gravity=14,
        bomb_drag=.012,bomb_eject=55,bomb_range=1100,bomb_cone=.573576,bomb_fuse=18,
        sentry_ahead=220,sentry_range=190,sentry_cone=.15,sentry_spinup=.25}
end
local function warp(c)
    local function smooth(a,b,x) local t=clamp((x-a)/(b-a),0,1);return t*t*(3-2*t) end
    local exit=clamp(1.65/math.max(.001,math.abs(c.curve)),135,315)
    return {speed=exit+(630-exit)*smooth(0,.4,c.age)*smooth(0,.6,c.remaining),acceleration=850,lateral=90,
        strength=smooth(0,.18,c.age)*smooth(0,.4,c.remaining)}
end
return {combat=combat,warp=warp,victory=victory,missile=missile,track=track,ai=ai,plan=plan,personality=personality,boost_plan=boost_plan,pickup=pickup,balance=balance,menu=menu,menu_defaults=menu_defaults,environment=environment,activate=activate}
