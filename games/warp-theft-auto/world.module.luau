-- Port Meridian. Streets, sidewalk loops and collision all use metres in the
-- same coordinate system. Buildings and civic landmarks are generated once.
local room = require("room")
local city = { STEP = 56, BOUND = 178 }
local v = vector.create
local STEP = city.STEP
local BOUND = city.BOUND

local ASPHALT, OFFICE, HOTEL, WAREHOUSE, LAWN, WATER, PAD, PLAZA, CONTAINER = 0, 1, 2, 3, 4, 5, 6, 7, 8
local PARK_WIDTH, DIAGONAL_PATH, TREE_GRATE = 37, 1.15, 1.8
local GROUND_HEIGHT = 4.2
local OFFICE_COLORS = { "#738f96", "#63868b", "#83999d", "#797f8c" }
local HOTEL_COLORS = { "#d79d81", "#dfc79a", "#bbc3a9", "#c89783", "#dfb5a3" }
local DOCK_COLORS = { "#97a7a3", "#bf8a71", "#bcb498" }
local CONTAINERS = { "#ba5f46", "#477479", "#b49152", "#6c8392" }
local METAL, LIMESTONE, TARMAC = "#42626c", "#e1d4ae", "#4b6260"

local function box(x,y,z,w,h,d,color,surface,solid,extra)
    local opts = extra or {}
    opts.pos, opts.size = v(x,y,z), v(w,h,d)
    opts.color, opts.surface = color, surface
    opts.solid, opts.tag = solid == true, if solid then "city" else "scenery"
    return world.spawn("cube", opts)
end

-- Index by street block. Clamp includes objects on the outer promenade.
local function index(x)
    return math.clamp(math.floor(x / STEP) + 4, 1, 6)
end

function city.block_index(x,z)
    return (index(x)-1)*6+index(z)
end

local function obstacle(s,x,y,z,w,h,d,color,surface,extra)
    local id = box(x,y,z,w,h,d,color,surface,true,extra)
    local r = { x0=x-w/2, x1=x+w/2, z0=z-d/2, z1=z+d/2, top=y+h/2 }
    for ix=index(r.x0),index(r.x1) do
        for iz=index(r.z0),index(r.z1) do
            table.insert(s.blocks[(ix-1)*6+iz],r)
        end
    end
    return id
end

function city.roof(s,x,z,radius)
    local r = math.max(radius or 0, 0)
    local top = 0
    for ix=index(x-r),index(x+r) do
        for iz=index(z-r),index(z+r) do
            for _,b in s.blocks[(ix-1)*6+iz] do
                local dx = math.max(b.x0-x, 0, x-b.x1)
                local dz = math.max(b.z0-z, 0, z-b.z1)
                if dx*dx + dz*dz <= r*r then top = math.max(top, b.top) end
            end
        end
    end
    return top
end

function city.blocked(s,x,z,radius,bottomY)
    local r = radius or 0
    return math.abs(x)+r > BOUND or math.abs(z)+r > BOUND
        or city.roof(s,x,z,r) > (bottomY or 0) + 0.05
end

function city.spawn(id)
    return v(6.1, 0, 11 + (id-1)%4*10)
end

