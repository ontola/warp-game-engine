local rooms = require("rooms")
local carriage = require("carriage")
local shells = require("shells")
local tidepool = require("tidepool")
local portals = require("portals")
local view = require("view")
local HEIGHT, SPEED, GRAVITY, JUMP = 1.20, 4.2, 18, 7.9

local function roster()
    local present = {}
    for _,p in players.list() do if not p.bot then present[p.id]=p end end
    for _,r in state.crew do
        if not present[r.id] then r.id=0 end
    end
    for _,p in players.list() do
        if not p.bot and state.crew[1].id~=p.id and state.crew[2].id~=p.id then
            for _,r in state.crew do
                if r.id==0 then r.id=p.id break end
            end
        end
    end
end

local function resetRunner(r,spawn)
    r.x,r.y,r.vy,r.on,r.face = spawn[1],spawn[2],0,true,1
    r.deck,r.springImpulse=false,false
    r.foldLock=0
    r.stride,r.speed,r.impact,r.spawnGlow=0,0,0,0
    r.buffer,r.coyote,r.extra,r.ride = 0,0,false,false
end

local function enter(n)
    local l=rooms[n]
    if state.review then
        state.review={held=false,cleared=false,finished=false,approve=false}
        state.falls=0
    end
    state.room,state.phase,state.delay=n,"play",0
    state.key,state.owner,state.latch,state.past=false,0,false,true
    state.clockHeld,state.pad,state.open,state.timer=false,false,false,0
    state.crate=l.crate and l.crate[1] or nil
    state.ferry=carriage.start(l.ferry)
    state.water=tidepool.start(l.water)
    state.crabs,state.hush=shells.start(l.crabs),false
    state.glass={}
    state.reboard=false
    state.link=0
    state.balance=nil
    for _,span in l.land do
        if span[4]=="balance" then state.balance=0 break end
    end
    for i,r in state.crew do resetRunner(r,l.start[i]) end
end
local function init()
    state.crew={{id=0},{id=0}}
    state.mapRoom=nil
    state.review=nil
    state.time,state.falls=0,0
    roster()
    enter(1)
end

