local ink, iron, steel = "#203a43", "#38555b", "#61807d"
local wood, honey, sun = "#a16d50", "#d6a674", "#ffe0a2"
local amber, teal, violet = "#ffbb69", "#78e2c7", "#c4c6fe"

local function timber(box,x,y,width,onGround)
    -- Each mounting shoe steps inward beneath the rail, like a cast corbel.
    local bays=math.max(1,math.floor((width+90)/155))
    for n=1,bays do
        local cx=x+width*(n-0.5)/bays
        box(cx-13,y+18,26,6,ink)
        box(cx-9,y+24,17,5,iron)
        box(cx-6,y+29,10,if onGround then 31 else 9,iron)
        box(cx-5,y+29,3,if onGround then 29 else 7,steel)
    end
    box(x,y,width,19,ink)
    box(x+2,y+3,width-4,7,wood)
    box(x,y,width,4,sun)
    box(x+2,y+4,width-4,3,honey)
    box(x+3,y+12,width-6,4,iron)
    local bayWidth=width/bays
    if bayWidth>68 then
        for n=1,bays do
            local mark=x+(n-1)*bayWidth+bayWidth*(if n%2==1 then 0.24 else 0.60)
            box(mark,y+7,15,2,"#edc395")
            box(mark+5,y+9,21,2,"#815c48")
        end
    end
    for n=1,bays-1 do
        local cx=x+width*n/bays
        box(cx-3,y+4,7,12,ink)
        box(cx-2,y+6,5,9,steel)
        box(cx-1,y+7,3,3,sun)
    end
    for _,cx in {x+4,x+width-8} do
        box(cx,y+11,4,4,honey)
    end
end

local function stackedHeads(box,cx,y)
    box(cx-9,y,18,18,ink)
    box(cx-4,y+1,8,3,amber)
    box(cx-6,y+4,12,4,amber)
    box(cx-5,y+10,3,2,teal)
    box(cx+2,y+10,3,2,teal)
    box(cx-6,y+12,12,5,teal)
end

local function signalRail(box,x,y,width,kind,active,balance)
    local temporal=kind=="p" or kind=="f"
    local paired=kind=="balance"
    local color=if kind=="p" then amber elseif kind=="f" then violet else teal
    if active then
        box(x,y,width,15,ink)
        box(x,y,width,4,if paired then sun else color)
        box(x+2,y+5,width-4,6,iron)
    else
        for d=3,width-7,21 do
            box(x+d,y+3,8,4,ink)
            box(x+d,y+3,6,2,color)
        end
    end
    if paired then
        -- The collision lip stays intact for the complete grace period.
        if active then
            local lit=math.clamp(balance/0.5,0,1)*(width-12)
            if lit>0 then box(x+(width-lit)/2,y+7,lit,3,teal) end
        end
        stackedHeads(box,x+11,y+6)
        if width>=48 then stackedHeads(box,x+width-11,y+6) end
    elseif temporal then
        -- Amber chevrons face back in time, violet chevrons face forward.
        local direction=if kind=="p" then -1 else 1
        local count=math.max(1,math.floor(width/78))
        for n=1,count do
            local cx=x+width*(n-0.5)/count
            box(cx-7,y+5,14,11,ink)
            box(cx-2*direction-2,y+6,4,2,color)
            box(cx-2,y+8,4,2,color)
            box(cx+2*direction-2,y+10,4,2,color)
            box(cx-2,y+12,4,2,color)
            box(cx-2*direction-2,y+14,4,2,color)
        end
    else
        -- Twin contacts echo the two pads that energize a linked span.
        local count=math.max(1,math.floor(width/75))
        for n=1,count do
            local cx=x+width*(n-0.5)/count
            box(cx-9,y+6,18,10,ink)
            box(cx-6,y+8,4,4,color)
            box(cx+2,y+8,4,4,color)
            if active then box(cx-2,y+10,4,2,color) end
        end
    end
end

local function glassRail(box,x,y,width,age)
    local aqua,facet,glint="#71c1c7","#adede2","#edfff2"
    local panes=math.max(1,math.floor(width/38))
    if age>=0.42 then
        for d=4,width-6,24 do
            box(x+d,y+4,6,3,"#54959e")
        end
        if age<0.68 then
            local t=(age-0.42)/0.26
            for n=1,panes do
                local cx=x+width*(n-0.5)/panes
                local drop=t*t*(25+8*(n%2))
                box(cx-5,y+9+drop,7,4,aqua)
                box(cx-2,y+13+drop,3,3,facet)
            end
        end
        return
    end
    box(x+2,y+2,width-4,13,ink)
    box(x,y,width,4,glint)
    box(x+2,y+4,width-4,7,aqua)
    for n=1,panes do
        local cx=x+width*(n-0.5)/panes
        local facetWidth=if n%2==1 then 15 else 9
        box(cx-8,y+4,facetWidth,3,facet)
        box(cx-5,y+7,facetWidth-5,3,facet)
        box(cx-2,y+11,8,5,ink)
        box(cx,y+11,4,3,aqua)
        if age>0 then
            -- A dark fork cuts the bright edge as soon as someone lands.
            box(cx+6,y,4,4,ink)
            box(cx+3,y+3,4,4,ink)
            if age>0.16 then
                box(cx,y+6,5,4,ink)
                box(cx+6,y+6,4,4,ink)
            end
            if age>0.29 then
                box(cx-3,y+9,4,4,ink)
                box(cx+9,y+9,4,4,ink)
            end
        end
    end
end

local function draw(box,xy,l,s)
    for i,span in l.land do
        local x,y=xy(span[1],span[3])
        local right=xy(span[2],span[3])
        local width=right-x
        local kind=span[4]
        if kind=="glass" then
            glassRail(box,x,y,width,s.glass[tostring(i)] or 0)
        elseif kind=="balance" then
            local remaining=s.balance or 0
            signalRail(box,x,y,width,kind,remaining>0,remaining)
        elseif kind=="p" or kind=="f" then
            signalRail(box,x,y,width,kind,s.latch or s.past==(kind=="p"),0)
        elseif type(kind)=="number" then
            signalRail(box,x,y,width,kind,s.link>=kind,0)
        else
            timber(box,x,y,width,span[3]==0)
        end
    end
end

return {draw=draw}
