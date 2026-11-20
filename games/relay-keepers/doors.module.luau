local instruments=require("instruments")
local M={}
local ink,cream="#193540","#fff0c9"
local amber,mint="#ffc569","#8ee5c9"

-- A finish is a race flag over a landing area. The platform under it remains visible.
function M.finish(box,xy,p,split,s)
    local x,y=xy(p[1],p[2])
    local mast=x-32
    box(mast-5,y-4,15,4,ink)
    box(mast,y-86,6,82,ink)
    box(mast+1,y-85,2,79,"#b9c9bb")
    box(mast-1,y-90,8,6,ink)
    local flap=math.sin(s.time*2.6)
    for col=0,4 do
        local fy=y-82+math.floor((col/4)^1.5*flap*4)
        local fx=mast+6+col*7
        box(fx,fy-2,7,25,ink)
        for row=0,2 do
            box(fx,fy+row*7,7,7,if (row+col)%2==0 then cream else "#29454d")
        end
    end
    -- One berth per keeper at a shared exit, or one at each end of a split exit.
    local inside={}
    for i,r in s.crew do
        if r.id~=0 and r.on and math.abs(r.x-p[1])<0.85 and math.abs(r.y-p[2])<0.34 then
            inside[i]=true
        end
    end
    local slots=if split then 1 else 2
    for i=1,slots do
        local cx=x+(i-(slots+1)/2)*25
        local who=if split then (if inside[1] then 1 elseif inside[2] then 2 else nil) elseif inside[i] then i else nil
        local c=if who==1 then amber elseif who==2 then mint else "#91a59c"
        box(cx-10,y-5,20,5,ink)
        box(cx-9,y-7,18,3,c)
        if who then
            box(cx-8,y+2,16,5,c)
        else
            box(cx-7,y+2,14,5,ink)
            box(cx-5,y+3,10,2,c)
        end
    end
end

function M.barrier(box,xy,bar,open,height,pressure)
    if not bar then return end
    local x,y=xy(bar,0)
    local high=56*(height or 6.9)
    local crown=y-high
    box(x-18,crown,36,12,ink)
    box(x-14,crown+2,28,4,"#97aaa4")
    box(x-16,y-5,32,5,ink)
    if open then
        box(x-12,crown+12,24,7,"#467870")
        box(x-8,crown+13,16,4,mint)
        box(x-8,y-3,16,3,mint)
        return
    end
    -- A continuous shutter blocks the path. Diagonal stripes carry the warning.
    box(x-13,crown+12,26,high-12,ink)
    box(x-10,crown+12,20,high-17,"#6b7370")
    box(x+7,crown+12,3,high-17,"#404e50")
    for offset=21,high-25,60 do
        for i=0,4 do box(x-10+i*4,crown+offset+8-i*2,4,7,"#f3b86f") end
    end
    local cy=y-math.min(48,high*0.42)
    box(x-18,cy-17,36,34,ink)
    box(x-16,cy-15,32,30,"#435652")
    if pressure then
        instruments.pedal(box,x,cy+8,0.53,false)
    else
        box(x-7,cy-14,14,4,cream)
        box(x-9,cy-11,4,11,cream)
        box(x+5,cy-11,4,11,cream)
        box(x-12,cy-3,24,17,amber)
        box(x-4,cy,8,6,ink)
        box(x-2,cy+5,4,5,ink)
    end
end
return M
