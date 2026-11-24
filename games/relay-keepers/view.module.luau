local coast=require("coast")
local keepers=require("keepers")
local decks=require("decks")
local shells=require("shells")
local tidepool=require("tidepool")
local portals=require("portals")
local atlasmap=require("atlasmap")
local prompts=require("hudprompts")
local instruments=require("instruments")
local doors=require("doors")
local ink,cream="#18363e","#fff0c9"
local amber,mint="#ffbc66","#75e3ce"
local scale,left,top
local cover,coverLeft,coverTop
local function box(x,y,w,h,c)
    hud.box(left+x*scale,top+y*scale,w*scale,h*scale,c)
end
-- The backdrop covers the whole screen (no letterbox bars); play stays fully visible.
local function backdrop(x,y,w,h,c)
    hud.box(coverLeft+x*cover,coverTop+y*cover,w*cover,h*cover,c)
end
local function xy(x,y) return 40+56*x,520-56*y end
local function draw(rooms,s)
    local l,total=rooms[s.room],#rooms
    local w,h=hud.size()
    scale=math.min(w/1200,h/640)
    left,top=(w-1200*scale)/2,(h-640*scale)/2
    cover=math.max(w/1200,h/640)
    coverLeft,coverTop=(w-1200*cover)/2,(h-640*cover)/2
    hud.box(0,0,w,h,ink)
    if s.mapRoom then atlasmap.draw(box,rooms,s) return end
    coast.draw(backdrop,l,s)
    tidepool.draw(box,xy,l,s)
    if s.crew[1].id~=0 and s.crew[2].id~=0 and (s.phase=="play" or s.review) then
        prompts.corner(box,s)
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
    shells.draw(box,xy,l,s)
    portals.draw(box,xy,l,s)
    doors.finish(box,xy,l.goal,l.secondGoal~=nil,s)
    if l.secondGoal then doors.finish(box,xy,l.secondGoal,true,s) end
    doors.barrier(box,xy,l.gate,s.open,l.gateHeight,l.pad~=nil)
    doors.barrier(box,xy,l.lock,s.key,nil,false)
    if s.room==1 and not s.key then
        keepers.demonstrate(box,xy,l,s)
    end
    instruments.draw(box,xy,l,s)
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
