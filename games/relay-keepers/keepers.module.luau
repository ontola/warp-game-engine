-- Relief-print keepers. Rows of a single ink are combined into one rectangle.
local K={}
local colors={
    {k="#193640",s="#985735",m="#e99945",l="#ffca70",h="#fff1bb",v="#315566",e="#d2ffff",r="#c35e42"},
    {k="#193640",s="#317c77",m="#62b9a9",l="#94e9c8",h="#ddffe2",v="#315566",e="#d2ffff",r="#de8653"},
}
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

function K.draw(rect,x,feet,i,r,s)
    local pal=colors[i]
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
    if s.owner==i then
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

-- Simple printed figures on the instruction sign, with the two keepers' headgear.
local function signKeeper(rect,x,fy,seat)
    local ink,paint="#253f44",colors[seat].l
    rect(x-8,fy-21,16,11,ink)
    rect(x-6,fy-19,12,4,paint)
    rect(x-2,fy-16,8,3,"#fff5d9")
    if seat==1 then
        rect(x-5,fy-25,10,4,ink)
        rect(x-3,fy-24,6,3,paint)
        rect(x-10,fy-21,20,3,ink)
    else
        rect(x-7,fy-25,4,5,ink)
        rect(x+3,fy-25,4,5,ink)
    end
    rect(x-6,fy-10,12,7,ink)
    rect(x-4,fy-10,8,4,paint)
    rect(x-6,fy-4,4,4,ink)
    rect(x+2,fy-4,4,4,ink)
end

function K.demonstrate(rect,xy,l,s)
    local ledge=l.land[2]
    local rider=false
    for _,r in s.crew do
        if r.id==0 or r.y>=ledge[3]-0.08 then return end
        rider=rider or r.ride
    end
    if s.time<4 then return end

    -- An inset safety diagram leaves the actual floor and landing clear.
    local x=xy(ledge[1],0)-202
    local y=158
    local ink,board="#395354","#ebddbc"
    rect(x+3,y,158,112,ink)
    rect(x,y+3,164,106,ink)
    rect(x+6,y+5,152,101,board)
    rect(x+10,y+9,4,4,"#9eaa96")
    rect(x+150,y+9,4,4,"#9eaa96")
    local baseX,baseY=x+65,y+98
    local lipX,lipY=x+98,y+49
    rect(x+12,baseY,87,5,ink)
    rect(lipX,lipY,52,5,ink)
    -- Key painted over the landing.
    rect(x+132,lipY-18,10,10,ink)
    rect(x+135,lipY-15,4,4,board)
    rect(x+141,lipY-15,11,3,ink)
    rect(x+148,lipY-13,3,5,ink)

    local t=(s.time-4)%5
    if rider then t=2.05+(s.time%2.75) end
    local gx,gy=x+28,baseY
    if t>=0.55 then
        local k=math.clamp((t-0.55)/0.85,0,1)
        gx,gy=gx+(baseX-gx)*k,baseY-25*k-17*4*k*(1-k)
    end
    if t>=2.2 then
        local k=math.clamp((t-2.2)/0.85,0,1)
        gx,gy=baseX+(lipX+14-baseX)*k,baseY-25+(lipY-baseY+25)*k-16*4*k*(1-k)
    end
    signKeeper(rect,baseX,baseY,1)
    signKeeper(rect,gx,gy,2)
end
return K
