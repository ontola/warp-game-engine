local ink,cream="#18363e","#fff0c9"
local amber,mint="#ffbc66","#75e3ce"
local scale,left,top
local function box(x,y,w,h,c)
    hud.box(left+x*scale,top+y*scale,w*scale,h*scale,c)
end
local function xy(x,y) return 40+56*x,520-56*y end
local function landscape(l,s)
    local night=l.clock and not s.past
    local sky=if night then "#394a69" else "#9fc6bc"
    local far=if night then "#505b78" else "#85afa6"
    local ridge=if night then "#586981" else "#729995"
    box(0,0,1200,640,sky)
    local sunX=if night then 150 else 956
    local sunColor=if night then "#dde5d0" else "#f5da9a"
    for j,w in {28,48,60,66,66,60,48,28} do box(sunX-w/2,85+j*8,w,8,sunColor) end
    if night then
        box(sunX+1,90,34,49,sky)
        for _,p in {{270,120},{915,109},{1110,161},{650,72},{407,185},{784,141}} do box(p[1],p[2],4,4,cream) end
    else
        for _,p in {{52,160,134},{430,120,89},{735,166,103},{1010,135,64}} do
            box(p[1],p[2],p[3],5,"#c5d9c3")
            box(p[1]+16,p[2]-5,p[3]*0.54,6,"#c5d9c3")
        end
    end
    for _,p in {{0,344,112},{107,322,76},{171,353,143},{383,300,72},{447,325,110},{673,315,84},{730,274,37},{808,348,170},{992,303,80},{1054,329,146}} do
        box(p[1],p[2],p[3],530-p[2],far)
        box(p[1]+6,p[2]-6,p[3]-12,7,far)
        if night then
            for j=1,3 do box(p[1]+8*j,p[2]+21,3,9,"#a4a492") end
        end
    end
    for _,x in {75,405,817} do
        box(x,224,7,277,ridge)
        box(x-42,222,97,5,ridge)
        box(x-39,215,42,3,ridge)
        box(x+48,226,2,33,ridge)
    end
    box(0,475,1200,165,if night then "#435e70" else "#6c9893")
    box(0,558,1200,82,if night then "#344b61" else "#507d7c")
    for j=1,12 do
        local x=(j*143+s.time*5)%1240-45
        box(x,570+((j*13)%43),26+(j%4)*12,3,ridge)
    end
end

local function robot(x,y,i,r,s)
    local c=if i==1 then amber else mint
    local height=63
    if s.owner==i then
        for _,d in {-1,1} do
            box(x+d*20-5,y-43,10,20,cream)
            box(x+d*26-4,y-49,8,13,cream)
        end
    end
    box(x-15,y-height+8,30,32,ink)
    box(x-12,y-height+10,24,27,c)
    if i==1 then
        box(x-12,y-height,24,8,c)
        box(x-18,y-height+6,36,6,ink)
    else
        box(x-11,y-height-3,4,12,ink)
        box(x+7,y-height-3,4,12,ink)
        box(x-14,y-height-6,9,6,c)
        box(x+5,y-height-6,9,6,c)
    end
    box(x-10,y-48,20,10,ink)
    box(x+3*r.face-5,y-46,3,4,cream)
    box(x+3*r.face+2,y-46,3,4,cream)
    box(x-12,y-26,24,17,ink)
    box(x-9,y-25,18,10,c)
    box(x-13,y-10,9,10,ink)
    box(x+4,y-10,9,10,ink)
end

