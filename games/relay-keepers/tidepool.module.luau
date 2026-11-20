local M={}
local ink,foam,aqua="#234650","#c8fff0","#72daca"

function M.start(spec)
    if not spec then return nil end
    return {y=spec.low,prev=spec.low,open=false}
end

function M.step(spec,water,crew,dt)
    if not water then return end
    local open=false
    if spec.control then
        for _,r in crew do
            if r.on and math.abs(r.x-spec.control[1])<0.56 and math.abs(r.y-spec.control[2])<0.27 then
                open=true
                break
            end
        end
    end
    if open~=water.open then sound.play("tide",{pitch=if open then 1.45 else 0.78,db=-8}) end
    water.open=open
    water.prev=water.y
    local target=if open then spec.high else spec.low
    water.y+=math.clamp(target-water.y,-1.35*dt,1.35*dt)
end

local function inside(spec,x)
    return spec and x>=spec.left and x<=spec.right
end

function M.drift(spec,water,r,dt)
    if water and inside(spec,r.x) and r.y<=water.y+0.04 and r.y>=-1.95 then
        return (spec.current or 0)*dt
    end
    return 0
end

function M.float(spec,water,r,x,dt,pad)
    r.diving=false
    if not water or not inside(spec,x) or r.y>water.y+0.11 or r.y< -1.95 then return end
    r.diving=pad and (pad.held.down or (pad.move_y or 0)<-0.55) or false
    if r.diving then
        r.vy+=math.clamp(-2.65-r.vy,-13*dt,18*dt)
        r.vy=math.max(r.vy,-2.65)
    elseif r.y<water.y-0.035 then
        -- Added after gravity. A submerged keeper slows, turns and surfaces.
        r.vy=math.min(3.1,r.vy+28*dt)
    end
end

function M.support(spec,water,r,x,prev)
    if not water or not inside(spec,x) or r.diving then return nil end
    local surface=water.y
    -- A falling surface carries its swimmers down, while a swimmer rising
    -- from the bed settles onto the same surface at the instant they cross it.
    if prev>=water.prev-0.05 and r.y<=surface+0.052 and r.vy<=0 then return surface end
    if prev<surface and r.y>=surface and r.vy>0 then return surface end
    return nil
end

function M.draw(box,xy,l,s)
    local spec,water=l.water,s.water
    if not spec or not water then return end
    local x,y=xy(spec.left,water.y)
    local right,bottom=xy(spec.right,-1.85)
    local width,height=right-x,bottom-y
    box(x,y,width,height,"#3a8589")
    box(x,y,width,height*0.47,"#4b9b99")
    box(x,y,width,height*0.23,"#5ab3ad")
    box(x,y,width,4,aqua)
    local time=s.time
    local current=spec.current or 0
    local flow=if current<0 then -1 else 1
    for i=0,math.floor(width/55)-1 do
        local offset=7+(i*55+flow*time*12)%(width-28)
        local wave=math.sin(time*2+i)*1.4
        box(x+offset,y+wave,16,3,foam)
        box(x+offset+5,y+wave+3,14,2,aqua)
    end
    for i=1,math.floor(width/78) do
        local cx=x+12+(i*79+current*time*25)%(width-24)
        local cy=y+height*(0.36+0.16*(i%3))
        box(cx-8,cy,16,2,"#73c1b1")
        if current~=0 then
            box(cx+flow*7-2,cy-2,4,6,"#8cd8c6")
            box(cx+flow*10-1,cy,2,2,"#8cd8c6")
        end
    end
    -- Submerged salvage is marked by a buoy. Its down input and sinking
    -- bubbles teach the dive before either keeper has to try it.
    if l.key and not s.key and inside(spec,l.key[1]) and l.key[2]<spec.low-0.5 then
        local bx=xy(l.key[1],water.y)
        local by=y+math.sin(time*2.8)*2
        box(bx-8,by-5,16,6,ink)
        box(bx-6,by-7,12,4,"#ffbf73")
        box(bx-2,by-12,4,5,ink)
        box(bx-8,by+6,16,15,ink)
        box(bx-2,by+8,4,6,foam)
        box(bx-5,by+13,10,3,foam)
        box(bx-2,by+16,4,3,foam)
        local depth=(time*17)%20
        box(bx-2,by+24+depth,4,4,aqua)
    end
    for _,r in s.crew do
        if inside(spec,r.x) and r.y<water.y-0.15 then
            local rx,ry=xy(r.x,r.y)
            for i=0,2 do
                local rise=(time*24+i*13)%38
                local by=ry-20-rise
                if by>y+4 then box(rx+math.sin(rise*0.15+i)*7,by,4,4,foam) end
            end
        end
    end
    if spec.control then
        local px,py=xy(spec.control[1],spec.control[2])
        box(px-20,py-5,40,5,ink)
        box(px-16,py-(if water.open then 8 else 12),32,5,if water.open then foam else aqua)
        box(px-7,py-32,14,16,ink)
        box(px-3,py-29,6,3,aqua)
        box(px-5,py-25,10,5,aqua)
        local highX,highY=xy(spec.left,spec.high)
        local _,lowY=xy(spec.left,spec.low)
        box(highX+3,highY,4,lowY-highY+3,ink)
        box(highX+4,highY,10,3,foam)
        box(highX+4,lowY,10,3,foam)
        box(highX+2,y-2,13,4,aqua)
    end
end

return M
