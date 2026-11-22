-- Velocities are tiles per second. The three automatic launches keep their
-- authored crests even when neither player is holding the jump button.
local J={normal=10.35,spring=16.46,coil=16.72,shell=14.25}
local RISE,FALL,WATER=30.5,38,18
local RELEASE_SPEED,TERMINAL=5.5,18

function J.launch(r,speed,controlled)
    r.vy,r.cuttable=speed,controlled
    r.on,r.deck,r.ride=false,false,false
    r.coyote,r.buffer=0,0
end

function J.step(r,dt,held,firstTick,inWater)
    -- Even a tap buffered before touchdown gets one frame of full takeoff.
    if r.cuttable and not held and not firstTick then
        r.vy=math.min(r.vy,RELEASE_SPEED)
        r.cuttable=false
    end
    local gravity=if inWater then WATER elseif r.vy>0 then RISE else FALL
    r.vy=math.max(-TERMINAL,r.vy-gravity*dt)
end

return J
