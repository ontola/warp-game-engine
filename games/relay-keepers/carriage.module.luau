local M = {}

function M.start(spec)
    if not spec then return nil end
    local dx=spec.to[1]-spec.from[1]
    local dy=spec.to[2]-spec.from[2]
    return {x=spec.from[1],y=spec.from[2],length=math.sqrt(dx*dx+dy*dy),
        prevY=spec.from[2],away=false,t=0,direction=0,wait=0,lit=0,flash=0,taps={-1,-1}}
end

function M.step(spec,f,crew,input,dt)
    local count=0
    local remote=false
    for i,r in crew do
        if r.deck or (r.ride and crew[3-i].deck) then count+=1 end
        if spec.control and r.on and math.abs(r.x-spec.control[1])<0.55
                and math.abs(r.y-spec.control[2])<0.24 then remote=true end
        f.taps[i]=math.max(-1,f.taps[i]-dt)
        if spec.coil and r.deck and input[r.id].pressed.south then f.taps[i]=0.23 end
    end
    f.lit=count
    f.remote=remote
    if spec.control then
        f.direction=if remote then 1 else -1
    elseif f.direction==0 then
        if f.t==0 then
            f.wait=if count==2 or (f.away and count==1) then f.wait+dt else 0
            local boardingTime=if count==2 then 0.40 else 1.2
            if f.wait>=boardingTime then f.direction=1 end
        else
            f.wait+=dt
            if f.wait>=4 or (count==0 and f.wait>=0.7) then f.direction=-1 end
        end
    end
    local nextT=math.clamp(f.t+f.direction*(spec.speed or 3)*dt/f.length,0,1)
    local x=spec.from[1]+(spec.to[1]-spec.from[1])*nextT
    local y=spec.from[2]+(spec.to[2]-spec.from[2])*nextT
    local dx,dy=x-f.x,y-f.y
    for i,r in crew do
        if r.deck or (r.ride and crew[3-i].deck) then
            r.x=math.clamp(r.x+dx,0.3,19.7)
            r.y+=dy
        end
    end
    if nextT~=f.t and (nextT==0 or nextT==1) then
        f.direction,f.wait=0,0
        if nextT==1 then f.away=true end
        sound.play("dock",{db=-8,pitch=if nextT==0 then 0.85 else 1.12})
    end
    f.prevY=f.y
    f.x,f.y,f.t=x,y,nextT
    f.flash=math.max(0,f.flash-dt)
    if f.taps[1]>0 and f.taps[2]>0 then
        for i,r in crew do
            r.vy,r.on,r.coyote,r.buffer,r.deck,r.ride=12.8,false,0,0,false,false
            r.springImpulse=true
            f.taps[i]=-1
            players.rumble(r.id,0.35,0.13)
        end
        f.flash=0.45
        sound.play("launch",{db=-5})
    end
end
return M