function city.vehicle_spawns(seed)
    local spawns = {}
    for i=0,3 do table.insert(spawns,{ kind="car", pos=v(3.7,0,11+i*10), yaw=0 }) end
    table.insert(spawns,{kind="car", pos=v(-60.85,0,-89), yaw=math.pi})
    table.insert(spawns,{kind="car", pos=v(116.85,0,77), yaw=0})
    table.insert(spawns,{kind="car", pos=v(-81,0,116.85), yaw=-math.pi/2})
    table.insert(spawns,{ kind="helicopter", pos=v(154,0,26), yaw=0 })
    table.insert(spawns,{ kind="helicopter", pos=v(-154,0,-86), yaw=math.pi/2 })
    -- Park in marked curb lanes, well clear of intersections and both heliports.
    -- Keep the starter cars and aircraft at their stable fleet indices.
    for row=-2,2 do
        for col=-3,2 do
            if (row-col)%5==0 then continue end
            local variant=((seed or 0)+row*127+col*179)%13
            local along=(col+.5)*STEP+variant-6
            local side=if (row+col)%3==0 then -1 else 1
            local curb=row*STEP+side*4.85
            local vertical=(row+col)%2==0
            local pos=if vertical then v(curb,0,along) else v(along,0,curb)
            local free=true
            for _,other in spawns do
                if vector.magnitude(other.pos-pos)<7 then free=false; break end
            end
            if free then
                local yaw=if vertical then (if side>0 then 0 else math.pi) else -side*math.pi*.5
                table.insert(spawns,{kind="car",pos=pos,yaw=yaw})
            end
        end
    end
    return spawns
end

function city.district(x,z)
    if x >= STEP*2 then return "East Docks" end
    if x < -STEP then return "Lido Gardens" end
    if z > STEP or z < -STEP then return "Old Meridian" end
    return "Civic Center"
end

-- A hash per lot keeps the skyline reproducible when another city system
-- adds a random draw. Inputs and intermediate products fit in 53 bits.
local function lotSeed(seed,ix,iz,salt)
    local n = (seed%2147483647 + (ix+8)*7385609 + (iz+8)*1934963 + salt*834927) % 2147483647
    n = (n*48271)%2147483647
    return (n*16807)%2147483647
end
local function noise(seed,ix,iz,salt)
    return lotSeed(seed,ix,iz,salt)/2147483647
