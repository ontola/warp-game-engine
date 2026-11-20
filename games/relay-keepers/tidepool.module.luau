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

local function drop(box,x,y,c)
    box(x-2,y-9,4,4,c)
    box(x-4,y-5,8,4,c)
    box(x-7,y-1,14,8,c)
    box(x-4,y+7,8,3,c)
end

function M.draw(box,xy,l,s)
    local spec,water=l.water,s.water
    if not spec or not water then return end
    local x,y=xy(spec.left,water.y)
    local right,bottom=xy(spec.right,-1.85)
    local width,height=right-x,bottom-y
    local time=s.time
    local current=spec.current or 0
    local flow=if current<0 then -1 else 1
    box(x,y+3,width,height-3,"#1a5871")
    box(x,y+3,width,height*0.67,"#216e86")
    box(x,y+3,width,height*0.29,"#2a899b")
    -- The crests move along the playable surface at the swimmers' feet.
    -- Keep their stepped outer contour distinct from the straight deck edges.
    for i=0,math.ceil(width/13)-1 do
        local dx=i*13
        local section=math.min(13,width-dx)
        local wave=math.sin(i*0.63-flow*time*2.2)
        local crest=y+math.floor(wave*2.1+0.5)*2
        box(x+dx,crest-3,section,6,ink)
        box(x+dx,crest,section,7,if wave<0.10 then foam else aqua)
        if wave< -0.48 then box(x+dx,crest+7,section,4,"#55c6d4") end
    end
    -- A moving trail and its broad arrowhead show the true current direction.
    if current~=0 then
        for i=0,math.floor(width/105)-1 do
            local cx=x+17+(i*105+current*time*38)%(width-34)
            local cy=y+height*(0.38+0.15*(i%3))
            box(cx-12,cy-2,24,4,"#69b6bf")
            box(cx+flow*5-2,cy-6,5,12,"#69b6bf")
            box(cx+flow*10-2,cy-4,5,8,"#69b6bf")
        end
    end
    -- A few strands on the bed give the pool depth without obscuring salvage.
    for i=0,math.floor(width/150)-1 do
        local kx=x+24+i*150
        local turn=if math.sin(time*1.5+i)>0 then 3 else 0
        box(kx,bottom-17,5,17,"#206571")
        box(kx+3,bottom-27,5,13,"#206571")
        box(kx+turn,bottom-32,5,7,"#206571")
        box(kx+11,bottom-13,4,13,"#206571")
    end
    if l.key and not s.key and inside(spec,l.key[1]) and l.key[2]<spec.low-0.5 then
        local bx=xy(l.key[1],water.y)
        local by=y+math.sin(time*2.8)*2
        box(bx-2,by-20,4,16,ink)
        box(bx-2,by-21,13,7,"#ffab70")
        box(bx-11,by-7,22,13,ink)
        box(bx-15,by-3,30,6,ink)
        box(bx-9,by-5,18,9,"#ef8c58")
        box(bx-11,by-2,22,3,foam)
        box(bx-13,by+8,26,20,ink)
        box(bx-2,by+10,4,10,foam)
        box(bx-8,by+17,16,4,foam)
        box(bx-5,by+21,10,3,foam)
        box(bx-2,by+24,4,2,foam)
    end
    for _,r in s.crew do
        if inside(spec,r.x) and r.y<water.y-0.15 then
            local rx,ry=xy(r.x,r.y)
            for i=0,2 do
                local rise=(time*27+i*18)%54
                local by=ry-20-rise
                if by>y+7 then
                    local bx=rx+math.sin(rise*0.13+i)*9-9
                    box(bx-3,by-2,6,5,aqua)
                    box(bx-2,by-3,4,7,aqua)
                    box(bx-1,by-1,3,3,"#216e86")
                end
            end
        end
    end
    if spec.control then
        local px,py=xy(spec.control[1],spec.control[2])
        local side=if px<(x+right)/2 then -1 else 1
        local stand=px+side*29
        -- The floor tread and its side-mounted enamel sign remain readable
        -- when a room puts a relay contact on the same tile.
        box(px-23,py-6,46,6,ink)
        box(px-19,py-(if water.open then 8 else 13),38,6,if water.open then foam else aqua)
        box(stand-3,py-30,6,30,ink)
        box(stand-12,py-49,24,24,ink)
        box(stand-15,py-45,30,16,ink)
        drop(box,stand,py-37,if water.open then foam else aqua)
        local highX,highY=xy(spec.left,spec.high)
        local _,lowY=xy(spec.left,spec.low)
        local gx=highX+9
        box(gx-4,highY-5,8,lowY-highY+12,ink)
        for i=0,4 do
            local mark=highY+(lowY-highY)*i/4
            local len=if i%2==0 then 14 else 9
            box(gx-2,mark,len,3,foam)
        end
        box(gx-11,y-5,24,10,ink)
        box(gx-8,y-2,18,4,aqua)
        box(gx-9,highY-27,18,23,ink)
        drop(box,gx,highY-17,aqua)
    end
end

return M