-- The live console and the playtest desk share this small control surface.
-- Keeping the review in state preserves a paused scene across script reloads.
local function playtest(action,n)
    local review=state.review
    if action=="begin" then
        assert(type(n)=="number" and n%1==0 and n>=1 and n<=#rooms,"invalid playtest room")
        state.mapRoom=nil
        state.review={}
        enter(n)
    elseif action~="status" then
        assert(review,"begin a playtest first")
        if action=="pause" then
            review.held=true
        elseif action=="resume" then
            review.held=review.cleared or review.finished
        elseif action=="retry" then
            enter(state.room)
        elseif action=="next" then
            if state.room<#rooms then
                enter(state.room+1)
            elseif not review.finished then
                review.held,review.finished=true,true
                review.approve=false
                state.phase="complete"
                game.finish()
            end
        else
            error("unknown playtest action")
        end
    end
    review=state.review
    return {room=state.room,name=rooms[state.room].name,total=#rooms,active=review~=nil,
        held=review and review.held or false,cleared=review and review.cleared or false,
        finished=review and review.finished or false,approve=review and review.approve or false,
        falls=state.falls}
end

local function near(r,point,radius)
    return point and math.abs(r.x-point[1])<radius and math.abs(r.y-point[2])<0.34
end
local function material(l,i)
    local kind=l.land[i][4]
    if type(kind)=="number" then return state.link>=kind end
    if kind=="p" then return state.past or state.latch end
    if kind=="f" then return not state.past or state.latch end
    if kind=="glass" then return (state.glass[tostring(i)] or 0)<0.42 end
    if kind=="balance" then return (state.balance or 0)>0 end
    return true
end
local function gateLimit(x,desired,bar,y,height)
    if not bar or y>=height or y < -1 then return desired end
    if x<=bar then return math.min(desired,bar-0.32) end
    return math.max(desired,bar+0.32)
end

local function move(l,i,r,other,pad,dt)
    local dx=pad.move_x
    if pad.held.left then dx=-1 elseif pad.held.right then dx=1 end
    if math.abs(dx)<0.15 then dx=0 end
    if dx~=0 then r.face=math.sign(dx) end
    local x=math.clamp(r.x+dx*SPEED*dt+tidepool.drift(l.water,state.water,r,dt),0.3,19.7)
    if not state.open then x=gateLimit(r.x,x,l.gate,r.y,l.gateHeight or 6.9) end
    if not state.key then x=gateLimit(r.x,x,l.lock,r.y,6.9) end
    if state.crate and r.y<0.85 and r.y>-0.2 then
        local side=math.sign(state.crate-r.x)
        if side*dx>0 and math.abs(x-state.crate)<0.74 then
            local cx=math.clamp(x+side*0.74,0.55,(l.gate or 20)-0.86)
            state.crate=cx
            x=cx-side*0.74
        end
    end
    r.speed=math.abs(x-r.x)/dt
    if r.on then r.stride=(r.stride or 0)+math.abs(x-r.x)*4.3 end
    r.impact=math.max(0,(r.impact or 0)-dt*5)
    r.spawnGlow=math.max(0,(r.spawnGlow or 0)-dt*2)
    r.x=x
    r.buffer=if pad.pressed.south and not r.springImpulse then 0.13 else math.max(0,r.buffer-dt)
    r.springImpulse=false
    r.coyote=if r.on then 0.075 else math.max(0,r.coyote-dt)
    if r.buffer>0 and (r.coyote>0 or r.extra) then
        if r.coyote==0 then r.extra=false end
        r.vy,r.coyote,r.buffer,r.ride=JUMP,0,0,false
        sound.play("jump",{pitch=0.85+0.15*i,db=-9})
    end
    local prev=r.y
    r.vy-=GRAVITY*dt
    tidepool.float(l.water,state.water,r,x,dt,pad)
    r.y+=r.vy*dt
    local top,source=-100,""
    local function support(y,tag)
        if prev>=y-0.001 and r.y<=y and y>top then top,source=y,tag end
    end
    for j,b in l.land do
        if x>b[1]-0.16 and x<b[2]+0.16 and material(l,j) then support(b[3],j) end
    end
    local surface=tidepool.support(l.water,state.water,r,x,prev)
    if surface and surface>top then top,source=surface,"water" end
    local f=state.ferry
    local permanentBridge=type(source)=="number" and type(l.land[source][4])=="number"
    local contactY=if f and not r.deck then f.prevY else (f and f.y)
    if f and math.abs(x-f.x)<(l.ferry.width or 3)/2+0.10 and
            prev>=contactY-0.001 and r.y<=f.y and (f.y>top or (f.y==top and not permanentBridge)) then
        top,source=f.y,"deck"
    end
    if state.crate and math.abs(x-state.crate)<0.58 then support(0.9,"crate") end
    if math.abs(x-other.x)<0.48 then support(other.y+HEIGHT,"head") end
    if not r.on and source~="" and r.vy < -4 then
        r.impact=math.min(1,-r.vy/12)
        if source=="water" then sound.play("tide",{db=-13,pitch=1.2})
        elseif r.vy < -8 then sound.play("dock",{db=-19,pitch=1.7}) end
    end
    r.deck=source=="deck"
    r.on=source~=""
    r.ride=source=="head"
    if r.on then
        r.y,r.vy=top,0
        r.extra=state.owner==i
        if type(source)=="number" and l.land[source][4]=="glass" then
            local k=tostring(source)
            if not state.glass[k] then state.glass[k]=0 end
        end
        if near(r,l.spring,0.57) then
            r.vy,r.on=12.6,false
            sound.play("jump",{pitch=1.5,db=-6})
        end
    end
    if state.crabs then shells.touch(state,r,prev) end
    if near(r,l.key,0.50) and not state.key then
        state.key=true
        sound.play("pickup")
    end
    if near(r,l.wings,0.53) and state.owner==0 then
        state.owner,r.extra=i,true
        sound.play("pickup",{pitch=1.3})
    end
    if near(r,l.latch,0.55) and not state.latch then
        state.latch=true
        sound.play("pickup",{pitch=0.8})
    end
    if r.y < -2.2 then
        -- A stranded partner cannot dispatch a tandem deck alone.
        -- Reboard together, keeping every acquired power and lit relay.
        if l.ferry then
            state.reboard=true
        else
            resetRunner(r,l.start[i])
            r.spawnGlow=1
        end
        state.falls+=1
        players.rumble(r.id,0.45,0.15)
    end
end

-- Either player can browse. Releasing the stick resets the repeat delay.
local function mapInput(inputs,dt)
    for _,runner in state.crew do
        local p=inputs[runner.id]
        if p.pressed.north then
            state.mapRoom=if state.mapRoom then nil else state.room
            state.mapRepeat=0
            return true
        end
    end
    if not state.mapRoom then return false end
    local dx,dy=0,0
    for _,runner in state.crew do
        local p=inputs[runner.id]
        if p.pressed.right_shoulder or p.pressed.left_shoulder then
            local offset=if p.pressed.right_shoulder then 56 else -56
            state.mapRoom=math.clamp(state.mapRoom+offset,1,#rooms)
            return true
        end
        if p.pressed.south then
            local selection=state.mapRoom
            state.mapRoom=nil
            enter(selection)
            return true
        end
        if p.held.left or p.pressed.left or p.move_x < -0.55 then dx=-1
        elseif p.held.right or p.pressed.right or p.move_x > 0.55 then dx=1
        elseif p.held.up or p.pressed.up or (p.move_y or 0)>0.55 then dy=-1
        elseif p.held.down or p.pressed.down or (p.move_y or 0)<-0.55 then dy=1 end
    end
    if dx==0 and dy==0 then
        state.mapRepeat=0
    elseif state.mapRepeat<=0 then
        local col=(state.mapRoom-1)%7
        local row=math.floor((state.mapRoom-1)/7)
        col=math.clamp(col+dx,0,6)
        row=math.clamp(row+dy,0,math.ceil(#rooms/7)-1)
        state.mapRoom=math.min(#rooms,row*7+col+1)
        state.mapRepeat=0.20
    else
        state.mapRepeat-=dt
    end
    return true
end

local function tick(dt,inputs)
    if state.crew[1].id==0 or state.crew[2].id==0 then return end
    if state.review then
        for _,r in state.crew do
            if inputs[r.id].pressed.west then state.review.held=true end
            if state.review.cleared and not state.review.finished and inputs[r.id].pressed.right_shoulder then
                state.review.approve=true
            end
        end
        if state.review.held then return end
    elseif mapInput(inputs,dt) then return end
    state.time+=dt
    if state.phase~="play" then
        state.delay+=dt
        if state.phase=="transfer" and state.delay>=1 then enter(state.room+1)
        elseif state.phase=="complete" and state.delay>1 then
            for _,r in state.crew do if inputs[r.id].pressed.south then init() break end end
        end
        return
    end
    local l=rooms[state.room]
    for k,age in state.glass do state.glass[k]=if age+dt<2.6 then age+dt else nil end
    if state.crabs then shells.step(l,state,dt) end
    if l.water then tidepool.step(l.water,state.water,state.crew,dt) end
    if state.ferry then carriage.step(l.ferry,state.ferry,state.crew,inputs,dt) end
    -- Snapshot the supporting bodies so both see the same contact surface.
    local before={}
    for i,r in state.crew do before[i]={x=r.x,y=r.y} end
    for i,r in state.crew do
        move(l,i,r,before[3-i],inputs[r.id],dt)
    end
    if state.reboard then
        state.reboard=false
        state.ferry=carriage.start(l.ferry)
        for i,r in state.crew do
            resetRunner(r,l.start[i])
            r.spawnGlow=1
        end
        return
    end
    -- Carry a rider by the actual displacement of their supporting player.
    for i,r in state.crew do
        local base,old=state.crew[3-i],before[3-i]
        if r.ride and math.abs(base.x-old.x)<=SPEED*dt+0.001 and math.abs(base.y-old.y)<0.5 then
            local x=math.clamp(r.x+base.x-old.x,0.3,19.7)
            if not state.open then x=gateLimit(r.x,x,l.gate,r.y,l.gateHeight or 6.9) end
            if not state.key then x=gateLimit(r.x,x,l.lock,r.y,6.9) end
            r.x=x
            if math.abs(x-base.x)<0.48 then r.y=base.y+HEIGHT end
        end
    end
    portals.step(l,state)
    -- A stacked pair lights the counterweights. A short coast lets the rider dismount.
    if state.balance~=nil then
        local previous=state.balance
        state.balance=if state.crew[1].ride or state.crew[2].ride then 0.50 else math.max(0,previous-dt)
        if previous==0 and state.balance>0 then sound.play("counterweight",{db=-8}) end
    end
    local held=l.pad and state.crate and math.abs(state.crate-l.pad[1])<0.64 or false
    local clock,home=false,true
    local paired=l.secondGoal and ((near(state.crew[1],l.secondGoal,0.85) and near(state.crew[2],l.goal,0.85)) or
        (near(state.crew[2],l.secondGoal,0.85) and near(state.crew[1],l.goal,0.85)))
    for _,r in state.crew do
        held=held or (r.on and near(r,l.pad,0.56)) or false
        clock=clock or (r.on and near(r,l.clock,0.53)) or false
        home=home and r.on and (paired or near(r,l.goal,0.85))
    end
    if clock and not state.clockHeld then
        state.past=not state.past
        sound.play("pickup",{pitch=if state.past then 0.65 else 1.35})
    end
    if held and not state.pad then sound.play("pickup",{pitch=0.55,db=-10}) end
    state.clockHeld,state.pad=clock,held
    state.timer=if held then l.timer or 0 else math.max(0,state.timer-dt)
    state.open=if l.timer then state.latch or (not held and state.timer>0)
        elseif l.pad then held or state.latch else state.key
    local nextLink=l.links and l.links[state.link+1]
    if nextLink then
        local a,b=state.crew[1],state.crew[2]
        local paired=(near(a,nextLink[1],0.62) and near(b,nextLink[2],0.62))
            or (near(b,nextLink[1],0.62) and near(a,nextLink[2],0.62))
        if a.on and b.on and paired then
            state.link+=1
            sound.play("pickup",{pitch=0.7+state.link*0.16})
        end
    end
    if home and (not l.links or state.link==#l.links) and (not l.secondGoal or paired) and (not l.key or state.key) and (not l.latch or state.latch) then
        state.phase=if state.room==#rooms then "complete" else "transfer"
        game.log(l.name)
        sound.play("pickup",{pitch=1.4})
        if state.review then
            state.review.held,state.review.cleared=true,true
        elseif state.phase=="complete" then
            game.finish()
        end
    end
end
return {init=init,tick=tick,players_changed=roster,debug={playtest=playtest},draw=function()
    view.draw(rooms,state)
end}
