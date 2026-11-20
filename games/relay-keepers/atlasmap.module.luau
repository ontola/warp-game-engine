local coast=require("coast")
local M={}

local ink,rail,ivory="#142c37","#42606a","#fff2ce"
local amber,mint,violet="#ffc275","#86efda","#c5baff"
-- Each stamp is the silhouette on that district's skyline.
local stamps={
    beacon={{8,1,7,3},{7,5,9,3},{9,9,5,2},{8,11,7,5},{6,16,11,3}},
    sails={{11,1,2,14},{8,4,2,3},{6,7,4,3},{3,10,7,4},{14,7,3,6},{1,15,20,2},{4,17,14,2}},
    yard={{2,2,17,3},{16,5,3,14},{4,5,2,6},{3,11,4,3},{9,16,6,3}},
    mill={{8,9,6,10},{3,2,4,3},{6,5,4,3},{10,8,3,3},{13,5,4,3},{16,2,4,3},{4,12,4,3},{2,15,4,3},{15,12,4,3}},
    observatory={{10,1,4,3},{5,4,14,3},{3,7,18,4},{4,13,16,5},{2,18,20,2}},
    kilns={{3,3,4,8},{14,1,4,10},{1,11,19,8}},
    ice={{9,0,3,9},{5,4,3,10},{1,8,3,8},{13,9,3,6},{17,3,3,8}},
    glass={{9,1,4,3},{6,4,10,3},{3,7,16,3},{2,11,3,7},{10,11,2,7},{17,11,3,7},{1,18,20,2}},
    trestle={{2,2,3,17},{17,2,3,17},{5,5,3,3},{8,8,6,3},{14,5,3,3},{0,12,22,3}},
    lantern={{10,0,2,4},{7,4,8,2},{5,7,12,2},{6,10,2,5},{10,9,2,6},{14,10,2,5},{7,16,8,2},{10,18,2,2}},
    water={{5,1,8,3},{3,4,6,3},{1,7,5,4},{1,11,8,3},{8,13,6,3},{14,11,5,3},{18,8,3,3},{2,17,18,2}},
    crab={{1,3,3,5},{6,3,3,5},{14,3,3,5},{19,3,3,5},{2,8,6,3},{15,8,6,3},{5,11,13,5},{3,16,4,3},{10,16,3,3},{17,16,3,3}},
    portal={{8,0,6,2},{4,3,4,3},{14,3,4,3},{2,7,3,6},{17,7,3,6},{4,14,4,3},{14,14,4,3},{8,18,6,2},{10,8,3,4}},
}
local stocks={
    beacon="#394655", sails="#304655", yard="#334c50", mill="#454a42",
    observatory="#383c58", kilns="#513e48", ice="#2b4050", glass="#304b47",
    trestle="#453745", lantern="#503735",
    water="#234b54", crab="#503f42", portal="#423959",
}
local function wx(x) return math.floor(5+x*6.75+0.5) end
local function wy(y) return math.floor(48-y*5.7+0.5) end

-- Bake fixed thumbnail geometry once. Cursor and player location remain live.
local cache={}
local function card(l)
    if cache[l] then return cache[l] end
    local palette,kind=coast.style(l)
    if not stamps[kind] then kind="beacon" end
    local geometry={}
    local function rect(x,y,w,h,c) table.insert(geometry,{x,y,w,h,c}) end
    local function mark(p,w,h,c,lift)
        if p then rect(wx(p[1])-math.floor(w/2),wy(p[2])-lift-h,w,h,c) end
    end
    if l.water then
        local left,right,y=wx(l.water.left),wx(l.water.right),wy(l.water.low)
        rect(left,y,right-left,math.max(1,56-y),"#42727a")
        rect(left,y,right-left,1,"#8cdccf")
    end
    for _,span in l.land do
        local mat=span[4]
        local c=if mat=="f" then violet elseif mat=="p" then amber elseif mat=="glass" then "#a9dfef" elseif mat=="balance" then "#efaac6" elseif type(mat)=="number" then "#7ca79c" else "#c3d0c0"
        local x,y=wx(span[1]),wy(span[3])
        local w=math.max(1,wx(span[2])-x)
        rect(x,y,w,3,c)
        if mat=="balance" then
            rect(x,y-3,2,3,c)
            rect(x+w-2,y-3,2,3,c)
            rect(x+math.floor(w/2)-1,y+3,3,2,c)
        end
    end
    if l.ferry then
        local f=l.ferry
        local ox,oy=wx(f.from[1]),wy(f.from[2])
        local tx,ty=wx(f.to[1]),wy(f.to[2])
        for i=1,3 do
            rect(math.floor(ox+(tx-ox)*i/4),math.floor(oy+(ty-oy)*i/4)+2,2,2,"#90b1b0")
        end
        rect(ox-5,oy,11,3,if f.coil then amber else mint)
    end
    if l.gate then
        local y=wy(l.gateHeight or 6.9)
        rect(wx(l.gate)-1,y,2,wy(0)-y,"#986f66")
    end
    if l.lock then rect(wx(l.lock)-1,wy(6.9),2,wy(0)-wy(6.9),"#9e845d") end
    if l.crabs then
        for _,crab in l.crabs do
            local x,y=wx(crab.from+(crab.to-crab.from)*(crab.phase or 0)),wy(crab.y)
            rect(x-3,y-3,6,3,"#edab94")
            rect(x-2,y-5,3,2,"#edab94")
        end
    end
    if l.portals then
        local colors={"#dfafff","#ffba88","#8be9ed"}
        for n,pair in l.portals do
            for _,p in pair do
                local x,y=wx(p[1]),wy(p[2])
                rect(x-3,y-6,2,4,colors[n])
                rect(x+2,y-6,2,4,colors[n])
                rect(x-1,y-8,3,2,colors[n])
                rect(x-1,y-2,3,2,colors[n])
            end
        end
    end
    if l.links then
        for _,pair in l.links do
            for _,p in pair do mark(p,4,2,"#8ccbb8",1) end
        end
    end
    mark(l.pad,6,2,amber,0)
    mark(l.crate,5,5,"#e1a475",0)
    mark(l.clock,5,5,violet,3)
    mark(l.key,6,2,amber,5)
    mark(l.key,3,3,amber,6)
    mark(l.spring,7,3,amber,0)
    mark(l.wings,7,2,ivory,6)
    mark(l.wings,3,3,ivory,3)
    local function exit(p,c)
        if p then
            local x,y=wx(p[1]),wy(p[2])
            rect(x-2,y-7,2,7,c)
            rect(x,y-8,4,2,c)
            rect(x+3,y-6,2,6,c)
        end
    end
    exit(l.goal,if l.secondGoal then amber else mint)
    exit(l.secondGoal,mint)
    local tile={palette=palette,kind=kind,steps=geometry}
    cache[l]=tile
    return tile
