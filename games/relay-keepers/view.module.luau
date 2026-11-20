local coast=require("coast")
local keepers=require("keepers")
local decks=require("decks")
local shells=require("shells")
local tidepool=require("tidepool")
local portals=require("portals")
local atlasmap=require("atlasmap")
local ink,cream="#18363e","#fff0c9"
local amber,mint="#ffbc66","#75e3ce"
local scale,left,top
local function box(x,y,w,h,c)
    hud.box(left+x*scale,top+y*scale,w*scale,h*scale,c)
end
local function xy(x,y) return 40+56*x,520-56*y end
local function gridIcon(x,y)
    for row=0,1 do
        for col=0,2 do box(x+col*7,y+row*7,5,5,cream) end
    end
end

local function draw(rooms,s)
    local l,total=rooms[s.room],#rooms
    local w,h=hud.size()
    scale=math.min(w/1200,h/640)
    left,top=(w-1200*scale)/2,(h-640*scale)/2
    hud.box(0,0,w,h,ink)
    if s.mapRoom then atlasmap.draw(box,rooms,s) return end
    coast.draw(box,l,s)
    tidepool.draw(box,xy,l,s)
    box(1128,20,48,26,ink)
    box(1135,24,4,4,if s.review then "#64817e" else amber)
    box(1130,29,4,4,if s.review then amber else "#64817e")
    box(1140,29,4,4,"#64817e")
    box(1135,34,4,4,"#64817e")
    if s.review then
        local c=if s.review.held then amber else cream
        box(1150,26,20,11,c)
        box(1153,36,5,4,c)
        box(1154,30,3,3,ink)
        box(1162,30,3,3,ink)
    else
        gridIcon(1151,27)
    end
    decks.draw(box,xy,l,s)
    if s.ferry then
        local f,spec=s.ferry,l.ferry
        local ax,ay=xy(unpack(spec.from))
        local bx,by=xy(unpack(spec.to))
        for i=0,24 do
            local k=i/24
            box(ax+(bx-ax)*k-2,ay+(by-ay)*k+12,4,4,"#738988")
        end
        for _,p in {spec.from,spec.to} do
            local x,y=xy(unpack(p))
            box(x-6,y+9,12,12,ink)
            box(x-3,y+12,6,6,cream)
        end
        local x,y=xy(f.x,f.y)
        local span=(spec.width or 3)*56
        box(x-span/2,y,span,5,if f.flash>0 then cream else amber)
        box(x-span/2+3,y+5,span-6,15,ink)
        box(x-span/2+8,y+8,span-16,4,"#658586")
        box(x-span/2+12,y+20,span-24,5,"#345863")
        for _,side in {-1,1} do
            local wx=x+side*(span*0.32)
            box(wx-7,y+21,14,6,ink)
            box(wx-3,y+22,6,4,amber)
        end
        for i,r in s.crew do
            local aboard=r.deck or (r.ride and s.crew[3-i].deck)
            local lamp=if i==1 then amber else mint
            box(x-17+18*(i-1),y+12,13,6,if aboard then lamp else "#5d7679")
        end
        if spec.coil then
            for _,d in {-1,1} do
                for j=0,2 do box(x+d*span*0.32-10+j*2,y+9+j*4,20-j*4,2,cream) end
            end
            if f.lit>0 or f.taps[1]>0 or f.taps[2]>0 then
                -- Two south-button symbols breathe on one beat before takeoff.
                local beat=math.max(0,math.cos(s.time*5))
                for i=1,2 do
                    local gx=x+(if i==1 then -33 else 33)
                    local gy=y-104-beat*4
                    local color=if i==1 then amber else mint
                    box(gx-9,gy-8,19,18,ink)
                    box(gx-2,gy-6,4,4,"#58716e")
                    box(gx-6,gy-1,4,4,"#58716e")
                    box(gx+3,gy-1,4,4,"#58716e")
                    box(gx-2,gy+4,5,5,if f.taps[i]>0 or beat>0.8 then cream else color)
                end
            end
            if f.flash>0 then
                local t=1-f.flash/0.45
                for j=-3,3 do box(x+j*(9+14*t),y-10-65*t+3*j*j,5,7,cream) end
            end
        end
        if spec.control then
            local cx,cy=xy(unpack(spec.control))
            local lever=if f.remote then 7 else -7
            box(cx-20,cy-5,40,5,ink)
            box(cx+lever-4,cy-27,8,10,if f.remote then mint else amber)
            box(cx-4,cy-23,8,19,ink)
            box(cx-11,cy-26,22,8,if f.remote then mint else amber)
        end
    end
    local function at(p) return xy(p[1],p[2]) end
    local function portal(p)
        local x,y=at(p)
        box(x-29,y-85,58,7,ink)
        box(x-36,y-79,72,7,ink)
        box(x-32,y-77,64,3,"#7da4a0")
        for _,d in {-1,1} do
            local upright=x+d*36
            box(upright-4,y-73,8,73,ink)
            box(upright-2,y-66,3,38,"#508280")
            box(upright-8,y-8,16,8,ink)
            local r=s.crew[if d<0 then 1 else 2]
            local lit=r.on and math.abs(r.x-p[1])<0.85 and math.abs(r.y-p[2])<0.34
            local lamp=if d<0 then amber else mint
            box(x+d*13-8,y-93,16,11,ink)
            box(x+d*13-5,y-92,10,8,if lit then cream else lamp)
            if lit then
                box(x+d*13-3,y-98,6,3,cream)
                box(upright-2,y-65,4,50,lamp)
            end
        end
    end
    local link=l.links and l.links[s.link+1]
    if link then
        for _,p in link do
            local x,y=at(p)
            local lit=false
            for _,r in s.crew do
                if r.on and math.abs(r.x-p[1])<0.62 and math.abs(r.y-p[2])<0.34 then lit=true end
            end
            box(x-23,y-6,46,6,ink)
            box(x-17,y-8,34,4,if lit then cream else mint)
            local pulse=math.sin(s.time*3)*2
            for _,side in {-1,1} do
                box(x+side*9-5,y-26+pulse,10,9,ink)
                box(x+side*9-3,y-24+pulse,6,5,if lit then cream else mint)
            end
            box(x-5,y-23+pulse,10,3,mint)
        end
    end
    shells.draw(box,xy,l,s)
    portals.draw(box,xy,l,s)
    portal(l.goal)
    if l.secondGoal then portal(l.secondGoal) end
    local function door(bar,opened,height)
        if not bar then return end
        local x,y=xy(bar,0)
        local high=56*(height or 6.9)
        box(x-13,y-high,26,7,ink)
        box(x-10,y-high+7,20,11,ink)
        box(x-7,y-high+3,14,4,if opened then mint else amber)
        if not opened then
            box(x-5,y-high+17,10,high-17,ink)
            box(x-3,y-high+18,3,high-18,"#537576")
            for j=16,high-20,28 do
                box(x-8,y-j,16,9,ink)
                box(x-6,y-j+2,12,4,"#e1a15f")
            end
            box(x-12,y-9,24,9,ink)
        else box(x-8,y-high+11,16,6,mint) end
    end
    door(l.gate,s.open,l.gateHeight)
    door(l.lock,s.key)
    if s.room==1 and not s.key then
        keepers.demonstrate(box,xy,l,s)
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
        box(x-16,y-8,17,17,ink)
        box(x-19,y-5,23,11,ink)
        box(x-14,y-6,13,13,amber)
        box(x-16,y-3,17,7,amber)
        box(x-11,y-2,7,5,ink)
        box(x-14,y-5,6,3,cream)
        box(x+1,y-4,20,8,ink)
        box(x+10,y+3,5,5,ink)
        box(x+16,y+2,6,8,ink)
        box(x,y-3,20,4,amber)
        box(x+17,y,4,6,amber)
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
        box(x-22,y-46,44,42,"#ad7653")
        for j=1,3 do box(x-22+j*11,y-45,2,39,"#825940") end
        for j=0,6 do box(x-20+j*6,y-12-j*5,8,5,amber) end
        for _,height in {-45,-9} do
            box(x-23,y+height,46,4,"#e2ac70")
            for _,d in {-1,1} do box(x+18*d-1,y+height,3,3,ink) end
        end
    end
    for i,r in s.crew do
        if r.id~=0 then local x,y=xy(r.x,r.y) keepers.draw(box,x,y,i,r,s) end
    end
    -- The trail of lights is also the level-complete transition.
    local first=s.room<=14 and 1 or 15+7*math.floor((s.room-15)/7)
    local count=math.min(if first==1 then 14 else 7,total-first+1)
    local function progress(y)
        for n=1,count do
            local i=first+n-1
            local x=600+(n-(count+1)/2)*19
            box(x-5,y,10,5,if i<s.room then mint elseif i==s.room then cream else "#668982")
        end
    end
    progress(30)
    if s.review and s.review.held then
        box(570,48,60,28,ink)
        box(584,54,7,16,amber)
        box(597,54,7,16,amber)
        if s.review.cleared then
            box(612,54,5,16,mint)
            if not s.review.finished then
                local c=if s.review.approve then mint else cream
                box(634,48,39,28,ink)
                box(640,59,13,10,"#526e71")
                box(654,57,13,12,"#526e71")
                box(655,53,11,4,c)
            end
        end
    elseif s.phase~="play" then
        box(455,95,290,71,ink)
        progress(124)
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