local function draw(l,total,s)
    local w,h=hud.size()
    scale=math.min(w/1200,h/640)
    left,top=(w-1200*scale)/2,(h-640*scale)/2
    hud.box(0,0,w,h,ink)
    landscape(l,s)
    for i,b in l.land do
        local x,y=xy(b[1],b[3])
        local width=(b[2]-b[1])*56
        local kind=b[4]
        local temporal=kind=="p" or kind=="f"
        local active=not temporal or s.latch or (s.past==(kind=="p"))
        local age=s.glass[tostring(i)]
        if age and age>=0.42 then active=false end
        local color=if kind=="p" then amber elseif kind=="f" then "#b7bef7" elseif kind=="glass" then "#a0e3e2" else "#e5cfaa"
        if active then
            box(x,y,width,8,color)
            box(x,y+8,width,14,ink)
            box(x+4,y+13,width-8,3,"#527273")
            if kind=="glass" then
                for j=7,width-6,18 do box(x+j,y,3,7,ink) end
            elseif not temporal then
                for j=14,width-10,122 do
                    box(x+j,y+22,8,if b[3]==0 then 56 else 22,"#456867")
                    box(x+j-4,y+22,16,5,ink)
                end
            end
        else
            for j=2,width-6,18 do box(x+j,y+3,8,3,color) end
        end
    end
    local function at(p) return xy(p[1],p[2]) end
    local function portal(p)
        local x,y=at(p)
        box(x-40,y-82,80,8,ink)
        for _,d in {-1,1} do
            box(x+d*37-3,y-74,6,74,ink)
            local r=s.crew[if d<0 then 1 else 2]
            local lit=r.on and math.abs(r.x-p[1])<0.85 and math.abs(r.y-p[2])<0.34
            box(x+d*11-5,y-89,10,10,ink)
            box(x+d*11-3,y-87,6,6,if lit then cream elseif d<0 then amber else mint)
        end
    end
    portal(l.goal)
    if l.secondGoal then portal(l.secondGoal) end
    local function door(bar,opened,height)
        if not bar then return end
        local x,y=xy(bar,0)
        local high=56*(height or 6.9)
        box(x-13,y-high,26,11,ink)
        if not opened then
            box(x-7,y-high+11,14,high-11,ink)
            for j=20,high-3,21 do box(x-5,y-j,10,7,amber) end
        else box(x-8,y-high+11,16,6,mint) end
    end
    door(l.gate,s.open,l.gateHeight)
    door(l.lock,s.key)
    if s.room==1 and not s.key then
        -- A looping stencil painted on the first quay demonstrates two hops.
        local t=(s.time%4)/4
        local baseX,baseY=352,510
        local ghost="#aec4af"
        box(baseX-9,baseY-38,18,38,ghost)
        local k=math.clamp(t*2,0,1)
        local gx,gy=baseX-45+45*k,baseY-38*k-30*math.sin(math.pi*k)
        if t>0.5 then
            k=(t-0.5)*2
            gx,gy=baseX+46*k,baseY-38-48*k-29*math.sin(math.pi*k)
        end
        box(gx-9,gy-36,18,29,ghost)
        box(gx-9,gy-7,6,7,ghost)
        box(gx+3,gy-7,6,7,ghost)
    end
    if l.pad then
        local x,y=at(l.pad)
        box(x-28,y-6,56,6,ink)
        box(x-23,y-(if s.pad then 6 else 11),46,5,if s.pad then mint else amber)
        if l.timer then
            box(x-13,y+12,26,8,ink)
            box(x-11,y+14,22*math.max(0.06,s.timer/l.timer),4,amber)
        end
    end
    if l.latch then
        local x,y=at(l.latch)
        box(x-2,y-30,4,30,ink)
        box(x+2,y-30,17,11,if s.latch then mint else amber)
    end
    if l.clock then
        local x,y=at(l.clock)
        box(x-28,y-5,56,5,ink)
        box(x-12,y-29,24,23,ink)
        box(x-9,y-26,18,17,if s.past then amber else "#b7bef7")
        box(x-1,y-24,3,8,ink)
        box(x+(if s.past then -6 else 0),y-18,7,3,ink)
    end
    if l.spring then
        local x,y=at(l.spring)
        for j=0,2 do box(x-20+j*3,y-5-j*5,40-j*6,3,amber) end
        box(x-23,y-18,46,5,ink)
    end
    if l.key and not s.key then
        local x,y=at(l.key)
        y-=31+math.sin(s.time*4)*3
        box(x-15,y-8,15,15,ink)
        box(x-12,y-5,9,9,amber)
        box(x-9,y-2,3,3,ink)
        box(x,y-3,19,5,amber)
        box(x+12,y+1,5,7,amber)
    end
    if l.wings and s.owner==0 then
        local x,y=at(l.wings)
        y-=28+math.sin(s.time*4)*3
        for _,d in {-1,1} do
            box(x+d*7-5,y-2,10,14,ink)
            box(x+d*12-5,y-9,10,16,cream)
            box(x+d*17-3,y-13,6,11,cream)
        end
    end
    if s.crate then
        local x,y=xy(s.crate,0)
        box(x-26,y-50,52,50,ink)
        box(x-22,y-46,44,42,"#bb8460")
        for j=0,6 do box(x-20+j*6,y-12-j*5,8,4,amber) end
    end
    for i,r in s.crew do
        if r.id~=0 then local x,y=xy(r.x,r.y) robot(x,y,i,r,s) end
    end
    -- The trail of lights is also the level-complete transition.
    for i=1,total do
        local x=600+(i-(total+1)/2)*19
        box(x-5,30,10,5,if i<s.room then mint elseif i==s.room then cream else "#668982")
    end
    if s.phase~="play" then
        box(475,95,250,71,ink)
        for i=1,total do box(486+i*15,124,10,10,if i<=s.room then mint else "#507170") end
        if s.phase=="complete" then
            box(585,176,30,30,ink)
            box(590,181,20,20,mint)
            box(596,184,7,14,ink)
        end
    end
    if s.crew[1].id==0 or s.crew[2].id==0 then
        box(487,195,226,95,ink)
        for i,r in s.crew do
            local x=535+100*(i-1)
            box(x-23,224,46,30,if r.id~=0 then mint else "#68817c")
            box(x-14,236,11,4,ink)
            box(x-11,232,4,12,ink)
            box(x+9,234,5,5,ink)
        end
    end
end
return {draw=draw}