end

local function shoulders(box,cx,y,left,enabled)
    local c=if enabled then ivory else rail
    -- The full pad locates the highlighted bumper even without a button label.
    box(cx-16,y+4,32,9,rail)
    box(cx-17,y+12,8,4,rail)
    box(cx+9,y+12,8,4,rail)
    box(cx-14,y,10,3,if left then c else rail)
    box(cx+4,y,10,3,if left then rail else c)
    box(cx-12,y+8,6,2,ink)
    box(cx-10,y+6,2,6,ink)
    box(cx+8,y+7,3,3,ink)
    local x=cx+(if left then -30 else 30)
    for i=0,2 do
        box(x+(if left then i else -i)*3,y+7-i*2,3,3,c)
        if i>0 then box(x+(if left then i else -i)*3,y+7+i*2,3,3,c) end
    end
end

function M.draw(box,rooms,s)
    local count=#rooms
    local rows=math.ceil(count/7)
    local focus=math.floor((s.mapRoom-1)/7)
    local first=math.clamp(focus-6,0,math.max(0,rows-8))
    box(0,0,1200,640,ink)
    for row=first,math.min(rows-1,first+7) do
        local begin=row*7+1
        local y=17+(row-first)*72
        local chapter=card(rooms[begin])
        local accent=chapter.palette[2]
        local active=row==focus
        box(43,y+27,1122,3,if active then accent else rail)
        box(16,y+14,32,31,if active then accent else rail)
        box(18,y+16,28,27,stocks[chapter.kind])
        for _,r in stamps[chapter.kind] do box(21+r[1],y+19+r[2],r[3],r[4],accent) end
        for n=begin,math.min(begin+6,count) do
            local x=64+(n-begin)*159
            local tile=card(rooms[n])
            local selected=n==s.mapRoom
            box(x-2,y-2,147,62,if selected then ivory else rail)
            box(x,y,143,58,stocks[tile.kind])
            for _,r in tile.steps do box(x+r[1],y+r[2],r[3],r[4],r[5]) end
            if n==s.room then
                box(x+4,y+3,12,7,ink)
                box(x+5,y+4,4,4,amber)
                box(x+10,y+4,4,4,mint)
            end
            if selected then
                box(x+60,y-5,23,4,ivory)
                box(x+65,y+60,13,3,ivory)
            end
        end
    end
    -- One notch per set of seven rooms; the illuminated rail is the viewport.
    local step=math.min(28,math.floor(660/math.max(rows,1)))
    local width=step*(rows-1)
    local left=math.floor((1200-width)/2)
    box(left,610,math.max(1,width),2,rail)
    for row=0,rows-1 do
        local x=left+row*step
        local c=if row==focus then ivory elseif row>=first and row<first+8 then card(rooms[row*7+1]).palette[2] else rail
        box(x-3,607,7,8,c)
        if row==focus then box(x-5,617,11,2,ivory) end
    end
    shoulders(box,left-46,603,true,s.mapRoom>1)
    shoulders(box,left+width+46,603,false,s.mapRoom<count)
end

return M
