-- Relief-print keepers. Rows of a single ink are combined into one rectangle.
local K={}
local colors={
    {k="#193640",s="#985735",m="#e99945",l="#ffca70",h="#fff1bb",v="#315566",e="#d2ffff",r="#c35e42"},
    {k="#193640",s="#317c77",m="#62b9a9",l="#94e9c8",h="#ddffe2",v="#315566",e="#d2ffff",r="#de8653"},
}
local stencils={k="#34596579",s="#55797968",m="#e1edce7c",l="#fbf1d699",h="#fff3cd99",v="#42657577",e="#fff1cc99",r="#b3d8be7c"}
local stamps={
    { -- The lamp and broad brim stay readable from the other side of the room.
        ".....kkkkk.....","...kkllmmskk...", "..klllhmmmsmk..",
        "..klllhmmmsmk..",".klllllllllhhk.", "kkkkkkkkkkkkkkk",
        "..kmvvvvvvvvvk.","..ksvvveevveek.", "..kssvvvvvvvsk.",
        "...kkmmllllkk..","..kkrrrrrrsk...","..kskmmmmlmk...",
        ".ksskmlhmlmkk..",".ksskmhmmlkmsk.", "..kkkssmmmkssk.",
        "....kkksskkkk..",
    },
    { -- Twin posts, an oval dive mask, a side valve and a belt of brass.
        "...kk.....kk...","..klhk...klhk..", "...kk.kkk.kk...",
        "...kklhhhllk...","..klllmllllmk..", ".ksmkkkkkkkkmk.",
        ".kskvhvvvvvvkk.",".kskveevvveevk.", ".kskvvvvvvvvkk.",
        "..kmkkllllkkk..","...kkssssmkk...", "..ksklllmmmk...",
        ".klsklhlmmmk...",".kmkklllmmmksk.", "..kkkrrrrrrkmk.",
        "....kkksskkkk..",
    },
}
local prints={}
for i,rows in stamps do
    local runs={}
    for y,row in rows do
        local x=1
        while x<=#row do
            local c=row:sub(x,x)
            local last=x+1
            while last<=#row and row:sub(last,last)==c do last+=1 end
            if c~="." then table.insert(runs,{x-1,y-1,last-x,c}) end
            x=last
        end
    end
    prints[i]=runs
end

function K.draw(rect,x,feet,i,r,s,ghost)
    local pal=if ghost then stencils else colors[i]
    local dir=r.face or 1
    local stride=if r.on then math.sin(r.stride or 0)*math.min(1,(r.speed or 0)/4) else 0
    local tuck=if r.on then 0 elseif (r.vy or 0)>0 then 4 else 2
    local y=feet-67+math.abs(stride)*2+(r.impact or 0)*3
    local function localRect(dx,dy,w,h,c)
        rect(x+(if dir<0 then -dx-w else dx),dy,w,h,c)
    end
    if (r.spawnGlow or 0)>0 then
        local radius=12+(1-r.spawnGlow)*22
        for k=0,5 do
            local a=k*math.pi/3
            rect(x+radius*math.cos(a)-2,feet-29+radius*math.sin(a),4,4,"#fff0c9")
        end
    end
    if (r.impact or 0)>0.08 then
        local spread=(1-r.impact)*17
        for _,d in {-1,1} do rect(x+d*(12+spread),feet-4-spread*0.22,6,3,"#fff0c9") end
    end
    if s.owner==i and not ghost then
        local flap=if r.on then 2 else 3+3*math.sin(s.time*15)
        for _,d in {-1,1} do
            for n=0,2 do
                local fx,fy=x+d*(18+7*n)-5,feet-31-(n+1)*(5+flap)
                rect(fx-2,fy-2,12,16-2*n,pal.k)
                rect(fx,fy,8,10-2*n,if n==0 then pal.l else "#fff2cb")
            end
        end
    end
    for _,d in {-1,1} do
        local lift=math.max(0,d*stride)*5+tuck*(if d<0 then 0.5 else 1)
        local leg=d*7-4+stride*d*4+(if r.on then 0 else d*2)
        localRect(leg,feet-15-lift,9,12,pal.k)
        localRect(leg+2,feet-12-lift,5,7,pal.s)
        localRect(leg-1,feet-6-lift,14,6,pal.k)
        localRect(leg+3,feet-5-lift,8,2,pal.l)
    end
    for _,p in prints[i] do
        localRect((p[1]-7.5)*3.5,y+p[2]*3.5,p[3]*3.5,3.5,pal[p[4]])
    end
end

function K.demonstrate(rect,xy,l,s)
    local bx,by=xy(l.land[2][1]-0.62,l.land[1][3])
    local ex,ey=xy(l.land[2][1]+0.88,l.land[2][3])
    -- The painted pair steps aside as the players approach the demonstration.
    for _,r in s.crew do
        if r.x>l.land[2][1]-1.30 and r.x<l.land[2][2] then return end
    end
    local t=s.time%5.1
    local gx,gy=bx+53,by
    local on=true
    if t>=0.65 then
        local k=math.clamp((t-0.65)/0.78,0,1)
        gx,gy=bx+53*(1-k),by-67*k-59*4*k*(1-k)
        on=k>=1
    end
    if t>=2.2 then
        local k=math.clamp((t-2.2)/0.8,0,1)
        gx,gy=bx+(ex-bx)*k,by-67+(ey-by+67)*k-60*4*k*(1-k)
        on=k>=1
    end
    -- Brief blank beat between loops, after letting the landing read.
    if t<4.45 then
        K.draw(rect,bx,by,1,{face=1,on=true},s,true)
        K.draw(rect,gx,gy,2,{face=if t<1.45 then -1 else 1,on=on},s,true)
    end
end
return K
