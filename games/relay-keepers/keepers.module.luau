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
    local vy=r.vy or 0
    local launch=if r.on then 0 else math.clamp(vy/10.5,0,1)
    local tuck=if r.on then 0 else 1-math.clamp(math.abs(vy)/7,0,1)
    local impact=if r.on then r.impact or 0 else 0
    -- Work up from the foot contact, leaving both boots planted during the squash.
    local pixelY=3.5*(1+0.045*launch-0.085*impact)
    local pixelX=3.5*(1-0.025*launch+0.045*impact)
    local y=feet-11-16*pixelY+math.abs(stride)*2
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
        local lift=math.max(0,d*stride)*5+tuck*(if d<0 then 8 else 10)
        if d>0 then lift+=launch*4 end
        local leg=d*7-4+stride*d*4+d*(2*launch+3*tuck)
        localRect(leg,feet-15-lift,9,12,pal.k)
        localRect(leg+2,feet-12-lift,5,7,pal.s)
        localRect(leg-1,feet-6-lift,14,6,pal.k)
        localRect(leg+3,feet-5-lift,8,2,pal.l)
    end
    for _,p in prints[i] do
        localRect((p[1]-7.5)*pixelX,y+p[2]*pixelY,p[3]*pixelX,pixelY,pal[p[4]])
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

    local t=(s.time-4)%3.65
    if rider then t=1.45+(s.time%2.15) end
    local launch=0.48
    local fromX,fromY,toX,toY=x+28,baseY,baseX,baseY-25
    if t>=1.68 then
        launch=1.68
        fromX,fromY,toX,toY=baseX,baseY-25,lipX+14,lipY
    end
    local elapsed=t-launch
    local rise=0.32
    local flight=0.50
    local apex=fromY-35
    local progress=math.clamp(elapsed/flight,0,1)
    local gx=fromX+(toX-fromX)*progress
    local gy=if elapsed<rise then apex+35*(1-math.clamp(elapsed/rise,0,1))^2
        else apex+(toY-apex)*math.clamp((elapsed-rise)/(flight-rise),0,1)^2

    -- The bottom face button stays depressed through the climb. Its ink trail
    -- fills while it is held and clears when the keeper reaches the landing.
    local px,py=x+15,y+13
    rect(px+3,py+2,28,13,ink)
    rect(px,py+8,8,12,ink)
    rect(px+26,py+8,8,12,ink)
    rect(px+5,py+4,24,9,board)
    rect(px+9,py+5,3,8,ink)
    rect(px+6,py+8,9,3,ink)
    rect(px+24,py+3,3,3,ink)
    rect(px+20,py+7,3,3,ink)
    rect(px+28,py+7,3,3,ink)
    local held=elapsed>=0 and elapsed<rise
    rect(px+23,py+(if held then 12 else 10),6,5,if held then colors[2].l else ink)
    if elapsed>=0 and elapsed<flight then
        local fill=math.clamp(elapsed/rise,0,1)
        rect(px+4,py+23,26,3,ink)
        rect(px+4,py+23,math.max(2,26*fill),3,colors[2].m)
    end
    signKeeper(rect,baseX,baseY,1)
    signKeeper(rect,gx,gy,2)
end
return K
