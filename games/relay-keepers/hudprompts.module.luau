-- The same physical pad and paper map appear on either side of the atlas.
local M={}
local ink,edge,paper="#16303c","#527178","#fff0c6"

local function controller(box,x,y,face)
    local body,shade,well="#c6d5c9","#91aca7","#25404a"
    -- Shoulder caps, two angled grips and the central cutout form the silhouette.
    box(x+9,y+2,13,4,shade)
    box(x+37,y+2,13,4,shade)
    box(x+9,y+5,41,23,body)
    box(x+6,y+8,47,17,body)
    for i=0,3 do
        local h=16-i*2
        box(x+6-i*2,y+17+i*3,h,6,body)
        box(x+53+i*2-h,y+17+i*3,h,6,body)
    end
    box(x+1,y+31,10,3,shade)
    box(x+49,y+31,10,3,shade)
    box(x+19,y+24,21,4,shade)
    -- The D-pad sits on the left. Two low thumbsticks anchor the orientation.
    box(x+13,y+10,5,13,well)
    box(x+9,y+14,13,5,well)
    for _,sx in {22,31} do
        box(x+sx,y+22,5,2,shade)
        box(x+sx-1,y+24,7,3,well)
        box(x+sx,y+27,5,2,well)
    end
    box(x+24,y+12,4,2,shade)
    box(x+31,y+12,4,2,shade)
    local positions={{44,10},{50,16},{44,22},{38,16}}
    for i,p in positions do
        local cx,cy=x+p[1],y+p[2]
        if i==face then
            box(cx-3,cy-5,6,10,ink)
            box(cx-5,cy-3,10,6,ink)
            local c=if face==1 then "#ffd258" elseif face==3 then "#84e1a3" else "#85cef1"
            box(cx-3,cy-4,6,8,c)
            box(cx-4,cy-3,8,6,c)
            box(cx-2,cy-3,3,2,paper)
        else
            box(cx-2,cy-2,5,5,well)
        end
    end
end

local function foldedMap(box,x,y,close)
    -- Unequal ridges make the concertina recognizable even in monochrome.
    local facets={paper,"#a6c7bb","#f1dca4"}
    for i=0,8 do
        local fold=math.floor(i/3)+1
        local rise=if fold==2 then 2-i%3 else i%3
        local xx,yy=x+4*i,y+4-2*rise
        box(xx,yy,4,27,facets[fold])
    end
    box(x+4,y+16,7,3,"#678f8e")
    box(x+9,y+18,5,3,"#678f8e")
    box(x+15,y+17,5,3,"#678f8e")
    -- A red pin with a punched centre and a tapered tip.
    box(x+20,y+2,9,2,ink)
    box(x+18,y+4,13,8,ink)
    box(x+20,y+12,9,3,ink)
    box(x+22,y+15,5,3,ink)
    box(x+24,y+18,2,2,ink)
    box(x+21,y+4,7,8,"#e87856")
    box(x+20,y+6,9,4,"#e87856")
    box(x+23,y+12,3,3,"#e87856")
    box(x+23,y+6,3,3,paper)
    if close then
        box(x+27,y+20,14,13,ink)
        for i=0,2 do
            box(x+30+i*3,y+22+i*3,3,3,paper)
            box(x+36-i*3,y+22+i*3,3,3,paper)
        end
    end
end

function M.draw(box,x,y,action)
    box(x+4,y,119,47,ink)
    box(x,y+4,127,39,ink)
    box(x+4,y+44,119,3,edge)
    local face=if action=="feedback" then 4 elseif action=="play" then 3 else 1
    controller(box,x+6,y+5,face)
    for n=0,2 do
        box(x+69+n*2,y+18+n*2,3,3,"#8baaa6")
        box(x+69+n*2,y+26-n*2,3,3,"#8baaa6")
    end
    if action=="feedback" then
        box(x+85,y+9,31,21,paper)
        box(x+89,y+29,9,4,paper)
        box(x+89,y+32,5,3,paper)
        box(x+94,y+14,4,11,ink)
        box(x+102,y+14,4,11,ink)
    elseif action=="play" then
        for i=0,5 do box(x+91+i*4,y+10+i*2,4,24-i*4,paper) end
    else
        foldedMap(box,x+82,y+8,action=="close")
    end
end
return M
