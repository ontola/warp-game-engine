"""Regression tests for cooperation and recovery in the real Warp tick.

Usage: python scripts/test_mechanics.py /path/to/warp
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
from typing import NamedTuple

GAME = Path(__file__).resolve().parents[1]


class Sample(NamedTuple):
    time: float
    falls: float
    ax: float
    ay: float
    bx: float
    by: float
    a_on: float
    b_on: float
    riding: float
    crab_x: float
    hush: float
    crate: float
    water: float


# Each fixture has one unreachable exit. Only the tick's controller inputs change.
# An optional target X releases the stick on arrival, using the same inputs as a player.
# Actions are {seat, start seconds, end seconds, horizontal axis, button, target X}.
DRIVER = """
local elapsed=0
local function observedTick(dt)
    local input={}
    for _,r in state.crew do input[r.id]={move_x=0,held={},pressed={}} end
    for _,a in actions do
        if elapsed>=a[2] and elapsed<a[3] then
            local r=state.crew[a[1]]
            local p=input[r.id]
            p.move_x=a[4]
            if a[6] and (a[6]-r.x)*a[4]<=0 then p.move_x=0 end
            if a[5]=="jump" or a[5]=="tap" then
                p.held.south=a[5]=="jump"
                p.pressed.south=elapsed-dt<a[2]
            end
            if a[5]=="dive" then p.held.down=true end
        end
    end
    tick(dt,input)
    elapsed+=dt
    local a,b=state.crew[1],state.crew[2]
    print(string.format("sample %.6f %d %.5f %.5f %.5f %.5f %d %d %d %.5f %d %.5f %.5f",
        elapsed,state.falls,a.x,a.y,b.x,b.y,a.on and 1 or 0,b.on and 1 or 0,
        a.ride and 1 or 0,state.crabs and state.crabs[1].x or 0,state.hush and 1 or 0,
        state.crate or 0,state.water and state.water.y or 0))