end
local function sample(colors,f)
    return colors[1+math.floor(f*#colors)]
end

-- Signs read PORT, QUAY, LIDO or TIDE in the facade shader.
local function sign(x,y,z,w,h,color,word,yaw)
    box(x,y,z,w,h,0.28,color,10+word,false,{rot=v(0,yaw or 0,0)})
end

-- Only this ground-floor box is replaced while an interior is occupied.
-- The index keeps its footprint for cars, rotorcraft and roof queries.
local function groundFloor(s,kind,x,z,w,d,color,surface,seed)
    local front = z+d/2
    local proxySurface = room.proxy_surface(kind,seed)
    local shell = obstacle(s,x,GROUND_HEIGHT/2,z,w,GROUND_HEIGHT,d,color,proxySurface)
    table.insert(s.buildings,{
        id=#s.buildings+1, kind=kind, seed=seed, shell=shell, door=v(x,0,front),
        x=x, z=z, w=w, h=GROUND_HEIGHT, d=d, color=color, surface=surface, proxySurface=proxySurface,
    })
    -- A lit transom marks the same opening before and after streaming.
    box(x,2.59,front+.20,3.10,.18,.48,METAL,23)
end

local function office(s,x,z,ix,iz,seed)
    local landmark = ix == -1 and iz == -1
    local h = if landmark then 53 else 14 + math.floor(noise(seed,ix,iz,1)*14)*2.9
    local w = 22 + 6*noise(seed,ix,iz,2)
    local d = 22 + 7*noise(seed,ix,iz,3)
    local tone = sample(OFFICE_COLORS,noise(seed,ix,iz,4))
    groundFloor(s,"office",x,z,w+6,d+4,LIMESTONE,29,lotSeed(seed,ix,iz,61))
    obstacle(s,x,h/2+GROUND_HEIGHT,z,w,h,d,tone,OFFICE)
    obstacle(s,x,h+4.55,z,w+.6,.7,d+.6,LIMESTONE)
    if landmark or noise(seed,ix,iz,5)>.38 then
        local crown = if landmark then 10 else 4.6
        obstacle(s,x,h+4.9+crown/2,z,w*.69,crown,d*.72,tone,OFFICE)
        obstacle(s,x,h+5.2+crown,z,w*.72,.6,d*.75,LIMESTONE)
        if landmark then
            obstacle(s,x,h+17,z,6,4,6,LIMESTONE)
            obstacle(s,x,h+23,z,.55,8,.55,METAL)
            box(x,h+27.1,z,.6,.35,.6,"#ff805c",nil,false,{emissive=1.1})
        end
    end
    sign(x,3.44,z+d/2+2.16,6.0,1.20,"#24616a",1)
end

local function hotel(s,x,z,ix,iz,seed)
    -- Two narrow buildings on separate lots, small courts between them.
    for side=-1,1,2 do
        local u = x+side*9
        local salt = 12+side*5
        local h = 3.1 * (2+math.floor(noise(seed,ix,iz,salt)*4))
        local d = 18+noise(seed,ix,iz,salt+1)*10
        local w = 12+noise(seed,ix,iz,salt+2)*3.5
        local tone = sample(HOTEL_COLORS,noise(seed,ix,iz,salt+3))
        groundFloor(s,"hotel",u,z,w,d,tone,29,lotSeed(seed,ix,iz,63+side))
        obstacle(s,u,(h+GROUND_HEIGHT)/2,z,w,h-GROUND_HEIGHT,d,tone,HOTEL)
        obstacle(s,u,h+.2,z,w+.5,.4,d+.55,LIMESTONE)
        box(u,2.78,z+d/2+.56,w*.82,.24,1.35,if side<0 then "#397c7c" else "#bc6149")
        obstacle(s,u,h+.95,z-2,w*.28,1.5,d*.29,"#919e99")
        if side < 0 then sign(u,3.37,z+d/2+.18,w*.44,1.18,"#a34e40",if ix < -1 then 2 else 3) end
    end
end

local function docks(s,x,z,ix,iz,seed)
    local tone = sample(DOCK_COLORS,noise(seed,ix,iz,32))
    local h = 5.5 + 3*noise(seed,ix,iz,31)
    groundFloor(s,"warehouse",x-5,z-3,22,23,tone,WAREHOUSE,lotSeed(seed,ix,iz,65))
    obstacle(s,x-5,(h+GROUND_HEIGHT)/2,z-3,22,h-GROUND_HEIGHT,23,tone,WAREHOUSE)
    obstacle(s,x-5,h+.2,z-3,23,.4,24,METAL)
    sign(x-5,h-.85,z+8.7,8.5,1.4,"#365767",0)
    for i=0,2 do
        local c = sample(CONTAINERS,noise(seed,ix,iz,33+i))
        local units = if noise(seed,ix,iz,37+i)>.53 then 2 else 1
        for n=1,units do obstacle(s,x+11,(n-.5)*2.6,z-8+i*7,8,2.58,3.15,c,CONTAINER) end
    end
end

local function helipad(x,z)
    local side = if x>0 then 1 else -1
    local berth = x+14*side
    box(berth,.012,z-2,8,.024,13,TARMAC)
    box(x,.018,z,29,.036,29,TARMAC,PAD)
    for _,edge in {-1,1} do
        box(berth+edge*3.4,.042,z-2,.16,.016,10,"#d8bf80")
        box(berth,.042,z-2+edge*5,6.9,.016,.16,"#d8bf80")
    end
    for _,i in {-1,1} do
        for _,j in {-1,1} do
            box(x+13.1*i,.18,z+13.1*j,.36,.36,.36,"#a1f0d6",nil,false,{emissive=.7})
        end
    end
end

-- All four faces use the world clock shared by local players and spectators.
local function clockHands(s,x,y,z)
    s.clockFaces={}
    for i=0,3 do
        local yaw=i*math.pi/2
        local normal=v(math.sin(yaw),0,math.cos(yaw))
        local center=v(x,y,z)+normal*1.60
        local hour=box(center.x,center.y,center.z,.072,.34,.025,"#273f45")
        local minute=box(center.x,center.y,center.z,.049,.50,.025,"#273f45")
        table.insert(s.clockFaces,{center=center, right=v(normal.z,0,-normal.x), normal=normal,
            yaw=yaw, hour=hour, minute=minute})
    end
end

function city.set_clock(s,hour)
    local minute=math.floor(hour*60)%1440
    if minute==s.clockMinute then return end
    s.clockMinute=minute
    local function place(face,hand,angle,length,offset)
        local dir=face.right*math.sin(angle)+v(0,math.cos(angle),0)
        world.set(hand,{pos=face.center+dir*(length/2)+face.normal*offset,rot=v(0,face.yaw,-angle)})
    end
    for _,face in s.clockFaces do
        place(face,face.hour,minute*math.pi/360,.34,0)
        place(face,face.minute,(minute%60)*math.pi/30,.50,.028)
    end
end
local function park(s,x,z,plaza,lawns)
    box(x,.018,z,PARK_WIDTH,.036,PARK_WIDTH,if plaza then "#d0b78f" else "#657d49",if plaza then PLAZA else LAWN)
    table.insert(s.parks,{x=x,z=z})
    if plaza then
        -- Ferry exchange: an illuminated four-faced clock on a stepped pier.
        obstacle(s,x,.27,z,5.4,.54,5.4,LIMESTONE)
        obstacle(s,x,2.75,z,1.8,4.5,1.8,"#66918c")
        obstacle(s,x,5.8,z,3.15,1.6,3.15,"#d8cdb3",9)
        clockHands(s,x,5.8,z)
        obstacle(s,x,6.76,z,3.5,.34,3.5,METAL)
        obstacle(s,x,7.38,z,1.1,.92,1.1,"#5c8685")
    else
        local center = v(x,.036,z)
        table.insert(lawns.patches,{pos=center,width=PARK_WIDTH,depth=PARK_WIDTH})
        -- surface.wgsl paints the paths where abs(localX Â± localZ) <= 1.15.
        -- Its diagonal coordinate is sqrt(2) times the perpendicular distance.
        for _,turn in {-1,1} do
            table.insert(lawns.cutouts,{pos=center,width=DIAGONAL_PATH*math.sqrt(2),
                depth=PARK_WIDTH*math.sqrt(2),yaw=turn*math.pi/4})
        end
        for _,i in {-1,1} do
            box(x+9*i,.52,z+11,3.5,.22,.8,"#976d4e")
            for _,endX in {-1,1} do box(x+9*i+endX*1.1,.23,z+11,.22,.46,.6,METAL) end
        end
    end
end

local function streetLamp(s,x,z)
    box(x,2.8,z,.18,5.6,.18,METAL)
    box(x-1,5.52,z,2.1,.17,.18,METAL)
    box(x-1.87,5.42,z,.78,.18,.54,"#ffdd9e",nil,false,{emissive=1.1})
    table.insert(s.lamps,v(x-1.87,5.23,z))
end

-- Furnish the corners where Exchange Avenue meets the tram route. The small
-- plinths and tree grates sit behind the continuous sidewalk patrol loops.
local function crossing(x,z,side)
    box(x,1.72,z,.15,3.44,.15,METAL)
    box(x,3.02,z,.60,.97,.31,"#33454a")
    for _,face in {-1,1} do
        box(x,3.15,z+.17*face,.29,.30,.05,"#ffba47",nil,false,{emissive=.84})
        box(x,2.85,z+.17*face,.27,.14,.05,"#d98c4a")
    end
    sign(x+side*.50,3.80,z,1.72,.50,"#286660",1)
end

local function avenue(s)
    -- Grates and storefronts occupy the furnishing strip, clear of traffic.
    for _,z in {-130,-92,-70,-42,-16,7} do
        local x=if z==-16 then 9.35 else -9.35
        table.insert(s.plantSites,v(x,0,z))
        if z<0 then table.insert(s.plantSites,v(-x,0,z-4)) end
    end
    crossing(-7.15,-7.15,-1)
    crossing(7.15,7.15,1)
    crossing(7.15,-49,1)
    -- Covered bus stop, lit timetable and a bench, with an open sightline
    -- down the road. Riders can walk between it and the curb on either side.
    local x,z=-10.1,-27
    for _,endZ in {-2.15,2.15} do box(x,1.23,z+endZ,.14,2.46,.14,METAL) end
    box(x+.40,2.54,z,1.7,.22,4.95,"#2d7670")
    box(x+.4,.62,z,.69,.18,3.3,"#ba8b67")
    for _,leg in {-1,1} do box(x+.4,.29,z+leg,.12,.58,.17,METAL) end
    box(x+.12,1.42,z-1.65,.22,1.47,.67,"#e4d8b9",19)
    -- Facade-mounted awnings and named storefronts face the avenue.
    for _,side in {-1,1} do
        local ix=if side<0 then -1 else 0
        local r=noise(s.seed,ix,-1,2)
        local front=side*(28-(22+6*r)/2-3)
        local shade=if side<0 then "#c96644" else "#358780"
        for j=0,1 do
            local shopZ=-30+j*12
            box(front-side*.76,2.94,shopZ,1.52,.24,9.85,shade,18)
            box(front-side*1.47,2.72,shopZ,.10,.32,9.85,shade,18)
            sign(front-side*.17,3.61,shopZ,5.18,1.04,shade,if side<0 then 4+j else 3-j,-side*math.pi/2)
        end
    end
    -- Small corner kiosk with a striped canopy and newspaper racks.
    local kx,kz=17,-10.6
    obstacle(s,kx,1.04,kz,3.4,2.08,1.8,"#345e65",16)
    obstacle(s,kx,2.20,kz,3.85,.23,2.32,"#398783",18)
    sign(kx,1.74,kz+1.01,2.8,.68,"#39615f",3)
end

local function harbor()
    -- Two dockside gantries break the eastern silhouette above the sheds.
    for _,z in {-114,108} do
        box(181,-1.65,z,12,3.3,19,"#819190")
        for _,x in {176.8,184.8} do
            for _,j in {-1,1} do box(x,8.1,z+6*j,.72,16.2,.8,"#cd7049") end
        end
        for _,j in {-1,1} do box(187,16,z+j*6,24,.85,.95,"#de8856") end
        box(192,16,z,2.8,1.05,12.7,"#be6547")
        box(179,17,z-5,3.4,2.0,2.4,"#436d77",OFFICE)
        box(192,12.9,z,.18,5.7,.18,METAL)
        box(192,10,z,4.4,.4,2.0,"#c7aa66")
    end
    -- Moored cargo ferry and the low shape of an island on the horizon.
    box(210,-.55,46,11,2.3,50,"#3f5962")
    box(210,.68,46,10.8,.34,49,"#c0aa86")
    for i=0,3 do
        box(207.65,2.1,31+i*6,3.6,2.6,5.4,CONTAINERS[1+i],CONTAINER)
        box(212.1,2.1,31+i*6,3.6,2.6,5.4,CONTAINERS[4-i],CONTAINER)
    end
    box(210,3.15,61,9.5,4.7,9,LIMESTONE,HOTEL)
    box(210,6.12,62,10.4,1.4,7.9,"#728f97",OFFICE)
    -- A shared generated headland with beaches, rock shelves and a hilly spine.
    -- Split the far shore into overlapping capes around three separate bays.
    for _,p in {
        {435,-175,330,41,205,.58}, {527,-265,365,63,225,-.43},
        {-465,-355,620,72,250,-.32}, {-685,-413,410,87,205,.26},
        {-410,460,460,48,215,-.47}, {-583,538,470,62,228,.19},
        {680,420,520,74,230,-.53},
    } do
        world.spawn("assets/cape.glb",{pos=v(p[1],-3.3,p[2]),size=v(p[3],p[4],p[5]),
            rot=v(0,p[6],0),solid=false,tag="scenery"})
    end
end

function city.build(seed)
    seed = math.floor(seed or 1)
    local lawns = {patches={}, cutouts={}, seed=seed, growth=.30, distance=16}
    local s = { seed=seed, blocks={}, buildings={}, parks={}, walkLoops={}, plantSites={}, lamps={}, landmarks={} }
    for i=1,36 do s.blocks[i]={} end
    box(0,-3.9,0,30000,.8,30000,"#467e84",WATER)
    box(0,-2,0,BOUND*2,4,BOUND*2,"#61666a",ASPHALT,true)
    -- Keep walking and driving on the island, including along perimeter roads.
    for _,side in {-1,1} do
        box(side*(BOUND-.2),.35,0,.45,.7,BOUND*2,"#bcc6b9")
        box(0,.35,side*(BOUND-.2),BOUND*2,.7,.45,"#bcc6b9")
        box(side*(BOUND+.16),25,0,.32,52,BOUND*2,"#74898a",nil,true,{visible=false})
        box(0,25,side*(BOUND+.16),BOUND*2,52,.32,"#74898a",nil,true,{visible=false})
    end
    for ix=-3,2 do
        for iz=-3,2 do
            local x,z = (ix+.5)*STEP,(iz+.5)*STEP
            local pad = (ix==2 and iz==0) or (ix==-3 and iz==-2)
            local plaza = ix==0 and iz==0
            local garden = (ix==-2 and iz==1) or (ix==1 and iz==-2)
            if pad then helipad(x,z)
            elseif plaza or garden then park(s,x,z,plaza,lawns)
            elseif ix==2 then docks(s,x,z,ix,iz,seed)
            elseif (ix==-1 or ix==0 or ix==1) and (iz==-1 or iz==0) then office(s,x,z,ix,iz,seed)
            else hotel(s,x,z,ix,iz,seed) end
            -- One circuit per block, all of it on the continuous sidewalk.
            local reach=STEP/2-8
            table.insert(s.walkLoops,{v(x-reach,0,z-reach),v(x+reach,0,z-reach),v(x+reach,0,z+reach),v(x-reach,0,z+reach)})
            if plaza or garden then
                for _,dx in {-13,13} do
                    for _,dz in {-13,13} do
                        local pos=v(x+dx,0,z+dz)
                        table.insert(s.plantSites,pos)
                        if garden then table.insert(lawns.cutouts,{pos=pos,width=TREE_GRATE,depth=TREE_GRATE}) end
                    end
                end
            elseif iz==0 and not pad then
                -- Leave the path from the sidewalk to the front lobby clear.
                local treeX = if ix==-1 or ix==1 then x+5 else x
                table.insert(s.plantSites,v(treeX,0,z+18))
            end
            if (ix+iz)%2==0 then streetLamp(s,x-20.9,z+(if plaza then 20 else -15)) end
        end
    end
    grass.set(lawns)
    avenue(s)
    -- Vegetation supplies the visible trunk and crown; movement uses the trunk.
    for _,pos in s.plantSites do
        box(pos.x,.035,pos.z,TREE_GRATE,.04,TREE_GRATE,"#747e75",17)
        obstacle(s,pos.x,1.65,pos.z,.32,3.3,.32,"#655445",nil,{visible=false})
    end
    harbor()
    s.landmarks = {
        {name="Exchange Clock",pos=v(22,0,28),kind="walk"},
        {name="Lido Gardens",pos=v(-84,0,84),kind="walk"},
        {name="Market Green",pos=v(84,0,-84),kind="walk"},
        {name="Tide Market",pos=v(-53,0,87),kind="drive"},
        {name="Ferry Terminal",pos=v(164.5,0,59),kind="drive"},
        {name="Meridian Tower",pos=v(-28,0,-3),kind="drive"},
        {name="Harbor Heliport",pos=v(140,0,28),kind="fly"},
        {name="West Field",pos=v(-140,0,-84),kind="fly"},
    }
    return s
end

return city
