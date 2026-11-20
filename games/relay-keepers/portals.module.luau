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

function M.draw(box,xy,l,s)
    if not l.portals then return end
    for n,pair in l.portals do
        local c,shadow=colors[n],shadows[n]
        local phase=s.time*2.7+n
        for _,p in pair do
            local x,y=xy(p[1],p[2])
            -- Broken concentric rings keep the approach visible through the mouth.
            box(x-20,y-85,40,7,ink)
            box(x-18,y-83,36,3,c)
            for _,d in {-1,1} do
                box(x+26*d-7,y-77,14,10,ink)
                box(x+32*d-4,y-66,8,42,ink)
                box(x+26*d-6,y-23,12,12,ink)
                box(x+20*d-6,y-10,12,7,ink)
                box(x+25*d-4,y-75,8,5,c)
                box(x+32*d-2,y-62,4,34,shadow)
                box(x+26*d-3,y-21,6,7,c)
                local offset=(s.time*24+17*(d+1))%34
                box(x+32*d-2,y-62+offset,4,5,c)
            end
            box(x-26,y-4,52,4,shadow)
            -- Matching colours, glyph counts and pulse phases identify each pair.
            for k=1,n do
                local gx=x+(k-(n+1)/2)*10
                local gy=y-92+math.sin(phase)*2
                box(gx-3,gy-3,6,6,ink)
                box(gx-2,gy-2,4,4,c)
            end
            for k=0,2 do
                local a=phase+k*math.pi*2/3
                box(x+math.cos(a)*13-2,y-45+math.sin(a)*19-2,4,4,if k==0 then cream else c)
            end
        end
    end
end

return M