end
"""


def simulate(warp, game, main, room, actions, seconds):
    (game / "rooms.module.luau").write_text(
        'return {{name="Fixture",goal={19,8},' + room + '}}\n', encoding="utf-8")
    entry = "return {init=init,tick=tick,"
    if main.count(entry) != 1:
        raise AssertionError("Could not wrap the production tick")
    (game / "main.luau").write_text(main.replace(
        entry, "local actions=" + actions + "\n" + DRIVER + "\nreturn {init=init,tick=observedTick,"),
        encoding="utf-8")
    result = subprocess.run([warp, "check", str(game), "--ticks", str(60 * seconds)],
                            text=True, capture_output=True, timeout=60)
    output = result.stdout
    if result.returncode or '{\n' not in output:
        raise RuntimeError((output + result.stderr)[-3000:])
    snapshot = json.loads(output[output.index('{\n'):])
    if snapshot["errors"]:
        raise AssertionError(snapshot["errors"])
    samples = [Sample(*map(float, line.split()[1:])) for line in output.splitlines()
               if line.startswith("sample ")]
    assert samples and samples[-1].time >= seconds - 0.03, "Incomplete Warp replay"
    return samples


def during(samples, start, end):
    window = [s for s in samples if start <= s.time <= end]
    assert window, f"No samples between {start} and {end}"
    return window


FLOOR = 'land={{0,20,0}},'
CRAB = 'crabs={{from=6,to=7,y=0,speed=0.65}},'


def active_claws(run):
    rows = run(FLOOR + CRAB + 'start={{2,0},{16,0}}',
               '{{2,0,0.4,-1,nil,14.35},{1,0.5,1.8,1}}', 3)
    impact = next(s for s in rows if s.falls > 0)
    assert impact.ax == 2 and impact.ay == 0, "Claw hit did not respawn the walker"
    assert rows[-1].falls == 1, "One collision must count as exactly one fall"
    assert all(abs(s.bx - 14.32) < 0.09 and s.by == 0 for s in rows if s.time > 0.45), (
        "Claw hit disturbed the partner who had walked away from spawn")


def check_bell(rows):
    frozen = during(rows, 1.0, 2.8)
    assert all(s.hush and abs(s.crab_x - frozen[0].crab_x) < 0.001 for s in frozen), (
        "Bell did not hold the crab in place")
    assert frozen[0].ax < frozen[0].crab_x - 0.6 < frozen[-1].ax, "Missing approach"
    assert any(abs(s.ax - s.crab_x) < 0.08 and s.ay == 0 and s.a_on for s in frozen), (
        "Walker never passed through the held crab on foot")
    assert frozen[-1].ax > frozen[-1].crab_x + 0.7 and rows[-1].falls == 0
    released = during(rows, 3.75, 4.5)
    assert all(not s.hush for s in released)
    assert abs(released[-1].crab_x - released[0].crab_x) > 0.15, "Released crab stayed frozen"


def player_bell(run):
    rows = run(FLOOR + CRAB + 'start={{4.9,0},{1.5,0}},bell={3.6,0}',
               '{{2,0,0.5,1,nil,3.6},{1,1.05,2.3,1},{2,3,3.55,-1}}', 5)
    check_bell(rows)


def crate_bell(run):
    rows = run(FLOOR + CRAB + 'start={{4.9,0},{1,0}},bell={3.7,0},crate={2.1,0}',
               '{{2,0,0.47,1,nil,2.95},{2,0.51,0.84,-1},{1,1.05,2.3,1},{2,3,3.95,1,nil,5.3}}', 5)
    held = during(rows, 1.0, 2.8)
    assert all(abs(s.crate - 3.7) < 0.35 and abs(s.bx - 3.7) > 1 for s in held), (
        "Crate must hold the bell without a player on it")
    check_bell(rows)


def balance_grace(run):
    rows = run('start={{2,1.2},{2,0}},land={{0,3.5,0},{3.5,12,0,"balance"},'
               '{3.5,12,-1.15},{7.5,9.5,1.35}}',
               '{{2,0.1,1.3,1,nil,7.0},{1,1.45,1.8,1,"jump",8.6}}', 4)
    before = during(rows, 1.32, 1.43)
    assert all(s.riding and s.bx > 6.8 and abs(s.ax - s.bx) < 0.1 for s in before), (
        "Carrier did not carry the rider across the bridge")
    last_ride = max(s.time for s in rows if s.riding)
    drop = next(s for s in rows if s.time > last_ride and not s.b_on)
    assert 0.49 <= drop.time - last_ride <= 0.55, (
        f"Bridge stopped supporting the carrier after {drop.time - last_ride:.3f}s")
    assert all(s.b_on and s.by == 0 for s in during(rows, last_ride, last_ride + 0.45))
    assert rows[-1].by == -1.15 and rows[-1].ay == 1.35 and rows[-1].falls == 0, (
        "Carrier did not drop to the catch ledge while the rider reached the balcony")


def portal_return(run):
    rows = run(FLOOR + 'start={{2,0},{17,0}},portals={{{4,0},{12,0}}}',
               '{{1,0,0.44,1,nil,12},{1,1.5,1.9,1,nil,13.6},{1,2.0,2.49,-1,nil,4}}', 4)
    jumps = [(a, b) for a, b in zip(rows, rows[1:]) if abs(b.ax - a.ax) > 5]
    assert len(jumps) == 2, f"Expected one outward and one return teleport, got {len(jumps)}"
    outbound, inbound = jumps
    assert outbound[1].ax == 12 and inbound[1].ax == 4
    assert all(abs(s.ax - 12) < 0.4 for s in during(rows, 0.5, 1.49)), "Idle portal ping-pong"
    assert max(s.ax for s in rows) > 13.3 and inbound[1].time > 2
    assert rows[-1].falls == 0 and all(s.bx == 17 for s in rows)


def dive_release(run):
    rows = run('start={{8,0},{17,0}},land={{0,4,0},{4,14,-1.7},{14,20,0}},'
               'water={left=4,right=14,low=0,high=1.35,control={15,0}}',
               '{{1,0.2,0.9,0,"dive"},{2,2.7,3.17,-1,nil,15},{2,4.55,5.04,1,nil,17}}', 7)
    assert min(s.ay for s in during(rows, 0.2, 1.3)) < -1.2, "Down did not submerge the runner"
    assert all(s.a_on and s.ay == 0 for s in during(rows, 2.25, 2.65)), "Did not resurface"
    assert all(s.a_on and s.ay == s.water == 1.35 for s in during(rows, 4.21, 4.5)), (
        "Surfaced runner did not ride the rising water")
    assert rows[-1].ay == rows[-1].water == 0 and rows[-1].a_on and rows[-1].falls == 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("warp", help="Path to the Warp executable")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as temp:
        game = Path(temp) / "game"
        shutil.copytree(GAME, game, ignore=shutil.ignore_patterns("scripts"))
        production = (game / "main.luau").read_text(encoding="utf-8")

        def run(room, actions, seconds):
            return simulate(args.warp, game, production, room, actions, seconds)

        for test in (active_claws, player_bell, crate_bell, balance_grace, portal_return, dive_release):
            test(run)
            print("PASS", test.__name__)


if __name__ == "__main__":
    main()
