"""Exercise variable jump height, landing buffers and shoulder hops in Warp."""

import argparse
from pathlib import Path
import shutil
import tempfile

from test_mechanics import GAME, simulate

HZ = 60
FLOOR = 'start={{2.6,0},{13,0}},land={{0,20,0}}'
SHOULDERS = 'start={{2.6,0},{3.4,0}},land={{0,20,0}}'
LEDGE = 'start={{3,3},{13,0}},land={{0,3,3},{0,20,0}}'
PRESS = 6


def action(start, duration, dx=0, button=None):
    # A quarter-tick margin keeps decimal serialization away from frame boundaries.
    begin = (start - 0.25) / HZ
    end = (start + duration - 0.25) / HZ
    suffix = f',"{button}"' if button else ''
    return f'{{1,{begin:.9f},{end:.9f},{dx}{suffix}}}'


def floor_arc(rows):
    assert rows[PRESS].ay > 0 and not rows[PRESS].a_on, "Takeoff must start on the press"
    land = next(i for i in range(PRESS + 1, len(rows)) if rows[i].a_on)
    peak = max(range(PRESS, land), key=lambda i: rows[i].ay)
    return rows[peak].ay, peak, land


def test_arcs(run):
    arcs = []
    for duration in (1, 6, 90):
        rows = run(FLOOR, action(PRESS, duration, button="jump"))
        height, peak, land = floor_arc(rows)
        print(f'{duration:2} held ticks: {height:.3f} tiles, '
              f'{(peak-PRESS+1)/HZ:.3f}s to apex, {(land-PRESS+1)/HZ:.3f}s to land')
        assert all(s.a_on and s.ay == 0 for s in rows[land:]), "Holding should jump only once"
        arcs.append((height, peak, land))
    tap, medium, full = arcs
    instant = run(FLOOR, action(PRESS, 1, button="tap"))
    instant_height, _, _ = floor_arc(instant)
    assert abs(instant_height-tap[0]) < 0.18, "Press and release in one input poll must still hop"
    assert 0.30 < tap[0] < 1.00, "Quick tap should make a short hop"
    assert tap[0] + 0.15 < medium[0] < full[0] - 0.10, "Hold duration must give usable intermediate heights"
    assert 1.60 < full[0] < 1.78, "Full jump must still reach authored ledges"
    assert full[1] - PRESS + 1 <= 23 and full[2] - PRESS + 1 <= 44, "Full jump hangs too long"
    return tap[0], full[2]


def test_coyote(run):
    walk = action(PRESS, 70, dx=1)
    baseline = run(LEDGE, walk)
    left = next(i for i,s in enumerate(baseline) if not s.a_on)
    for late, should_jump in ((2, True), (10, False)):
        press = left + late
        rows = run(LEDGE, walk, action(press, 1, dx=1, button="jump"))
        assert not rows[press-1].a_on, "Must already be over the edge"
        assert (rows[press].ay > rows[press-1].ay) == should_jump, f'Unexpected coyote response at +{late} ticks'


def test_released_buffer(run, tap_height, full_landing):
    # Release a single tap three frames before touchdown. It should queue one short hop.
    rows = run(FLOOR, action(PRESS, 26, button="jump"), action(full_landing-3, 1, button="jump"))
    assert rows[full_landing-3].ay < rows[full_landing-4].ay, "Must press while falling"
    assert rows[full_landing+2].ay > rows[full_landing+1].ay > 0, "Buffered tap did not fire on landing"
    second_height = max(s.ay for s in rows[full_landing+1:])
    assert abs(second_height-tap_height) < 0.18, f'Released buffer grew into a full jump ({second_height:.3f})'
    assert rows[-1].a_on and rows[-1].ay == 0


def test_shoulders(run):
    approach = (action(PRESS, 25, button="jump"), action(PRESS, 10, dx=1))
    baseline = run(SHOULDERS, *approach)
    perched = next((i for i,s in enumerate(baseline) if s.riding), None)
    assert perched is not None, "Could not land on partner from the floor"
    assert all(abs(s.ay - 1.2) < 0.01 and s.riding for s in baseline[perched:])
    rows = run(SHOULDERS, *approach, action(perched-2, 30, button="jump"))
    peak = max(s.ay for s in rows[perched:])
    assert 2.8 < peak < 3.0, f'Buffered shoulder jump reached {peak:.3f} tiles'
    assert rows[-1].riding and rows[-1].a_on
    assert all(s.by == 0 and s.b_on and abs(s.bx - 3.4) < 0.01 for s in rows), "Rider disturbed the base"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("warp", help="Path to Warp executable")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as temp:
        game = Path(temp) / "game"
        shutil.copytree(GAME, game, ignore=shutil.ignore_patterns("scripts", "assets"))
        production = (game / "main.luau").read_text(encoding="utf-8")

        def run(room, *inputs):
            rows = simulate(args.warp, game, production, room, '{' + ','.join(inputs) + '}', 2)
            assert rows[-1].falls == 0, "Unexpected fall in jump fixture"
            return rows

        tap, landing = test_arcs(run)
        test_coyote(run)
        test_released_buffer(run, tap, landing)
        test_shoulders(run)
    print("Passed tap, hold, coyote, buffered release and shoulder rebound.")


if __name__ == "__main__":
    main()
