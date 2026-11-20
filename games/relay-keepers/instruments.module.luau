local M={}
local ink,iron,steel="#19363e","#3d5960","#749197"
local brass,cream,mint="#efaf57","#fff2c6","#80edca"
local violet="#c6b7ff"

-- Centered above a foot position. The gate uses the same silhouette in miniature.
function M.pedal(box,x,y,scale,held)
    local function part(dx,dy,w,h,c)
        box(x+dx*scale,y+dy*scale,w*scale,h*scale,c)
    end
    local top=if held then -11 else -24
    part(-27,-8,54,8,ink)
    part(-24,-5,48,3,steel)
    part(-11,top+8,22,-top-7,ink)
    part(-7,top+9,14,-top-8,iron)
    part(-23,top+3,46,8,ink)
    part(-20,top,40,10,ink)
    part(-18,top+2,36,4,if held then mint else brass)
    part(-19,top+6,38,3,if held then "#589885" else "#bb7743")
end

-- Two linked eyes hang below the walking surface so a relay and another control
-- can occupy exactly the same place. Both badges show who is standing on the pair.
local function relay(box,xy,l,s)
    local pair=l.links and l.links[s.link+1]
    if not pair then return end
    local lit={}
    for i,p in pair do
        lit[i]=false
        for _,r in s.crew do
            if r.on and math.abs(r.x-p[1])<0.62 and math.abs(r.y-p[2])<0.34 then lit[i]=true end
        end
    end
    for i,p in pair do
        local x,y=xy(p[1],p[2])
        local depth=if l.spring and math.abs(p[1]-l.spring[1])<0.7 and math.abs(p[2]-l.spring[2])<0.25 then 25 else 0
        for _,d in {-1,1} do
            box(x+d*25-4,y-4,8,9,ink)
            box(x+d*25-2,y-3,4,5,if lit[i] then cream else mint)
        end
        box(x-19,y+4+depth,38,20,ink)
        box(x-21,y+7+depth,42,13,ink)
        for j=1,2 do
            local cx=x+(j-1.5)*14
            local cy=y+depth+(if j==1 then 11 else 16)
            local color=if lit[j] then cream else "#69bba8"
            box(cx-8,cy-5,16,3,color)
            box(cx-8,cy+3,16,3,color)
            box(cx-10,cy-3,3,7,color)
            box(cx+7,cy-3,3,7,color)
            if lit[j] then box(cx-4,cy-1,8,3,cream) end
        end
    end
end

local function latch(box,x,y,tripped)
    local d=if tripped then 1 else -1
    box(x-17,y-7,34,7,ink)
    box(x-12,y-15,24,12,ink)
    box(x-7,y-11,14,6,if tripped then mint else steel)
    -- The handle throws across the pivot and stays over.
    for n=0,5 do
        box(x+d*n*3-4,y-15-n*4,8,7,ink)
        box(x+d*n*3-1,y-14-n*4,3,6,steel)
    end
    local knob=x+d*16
    box(knob-8,y-40,16,12,ink)
    box(knob-5,y-42,10,16,ink)
    box(knob-5,y-38,10,7,if tripped then mint else "#f78766")
    box(knob-4,y-38,5,3,cream)
    box(x-4,y-15,8,8,brass)
end

local function clockFace(box,x,y,past)
    box(x-17,y-6,34,6,ink)
    box(x-5,y-16,10,11,iron)
    local cy=y-31
    box(x-10,cy-20,20,40,ink)
    box(x-16,cy-16,32,32,ink)
    box(x-20,cy-10,40,20,ink)
    local rim=if past then brass else violet
    box(x-9,cy-17,18,34,rim)
    box(x-14,cy-13,28,26,rim)
    box(x-17,cy-8,34,16,rim)
    box(x-8,cy-13,16,26,cream)
    box(x-13,cy-8,26,16,cream)
    box(x-2,cy-12,4,13,ink)
    box(x+(if past then -12 else 0),cy-2,12,4,ink)
    box(x-3,cy-3,6,6,ink)
end

