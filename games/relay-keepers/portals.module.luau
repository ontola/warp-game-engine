local M={}
local colors={"#dfafff","#ffba88","#8be9ed"}
local shadows={"#80629b","#ac785a","#50888e"}
local ink,cream="#203245","#fff0cb"

function M.step(l,s)
    if not l.portals then return end
    for i,r in s.crew do
        local contact=false
        for n,pair in l.portals do
            for side,p in pair do
                local dx,dy=math.abs(r.x-p[1]),math.abs(r.y-p[2])
                local id=2*n+side
                if r.foldLock==id and dx<0.76 and dy<0.68 then
                    contact=true
                elseif r.on and dx<0.37 and dy<0.22 then
                    local dest=pair[3-side]
                    r.x,r.y,r.vy=dest[1],dest[2],0
                    r.foldLock=2*n+3-side
                    r.on,r.ride,r.deck=true,false,false
                    r.buffer,r.coyote,r.springImpulse=0,0,false
                    r.extra=s.owner==i
                    r.spawnGlow=1
                    sound.play("fold",{pitch=0.77+n*0.15+(side-1)*0.08,db=-5})
                    contact=true
                    break
                end
            end
            if contact then break end
        end
        if not contact then r.foldLock=0 end
    end
end

-- A suspended lens. The dark centre and the cuts at the rim keep its round
-- silhouette legible against the sky. Combine consecutive equal pixels once.
local lenses={}
local cores={"#383358","#553d44","#244550"}
for row=-7,7 do
    local previous=nil
    local first=-7
    for col=-7,8 do
        local radius=row*row+col*col
        local tone=nil
        if col<=7 and radius<=57 then
            if radius>=47 then tone=1
            elseif radius>=32 and (row==0 or col==0) then tone=1
            elseif radius>=36 then tone=2
            elseif radius>=32 then tone=3
            else tone=4 end
        end
        if previous~=tone then
            if previous then table.insert(lenses,{first*5-2.5,row*5-2.5,(col-first)*5,previous}) end
            previous,first=tone,col
        end
    end
end

local function pairMark(box,x,y,n,color,core)
    if n==1 then
        box(x-4,y-13,8,26,color)
        box(x-9,y-8,18,16,color)
        box(x-14,y-3,28,6,color)
        box(x-3,y-3,6,6,core)
    elseif n==2 then
        box(x-12,y-12,8,24,color)
        box(x+4,y-12,8,24,color)
    else
        box(x-4,y-13,8,26,color)
        box(x-13,y-4,26,8,color)
    end
end

function M.draw(box,xy,l,s)
    if not l.portals then return end
    for n,pair in l.portals do
        local mark=(n-1)%#colors+1
        local c,core=colors[mark],cores[mark]
        local palette={ink,c,shadows[mark],core}
        -- Both mouths of a pair turn on exactly the same beat.
        local angle=s.time*1.65+n*1.9
        for _,p in pair do
            local x,floorY=xy(p[1],p[2])
            local y=floorY-51+math.sin(angle)*2
            box(x-16,floorY-4,32,4,"#20324565")
            for _,r in lenses do box(x+r[1],y+r[2],r[3],5,palette[r[4]]) end
            for i=0,2 do
                local a=angle+i*math.pi*2/3
                for trail=0,2 do
                    local t=a-trail*0.23
                    local radius=22-trail
                    local size=6-trail
                    box(x+math.cos(t)*radius-size/2,y+math.sin(t)*radius-size/2,size,size,
                        if trail==0 then cream else shadows[mark])
                end
            end
            pairMark(box,x,y,mark,c,core)
        end
    end
end

return M
