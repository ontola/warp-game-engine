"""Play every authored room with two virtual controllers in the actual Warp VM.

Each waypoint is (seat, x, landing height, optional jump, optional second jump).
No player positions, flags, physics or level transitions are set by the test.
"""
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
# Walk, jump and double-jump waypoints. A pause on arrival lets a partner move.
def w(seat, x, y=0): return [seat, x, y, 0]
def j(seat, x, y=0): return [seat, x, y, 1]
def d(seat, x, y=0): return [seat, x, y, 2]

def boost(x, y, target, top):
    return [w(2,x,y), w(1,x-0.75,y), j(1,x,y+1.2), j(1,target,top)]

ROUTES = [
    boost(5.6,0,7.3,2.65)+[w(1,18),w(2,18)],
    [w(2,3.7),j(2,7.4,1.25),w(1,3.6),j(1,6.7,1.25),
     j(1,7.4,2.45),j(1,9,3.9),w(1,18),w(2,18)],
    [w(2,11.6),j(2,12.5,1.15),w(1,11.5),j(1,11.7,1.15),
     j(1,12.5,2.35),j(1,15.6,2.65)],
    [w(1,3.4),j(1,5,1.1),w(2,14.1),j(2,15,1),w(2,18),w(1,18)],
    boost(4.6,0,6,2.65)+[w(2,14.1),j(2,15,1.3),w(2,18),w(1,18)],
    [w(1,6.26),j(1,9.4),w(2,6.1),j(2,9.4),w(2,10.6),
     w(1,9.85),j(1,10.6,1.2),j(1,12,2.65),w(1,18),w(2,18)],
    boost(2.8,0,3.5,2.65)+[w(2,5.5),j(2,8),w(2,9.6),j(2,12),w(2,14),w(1,4.4,2.65),w(2,17),
     w(2,18),w(1,5.6),j(1,8),w(1,9.6),j(1,12),w(1,18)],
    [w(1,4),w(2,18),w(1,18)],
    [w(1,3.7),w(2,10.3),w(1,2.6),w(1,3.7),w(2,18),w(1,18)],
    boost(3.7,0,4.4,2.65)+[w(2,10),j(2,11.3,0.8),w(1,3.5,2.65),
     w(1,4.4,2.65),j(2,13,1),w(2,18),w(1,10),j(1,11.3,0.8),j(1,13,1),w(1,18)],
    boost(4.7,0,6,2.65)+[w(1,7.7,2.65),d(1,11,4.7),w(1,12,4.7),w(1,18),w(2,18)],
    [w(1,5.2),w(1,12,2.65),w(2,5.2),w(2,12.9,2.65),
     w(2,13.8,2.65),j(2,17),w(2,18),w(1,13.8,2.65),j(1,17.3)],
    boost(2.8,0,3.5,2.65)+[w(2,5.7),j(2,8.5),j(2,11),w(2,12.8),w(1,4.4,2.65),j(2,14.7),
     j(2,17.7),w(2,18.5),w(1,5.7),j(1,8.5),j(1,11),w(1,12.8),
     j(1,14.7),j(1,18.4)],
    boost(3.6,0,4.5,2.65)+[w(2,0.35),w(1,9.6),j(1,10.9,1),w(2,1.3),w(2,0.35),
     w(1,14,1),d(1,16,2.5),d(1,18,4.7),w(1,16.7,2.5),
     w(1,14.4,1),w(1,16.8),w(1,19),w(2,9.8),j(2,11,1),w(2,19)],
]

def literal(obj):
    if isinstance(obj, list):
        return "{" + ",".join(literal(item) for item in obj) + "}"
    return str(obj)

# This temporary hook drives the production tick. State observations only.
DRIVER = """
local playTick=tick
local testRoom,step,elapsed,stable=0,1,0,0
tick=function(dt)
    local plan=routes[state.room]
    if state.room~=testRoom then
        testRoom,step,elapsed,stable=state.room,1,0,0
        print("Testing "..rooms[testRoom].name)
    end
    local input={}
    for _,r in state.crew do input[r.id]={move_x=0,held={},pressed={}} end
    if state.phase=="play" then
        local a=assert(plan[step],"route ended before both players arrived")
        local r=state.crew[a[1]]
        local p=input[r.id]
        local delta=a[2]-r.x
        p.move_x=math.clamp(delta/(4.2*dt),-1,1)
        p.pressed.south=(elapsed==0 and a[4]>0) or (elapsed==22 and a[4]==2)
        if elapsed>0 and math.abs(delta)<0.015 and math.abs(r.y-a[3])<0.03 and r.on then
            stable+=1
        else stable=0 end
        if stable>=2 then step+=1 elapsed=0 stable=0 else elapsed+=1 end
        if elapsed>240 then
            error(string.format("room %d step %d: target %.2f,%.2f reached %.2f,%.2f, falls %d, past %s, open %s",
                testRoom,step,a[2],a[3],r.x,r.y,state.falls,tostring(state.past),tostring(state.open)))
        end
    end
    playTick(dt,input)
end
"""
with tempfile.TemporaryDirectory() as temp:
    game = Path(temp) / "game"
    shutil.copytree(ROOT,game,ignore=shutil.ignore_patterns("scripts"))
    src = (game / "main.luau").read_text()
    marker = "return {init=init,"
    assert src.count(marker) == 1
    (game/"main.luau").write_text(src.replace(marker,"local routes="+literal(ROUTES)+"\n"+DRIVER+"\n"+marker))
    run = subprocess.run([sys.argv[1],"check",str(game),"--ticks","16000"],capture_output=True,text=True)
    log=run.stdout+run.stderr
    for line in log.splitlines():
        if "Testing " in line or "hook" in line or "target " in line or "route ended" in line:
            print(line)
    snapshot=json.loads(run.stdout[run.stdout.index('{\n'):])
    state=snapshot["state"]
    assert not snapshot["errors"] and run.returncode==0, snapshot["errors"]
    assert state["phase"]=="complete" and state["room"]==len(ROUTES) and state["falls"]==0, state
    print(f"All {len(ROUTES)} rooms solved with two controllers, zero falls.")