local function spring(box,x,y)
    -- The coil sits beneath the cap, at the actual takeoff line.
    box(x-18,y+21,36,6,ink)
    for n=0,2 do
        local cy=y+7*n
        box(x-15,cy,8,5,ink)
        box(x-10,cy+2,12,5,ink)
        box(x+1,cy+4,11,5,ink)
        box(x+10,cy+5,7,5,ink)
        box(x-13,cy,6,3,brass)
        box(x-7,cy+2,10,3,brass)
        box(x+3,cy+4,10,3,brass)
    end
    box(x-22,y-7,44,9,ink)
    box(x-19,y-6,38,3,"#ffdd77")
    -- A broad, stationary up-arrow is part of the spring cap.
    box(x-4,y-20,8,12,ink)
    box(x-8,y-16,16,8,ink)
    box(x-12,y-12,24,5,ink)
    box(x-2,y-17,4,8,brass)
    box(x-6,y-13,12,4,brass)
end

local function key(box,x,y)
    box(x-24,y-9,21,23,ink)
    box(x-28,y-5,29,15,ink)
    box(x-21,y-12,15,29,ink)
    box(x-22,y-6,18,17,brass)
    box(x-25,y-3,24,11,brass)
    box(x-18,y-9,10,23,brass)
    box(x-19,y-3,12,11,ink)
    box(x-21,y-7,10,4,cream)
    box(x-2,y-4,31,9,ink)
    box(x+13,y+2,7,10,ink)
    box(x+22,y+2,7,15,ink)
    box(x-2,y-2,29,5,brass)
    box(x+15,y+2,3,8,brass)
    box(x+24,y+2,3,13,brass)
end

-- Feather tips are stepped apart. The viewer can read the open fan in silhouette.
local function wings(box,x,y)
    for _,d in {-1,1} do
        local function feather(dx,dy,w,h,c)
            box(x+(if d==1 then dx else -dx-w),y+dy,w,h,c)
        end
        feather(2,-6,17,17,ink)
        feather(9,-12,17,16,ink)
        feather(18,-19,12,15,ink)
        feather(25,-23,7,13,ink)
        feather(17,-1,8,9,ink)
        feather(10,6,8,7,ink)
        feather(4,-3,11,10,cream)
        feather(10,-9,12,10,cream)
        feather(17,-14,10,9,cream)
        feather(25,-19,4,7,cream)
        feather(6,4,9,5,"#9cceca")
        feather(17,-2,6,5,"#9cceca")
    end
    box(x-5,y+1,10,12,ink)
    box(x-3,y+3,6,6,brass)
end

local function crate(box,x,y)
    box(x-27,y-50,54,50,ink)
    box(x-23,y-46,46,42,"#98673e")
    for n=1,2 do box(x-23+15*n,y-42,3,34,"#684c37") end
    for n=0,6 do
        box(x-20+n*5,y-41+n*4,9,7,"#c68a4f")
        box(x+12-n*5,y-41+n*4,9,7,"#edb672")
    end
    for _,dy in {-46,-9} do
        box(x-23,y+dy,46,5,"#e6aa65")
        for _,d in {-1,1} do box(x+d*19-1,y+dy+1,3,3,ink) end
    end
end

function M.draw(box,xy,l,s)
    if l.spring then
        local x,y=xy(l.spring[1],l.spring[2])
        spring(box,x,y)
    end
    if l.pad then
        local x,y=xy(l.pad[1],l.pad[2])
        M.pedal(box,x,y,1,s.pad)
        if l.timer then
            local top=if s.pad then -11 else -24
            local fill=30*math.clamp(s.timer/l.timer,0,1)
            box(x-15,y+top+2,30,4,ink)
            if fill>0 then box(x-15,y+top+2,fill,4,cream) end
        end
    end
    if l.latch then
        local x,y=xy(l.latch[1],l.latch[2])
        latch(box,x,y,s.latch)
    end
    if l.clock then
        local x,y=xy(l.clock[1],l.clock[2])
        clockFace(box,x,y,s.past)
    end
    relay(box,xy,l,s)
    if l.key and not s.key then
        local x,y=xy(l.key[1],l.key[2])
        key(box,x,y-49+2*math.sin(s.time*3))
    end
    if l.wings and s.owner==0 then
        local x,y=xy(l.wings[1],l.wings[2])
        wings(box,x,y-41+3*math.sin(s.time*3))
    end
    if s.crate then
        local x,y=xy(s.crate,0)
        crate(box,x,y)
    end
end

return M
