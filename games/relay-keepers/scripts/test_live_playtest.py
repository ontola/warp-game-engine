"""Run the desk against fresh headless Warp processes and one temporary ledger.

Usage: python scripts/test_live_playtest.py /path/to/warp
"""
import argparse
from types import SimpleNamespace
from playtest import live_client, notes_command
from contextlib import contextmanager
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

from playtest_session import LiveClient, PlaytestSession


SCRIPT = Path(__file__).resolve()
GAME = SCRIPT.parents[1]
NOTE = "Retest the shoulder boost after moving the spawn."


def expect(actual, **values):
    for key, expected in values.items():
        if actual.get(key) != expected:
            raise AssertionError(f"{key}: expected {expected!r}, got {actual.get(key)!r}")


def wait_for(read, matches, description, timeout=10):
    deadline = time.monotonic() + timeout
    while True:
        value = read()
        if matches(value):
            return value
        if time.monotonic() >= deadline:
            raise AssertionError(f"Timed out waiting for {description}")
        time.sleep(0.04)


@contextmanager
def server(warp, fixture, phase):
    token = fixture / (phase + "-token.json")
    args = [
        str(warp), "serve", str(fixture / "game"), "--players", "2", "--live", "0",
        "--live-token-file", str(token), "--seconds", "90", "--size", "480x270",
    ]
    env = {**os.environ, "WARP_LIVE_DIR": str(fixture), "WARP_LIVE_CONSOLE": "0"}
    with (fixture / (phase + "-warp.log")).open("w") as log:
        process = subprocess.Popen(args, stdout=log, stderr=subprocess.STDOUT, env=env)
        try:
            def ready():
                if process.poll() is not None:
                    raise RuntimeError(f"Warp exited during startup with code {process.returncode}")
                try:
                    return json.loads(token.read_text(encoding="utf-8")).get("pid") == process.pid
                except (FileNotFoundError, json.JSONDecodeError):
                    return False

            wait_for(ready, bool, "private live token", timeout=30)
            yield LiveClient(token)
        finally:
            if process.poll() is None:
                process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


def clear_room(session, client, key_x, key_y):
    # Supply positions only. The production tick must collect the key and
    # recognize both grounded players at the exit before it reports a clear.
    client.eval(f"state.crew[1].x={key_x}; state.crew[1].y={key_y}; return true")
    wait_for(lambda: client.eval("return state.key"), bool, "key pickup")
    client.eval("state.crew[1].x=17.7; state.crew[1].y=0; "
                "state.crew[2].x=18.3; state.crew[2].y=0; return true")
    status = wait_for(session.status, lambda s: s["cleared"], "natural room clear")
    expect(status, held=True, finished=False)
    return status


def first_session(session, client):
    expect(session.status(), room=1, total=2, active=True, reviewed=0)
    clear_room(session, client, 7.3, 2.65)
    # The ordinary game advances after one second. Review must stay put.
    time.sleep(1.15)
    expect(session.status(), room=1, held=True, cleared=True, reviewed=0)
    session.action("resume")
    expect(session.status(), room=1, held=True, cleared=True)
    session.note("issue", NOTE)
    expect(session.last_note(), room=1, name="Shoulders", kind="issue", text=NOTE)
    expect(session.status(), reviewed=1)
    session.action("next")
    expect(session.status(), room=2, name="Stepwell", held=False, cleared=False, reviewed=1)


def resumed_session(session, client):
    expect(session.status(), room=2, name="Stepwell", total=2, active=True, reviewed=1)
    expect(session.last_note(), room=1, kind="issue", text=NOTE)
    session.action("pause")
    expect(session.status(), held=True)
    t = client.eval("return state.time")
    time.sleep(0.15)
    assert client.eval("return state.time") == t, "simulation advanced during review pause"
    session.action("resume")
    wait_for(lambda: client.eval("return state.time"), lambda value: value > t, "resumed tick")

    session.action("pause")
    client.call("edit_file", {
        "path": "game/rooms.module.luau",
        "old": 'name="Stepwell",start={{1,0},{2.5,0}}',
        "new": 'name="Stepwell revised",start={{4,0},{2.5,0}}',
    })
    status = wait_for(session.status, lambda s: s["name"] == "Stepwell revised",
                      "library hot reload")
    expect(status, room=2, held=True, reviewed=1)
    session.action("retry")
    expect(session.status(), room=2, name="Stepwell revised", held=False, cleared=False)
    assert client.eval("return state.crew[1].x") == 4, "retry used the old room layout"
    clear_room(session, client, 9, 3.9)
    session.note("good", "Verified the new spawn.")
    client.eval("state.review.approve=true; return true")
    expect(session.approve(), finished=True)
    expect(session.approve(), finished=True)
    expect(session.status(), room=2, total=2, held=True, finished=True, reviewed=2)


def run_phase(warp, fixture, phase):
    with server(warp, fixture, phase) as client:
        client = live_client(client.token_file)
        session = PlaytestSession(client, fixture / "ledger")
        try:
            notes = session.db.execute("SELECT * FROM notes ORDER BY id").fetchall()
            visits = session.db.execute("SELECT * FROM visits ORDER BY id").fetchall()
            session.start()
            if phase == "first":
                first_session(session, client)
            elif phase == "resumed":
                resumed_session(session, client)
            elif phase == "finished":
                expect(session.status(), room=2, total=2, held=True, finished=True, reviewed=2)
                expect(session.last_note(), room=2, kind="good")
                session.action("next")
                expect(session.status(), room=2, finished=True, reviewed=2)
                assert tuple(session.db.execute("SELECT room,finished FROM checkpoint").fetchone()) == (2,1)
            else:
                expect(session.status(), room=3, name="Twin Peaks", total=4,
                       active=True, held=False, finished=False, reviewed=2)
                assert tuple(session.db.execute("SELECT room,finished FROM checkpoint").fetchone()) == (3,0)
                assert session.db.execute("SELECT * FROM notes ORDER BY id").fetchall() == notes
                assert session.db.execute("SELECT * FROM visits ORDER BY id").fetchall() == visits
                expect(session.action("next"), room=4, finished=False, reviewed=3)
                expect(session.action("next"), room=4, finished=True, reviewed=4)
                # Keep the same VM and desk open across another campaign extension.
                client.call("edit_file", {
                    "path": "game/rooms.module.luau",
                    "old": "return {rooms[1], rooms[2], rooms[3], rooms[4]}",
                    "new": "return {rooms[1], rooms[2], rooms[3], rooms[4], rooms[5]}",
                })
                wait_for(lambda: client.review("status"), lambda s: s["total"]==5, "appended room reload")
                expect(session.status(), room=5, held=False, finished=False, reviewed=4)
                assert tuple(session.db.execute("SELECT room,finished FROM checkpoint").fetchone()) == (5,0)
                assert session.db.execute("SELECT * FROM notes ORDER BY id").fetchall() == notes
        finally:
            session.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("warp", type=Path)
    parser.add_argument("--phase", choices=("first", "resumed", "finished", "expanded"), help=argparse.SUPPRESS)
    parser.add_argument("--fixture", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    warp = args.warp.resolve(strict=True)
    if args.phase:
        run_phase(warp, args.fixture, args.phase)
        return
    with tempfile.TemporaryDirectory(prefix="relay-live-test-") as temp:
        fixture = Path(temp)
        game = fixture / "game"
        shutil.copytree(GAME, game, ignore=shutil.ignore_patterns("scripts"))
        rooms = game / "rooms.module.luau"
        source = rooms.read_text(encoding="utf-8")
        assert source.rstrip().endswith("return rooms")
        rooms.write_text(source.rstrip().removesuffix("return rooms") +
                         "return {rooms[1], rooms[2]}\n", encoding="utf-8")
        for phase in ("first", "resumed", "finished", "expanded"):
            if phase=="expanded":
                rooms.write_text(rooms.read_text(encoding="utf-8").replace(
                    "return {rooms[1], rooms[2]}", "return {rooms[1], rooms[2], rooms[3], rooms[4]}"
                ),encoding="utf-8")
            subprocess.run([sys.executable, str(SCRIPT), str(warp), "--phase", phase,
                            "--fixture", str(fixture)], check=True, timeout=85)
        args = SimpleNamespace(command="pending",log_dir=fixture/"ledger",seconds=0)
        pending = notes_command(args)
        assert len(pending)==3 and pending[0]["text"]==NOTE
        args.command,args.note_id,args.summary="respond",pending[0]["id"],"Changed spawn and verified on retry."
        notes_command(args)
        args.command="pending"
        assert len(notes_command(args))==2
        args.command="report"
        report=notes_command(args)
        assert sum(v["outcome"]=="cleared" for v in report["visits"])==2
        assert report["notes"][0]["response"]==args.summary
    print("Passed live clear hold, feedback, advance, cross-process resume, "
          "pause, module reload, retry, final coverage and appended-room recovery.")


if __name__ == "__main__":
    main()
