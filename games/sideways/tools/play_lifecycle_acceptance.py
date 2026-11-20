#!/usr/bin/env python3
"""Drive the compiled native play loop through an isolated Splitscreen Club host.

This never talks to the household daemon. --smoke-render creates a hidden
window, and the application must retain normal, wall-clock host input and
simulation semantics in that mode. A fake host is a protocol stimulus; all
asserted state comes from the actual executable's live state trace.

Install the tested dependency: python -m pip install websockets==17.3
From games/sideways/ after building the native executable:
  python tools/play_lifecycle_acceptance.py --exe ../../target/release/sideways.exe --out validation/run1 --extended
On Linux/macOS, omit the executable's .exe suffix. --exe and --out are resolved
relative to the caller's directory; the game's default working directory is
games/sideways/ beside this script's tools/ folder. --cwd can override that directory.

The executable, command, wire transcript, state trace, log, final state and
machine-readable report are preserved, including on failure. Physical USB
controller operation and visible presentation are explicitly not certified.
"""

import argparse
import asyncio
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess
import time
import traceback

import websockets

GAME_ROOT = Path(__file__).resolve().parents[1]
A, B, X, Y, LB, RB, BACK, START = range(8)
SETTINGS_GODOT = {"gearbox", "stage_km", "rivals", "contact", "weather", "vegetation", "render_scale", "anti_aliasing"}
SETTINGS_CORE = {"gearbox", "stage_km", "rivals", "contact", "weather"}


def controller(token, *, throttle=0, brake=0, steer=0, look=(0, 0), buttons=0):
    return {"controller": token, "axes": [steer, 0, look[0], look[1], brake, throttle], "buttons": buttons}


SEATS = [
    {"index": 0, "controller": "pad-a", "occupant": {"kind": "local", "player_id": "ada"}},
    {"index": 1, "occupant": {"kind": "ai"}},
    {"index": 3, "controller": "pad-b", "occupant": {"kind": "local", "player_id": "bo"}},
]
PLAYERS = [
    {"id": "ada", "name": "Ada", "color": "#3d8fe0", "couch": 1},
    {"id": "bo", "name": "Bo", "color": "#e83a37", "couch": 0},
]


class Probe:
    def __init__(self, args):
        self.args = args
        self.out = Path(args.out).resolve()
        self.out.mkdir(parents=True, exist_ok=True)
        self.trace = self.out / "state-trace.jsonl"
        if self.trace.exists():
            raise ValueError("Use a fresh output directory; state-trace.jsonl already exists")
        self.states = []
        self.messages = []
        self.ws = None
        self.proc = None
        self.started = time.monotonic()
        self.checks = []
        self.gaps = []
        self._trace_bytes = 0
        self._pending = b""
        self.wire = (self.out / "wire.jsonl").open("w", encoding="utf-8")

    def record_wire(self, direction, message):
        safe = dict(message)
        if "token" in safe:
            safe["token"] = "<test-token-redacted>"
        self.wire.write(json.dumps({"t": time.monotonic() - self.started, "direction": direction, "message": safe}) + "\n")
        self.wire.flush()

    async def handler(self, ws):
        if self.ws is not None:
            await ws.close(code=1008, reason="one game per test")
            return
        self.ws = ws
        try:
            async for raw in ws:
                message = json.loads(raw)
                self.record_wire("game_to_host", message)
                self.messages.append(message)
        except websockets.ConnectionClosed:
            pass

    def read_states(self):
        if self.trace.exists():
            with self.trace.open("rb") as source:
                source.seek(self._trace_bytes)
                data = source.read()
                self._trace_bytes += len(data)
            chunks = (self._pending + data).split(b"\n")
            self._pending = chunks.pop()
            for raw in chunks:
                if raw.strip():
                    self.states.append(json.loads(raw))
        return self.states

    def mark(self):
        return len(self.read_states())

    async def state(self, predicate, label, after=0, timeout=10):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            # Assert current state, not a transient older matching record.
            # Otherwise an earlier neutral frame could hide a later bad input.
            values = self.read_states()[after:]
            if values and predicate(values[-1]):
                return values[-1]
            if self.proc is not None and self.proc.poll() is not None:
                raise AssertionError(f"Game exited {self.proc.returncode} before {label}")
            await asyncio.sleep(0.05)
        last = self.states[-1] if self.states else None
        raise AssertionError(f"No state matched {label}; last state={last}")

    async def message(self, kind, predicate=lambda m: True, after=0, timeout=45):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            for message in self.messages[after:]:
                if message.get("type") == kind and predicate(message):
                    return message
            if self.proc is not None and self.proc.poll() is not None:
                raise AssertionError(f"Game exited {self.proc.returncode} waiting for {kind}")
            await asyncio.sleep(0.05)
        raise AssertionError(f"Timed out waiting for {kind}")

    async def send(self, message):
        assert self.ws is not None
        self.record_wire("host_to_game", message)
        await self.ws.send(json.dumps(message))

    async def hold(self, controllers, seconds=0.3):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            await self.send({"type": "controller_frame", "controllers": controllers})
            await asyncio.sleep(0.04)

    async def neutral(self, seconds=0.2):
        await self.hold([controller("pad-a"), controller("pad-b")], seconds)

    def passed(self, check, evidence):
        self.checks.append({"check": check, "status": "pass", "evidence": evidence})
        print(f"PASS {check}", flush=True)

    async def scenario(self):
        hello = await self.message("hello")
        assert (hello.get("role"), hello.get("game"), hello.get("token")) == ("game", "sideways", "acceptance-token")
        await self.send({"type": "welcome", "protocol_version": 1, "godot_sdk": 6, "party": {}})
        settings = await self.message("declare_settings")
        keys = {s["key"] for s in settings["settings"]}
        assert SETTINGS_CORE <= keys, keys
        if SETTINGS_GODOT - keys:
            self.gaps.append({"feature": "settings_parity", "missing": sorted(SETTINGS_GODOT - keys)})
        weather_definition = next(s for s in settings["settings"] if s["key"] == "weather")
        if "next stage" in weather_definition.get("label", "").lower():
            self.gaps.append({"feature": "live_explicit_weather", "detail": "Native declaration defers weather; Godot applies explicit weather immediately"})
        actions = await self.message("declare_actions")
        assert "new_stage" in {a["key"] for a in actions["actions"]}
        self.passed("authenticated_handshake_and_capability_declarations", {"settings": sorted(keys)})

        await self.send({"type": "setting_changed", "game": "sideways", "key": "stage_km", "value": 2})
        await self.send({"type": "setting_changed", "game": "sideways", "key": "rivals", "value": 0})
        await self.send({"type": "prepare", "game": "wrong-game", "session": "wrong", "seats": SEATS, "players": PLAYERS})
        await asyncio.sleep(0.15)
        assert not any(m.get("type") == "ready" for m in self.messages)
        prepare = {"type": "prepare", "game": "sideways", "session": "s1", "seats": SEATS, "players": PLAYERS}
        # Back-to-back wire events exercise session replacement while stage
        # preparation is pending. No sleep/ready wait separates these sends;
        # OS scheduling may still deliver them in separate transport polls.
        await self.send(dict(prepare, session="superseded"))
        await self.send(prepare)
        await self.message("ready", lambda m: m.get("session") == "s1")
        ready = await self.state(lambda s: s.get("host", {}).get("phase") == "ready" and s["host"].get("session") == "s1", "prepared final session", timeout=45)
        assert ready["host"]["session"] == "s1"
        assert ready["host"]["godot_sdk"] == 6
        assert ready["host"]["capabilities"]["actions"] is True
        assert ready["physics_hz"] == 120 and ready["car_substeps"] == 4
        assert ready["hidden"] is True
        assert ready["render_quiet"] is True and ready["window_visible"] is False
        assert all(p["camera_mode"] is False and abs(p.get("camera_yaw", 0.0)) < 1e-6 for p in ready["players"])
        order = [s["index"] for s in ready["host"]["seats"]]
        assert order == [3, 0, 1], order
        assert ready["view_order"] == [3, 0, 1], ready["view_order"]
        assert len(ready["view_rects"]) == 3
        validate_rectangles(ready["view_rects"], self.args.size)
        participation = await self.message("participation", lambda m: m.get("session") == "s1")
        assert participation.get("instant_join") is False
        self.passed("prepare_build_ready_and_canonical_couch_order", {"view_order": ready["view_order"], "view_rects": ready["view_rects"]})
        self.passed("back_to_back_prepares_replace_session_without_exit", {
            "session": ready["host"]["session"],
            "ready_sessions": [m.get("session") for m in self.messages if m.get("type") == "ready"],
            "transport_poll_batching": "not guaranteed; back-to-back wire events",
        })

        mark = self.mark()
        await self.send(prepare)
        await self.send({"type": "dispose", "session": "superseded"})
        for kind in ("start", "pause", "resume", "dispose"):
            await self.send({"type": kind, "session": "other-session"})
        await asyncio.sleep(0.3)
        steady = await self.state(lambda s: s.get("host", {}).get("phase") == "ready", "wrong-session ignored", after=mark)
        assert steady["race_frame"] == ready["race_frame"]
        assert sum(m.get("type") == "ready" and m.get("session") == "s1" for m in self.messages) == 1
        self.passed("duplicate_prepare_and_wrong_session_do_not_mutate", {"race_frame": steady["race_frame"]})

        await self.hold([controller("pad-a", buttons=1 << START), controller("pad-b")], 0.15)
        mark = self.mark()
        await self.send({"type": "start", "session": "s1"})
        await self.hold([controller("pad-a", buttons=1 << START), controller("pad-b")], 0.35)
        running = await self.state(lambda s: s.get("host", {}).get("phase") == "running", "start running", after=mark)
        assert not running["paused"], "Start held in the lobby opened the game pause menu"
        await self.neutral(0.25)
        self.passed("held_start_is_suppressed_on_session_start", {"paused": running["paused"]})
        await self.state(lambda s: s.get("race_phase") == "racing", "countdown ends", timeout=15)

        before_drive = self.read_states()[-1]
        mark = self.mark()
        await self.hold([controller("pad-b", brake=32767, steer=-32768), controller("pad-a", throttle=32767, steer=32767)], 1.0)
        driven = await self.state(lambda s: seat(s, 0)["input"]["throttle"] > 0.99 and seat(s, 3)["input"]["brake"] > 0.99, "canonical token input", after=mark)
        assert seat(driven, 0)["input"]["steer"] > 0.99
        assert seat(driven, 3)["input"]["steer"] < -0.99
        self.passed("controller_tokens_route_independently_of_array_or_couch_order", {"seat0": seat(driven, 0)["input"], "seat3": seat(driven, 3)["input"]})
        start_pos, end_pos = seat(before_drive, 0)["position"], seat(driven, 0)["position"]
        horizontal_motion = math.hypot(end_pos[0] - start_pos[0], end_pos[2] - start_pos[2])
        assert horizontal_motion > 0.1, f"RT reached diagnostic input but the car moved only {horizontal_motion} m horizontally"
        self.passed("right_trigger_moves_the_real_simulated_car", {"horizontal_m": horizontal_motion, "before": start_pos, "after": end_pos})

        mark = self.mark()
        await self.hold([controller("pad-b", throttle=32767)], 0.3)
        missing = await self.state(lambda s: seat(s, 0)["input"]["throttle"] == 0 and seat(s, 3)["input"]["throttle"] > 0.99, "missing controller neutral", after=mark)
        self.passed("missing_controller_neutralizes_same_frame", {"seat0_input": seat(missing, 0)["input"]})
        mark = self.mark()
        await self.hold([controller("pad-a", throttle=0), controller("pad-a", throttle=32767), controller("pad-b")], 0.3)
        duplicate = await self.state(lambda s: s["host"].get("controller_age_ms", 9999) < 200 and seat(s, 0)["input"]["throttle"] == 0, "duplicate token first wins", after=mark)
        self.passed("duplicate_controller_token_first_wins", {"throttle": seat(duplicate, 0)["input"]["throttle"]})
        await self.hold([controller("pad-a", throttle=32767), controller("pad-b")], 0.2)
        mark = self.mark()
        await asyncio.sleep(0.4)
        stale = await self.state(lambda s: s.get("host", {}).get("controller_age_ms", 0) >= 250 and seat(s, 0)["input"]["throttle"] == 0, "250ms stale input neutral", after=mark)
        self.passed("stale_controller_input_releases", {"age_ms": stale["host"]["controller_age_ms"], "input": seat(stale, 0)["input"]})

        await self.neutral()
        mark = self.mark()
        await self.hold([controller("pad-a", buttons=1 << START), controller("pad-b")], 0.25)
        menu = await self.state(lambda s: s["paused"] and s["host"]["phase"] == "running", "local pause", after=mark)
        mark = self.mark()
        await self.hold([controller("pad-a", throttle=32767, buttons=1 << START), controller("pad-b")], 0.4)
        frozen = await self.state(lambda s: s["paused"] and s["race_frame"] == menu["race_frame"], "paused clock fixed", after=mark)
        assert frozen["race_clock"] == menu["race_clock"]
        assert positions(frozen) == positions(menu)
        self.passed("held_start_single_edge_and_local_pause_freezes_world", {"race_frame": frozen["race_frame"], "race_clock": frozen["race_clock"]})
        await self.neutral()
        mark = self.mark()
        await self.hold([controller("pad-a", buttons=1 << START), controller("pad-b")], 0.2)
        await self.state(lambda s: not s["paused"], "local unpause", after=mark)
        await self.neutral()

        mark = self.mark()
        await self.send({"type": "pause", "session": "s1"})
        paused = await self.state(lambda s: s["host"]["phase"] == "paused", "host pause", after=mark)
        await self.hold([controller("pad-a", throttle=32767, look=(32767, 0)), controller("pad-b")], 0.4)
        mark = self.mark()
        renamed_profiles = [dict(PLAYERS[0], name="Joep", couch=0), dict(PLAYERS[1], couch=1)]
        await self.send({"type": "players_updated", "session": "s1", "players": renamed_profiles})
        renamed = await self.state(lambda s: seat(s, 0)["name"] == "Joep" and s["view_order"] == [0, 3, 1], "profile rename and view reorder", after=mark)
        assert renamed["race_frame"] == paused["race_frame"]
        assert renamed["race_round"] == paused["race_round"]
        assert positions(renamed) == positions(paused), "Profile/couch change reset or swapped cars"
        assert paused["render_quiet"] is True and renamed["render_quiet"] is True
        assert paused["window_visible"] is False and renamed["window_visible"] is False
        assert renamed["rendered_frames"] == paused["rendered_frames"], "Host-paused game still submits GPU render frames"
        self.passed("host_pause_profile_rename_and_couch_order_preserve_car_state", {"view_order": renamed["view_order"]})
        self.passed("host_pause_hides_window_and_stops_render_submission", {"rendered_frames": renamed["rendered_frames"], "window_visible": renamed["window_visible"]})

        await self.hold([controller("pad-a", buttons=1 << START), controller("pad-b")], 0.15)
        mark = self.mark()
        await self.send({"type": "resume", "session": "s1"})
        await self.hold([controller("pad-a", buttons=1 << START), controller("pad-b")], 0.3)
        resumed = await self.state(lambda s: s["host"]["phase"] == "running", "host resume", after=mark)
        assert not resumed["paused"]
        assert resumed["render_quiet"] is False
        resumed_gpu = await self.state(lambda s: s["rendered_frames"] > renamed["rendered_frames"], "GPU rendering resumes", after=mark)
        self.passed("host_resume_does_not_reuse_lobby_start_edge", {"paused": resumed["paused"]})
        self.passed("host_resume_restarts_render_submission", {"before": renamed["rendered_frames"], "after": resumed_gpu["rendered_frames"]})
        await self.neutral()
        mark = self.mark()
        await self.send({"type": "setting_changed", "game": "sideways", "key": "gearbox", "value": "manual"})
        manual = await self.state(lambda s: seat(s, 0).get("manual") is True, "live gearbox setting", after=mark)
        self.passed("gearbox_setting_applies_to_live_car", {"manual": seat(manual, 0)["manual"]})

        await self.neutral()
        mark = self.mark()
        await self.hold([controller("pad-a", buttons=1 << A), controller("pad-b")], 0.25)
        handbrake = await self.state(lambda s: seat(s, 0)["input"]["handbrake"] == 1, "A handbrake", after=mark)
        assert seat(handbrake, 3)["input"]["handbrake"] == 0
        self.passed("A_handbrake_is_seat_local", {"handbrake": seat(handbrake, 0)["input"]["handbrake"]})
        await self.neutral()
        old_camera = seat(self.read_states()[-1], 0)["camera_mode"]
        mark = self.mark()
        await self.hold([controller("pad-a", buttons=1 << Y), controller("pad-b")], 0.5)
        camera = await self.state(lambda s: seat(s, 0)["camera_mode"] != old_camera, "Y camera single edge", after=mark)
        self.passed("held_Y_changes_camera_once", {"before": old_camera, "after": seat(camera, 0)["camera_mode"]})
        await self.neutral()

        if "camera_yaw" in seat(self.read_states()[-1], 0):
            for label, raw, expected in (("right", (32767, 0), -math.pi / 2), ("down", (0, 32767), math.pi), ("up", (0, -32767), 0.0), ("left", (-32767, 0), math.pi / 2), ("released", (0, 0), 0.0)):
                mark = self.mark()
                await self.hold([controller("pad-a", look=raw), controller("pad-b")], 0.6)
                turned = await self.state(lambda s: angle_error(seat(s, 0)["camera_yaw"], expected) < 0.15, f"right-stick {label} orbit", after=mark)
                self.passed(f"right_stick_{label}_orbit", {"yaw": seat(turned, 0)["camera_yaw"], "expected": expected})
        else:
            self.gaps.append({"feature": "camera_orbit_e2e", "detail": "Trace lacks camera_yaw; right-stick orbit direction not certified"})

        await self.neutral()
        old_penalty = seat(self.read_states()[-1], 0)["penalty"]
        mark = self.mark()
        await self.hold([controller("pad-a", buttons=1 << X), controller("pad-b")], 0.5)
        recovery = await self.state(lambda s: seat(s, 0)["penalty"] == old_penalty + 5, "X recovery once", after=mark)
        self.passed("held_X_recovers_once_with_five_second_penalty", {"before": old_penalty, "after": seat(recovery, 0)["penalty"]})
        await self.neutral()

        # Back is host-owned; sending its wire bit cannot pause or recover.
        before = self.read_states()[-1]
        mark = self.mark()
        await self.hold([controller("pad-a", buttons=1 << BACK), controller("pad-b")], 0.3)
        back = await self.state(lambda s: s["host"]["phase"] == "running" and not s["paused"], "Back stays out of gameplay", after=mark)
        assert seat(back, 0)["penalty"] == seat(before, 0)["penalty"]
        self.passed("back_button_is_not_gameplay_action", {"paused": back["paused"], "penalty": seat(back, 0)["penalty"]})
        await self.neutral()

        if self.args.extended:
            await self.next_stage_checks()

        mark = self.mark()
        await self.send({"type": "dispose", "session": "s1"})
        disposed = await self.state(lambda s: s.get("host", {}).get("phase") == "idle" and not s.get("host", {}).get("session"), "dispose clears session", after=mark)
        assert disposed["hidden"] is True
        self.passed("dispose_clears_session", {"host": disposed["host"], "exit_reason": disposed.get("exit_reason")})

    async def next_stage_checks(self):
        mark = self.mark()
        await self.send({"type": "pause", "session": "s1"})
        await self.state(lambda s: s["host"]["phase"] == "paused", "pause before deferred roster", after=mark)
        before = self.read_states()[-1]
        next_seats = [SEATS[0], SEATS[1], {"index": 2, "occupant": {"kind": "ai"}},
                      {"index": 3, "controller": "pad-c", "occupant": {"kind": "local", "player_id": "cy"}}]
        next_players = [dict(PLAYERS[0], name="Joep", couch=0), {"id": "cy", "name": "Cy", "couch": 1}]
        mark = self.mark()
        await self.send({"type": "party_updated", "session": "s1", "seats": next_seats, "players": next_players, "presence": []})
        pending_roster = await self.state(lambda s: len(s["host"]["seats"]) == 4, "pending roster", after=mark)
        assert len(pending_roster["players"]) == len(before["players"]) == 3
        assert positions(pending_roster) == positions(before)
        assert seat(pending_roster, 3)["player_id"] == "bo", "Deferred join replaced the current driver immediately"
        self.passed("roster_changes_wait_for_next_stage_and_preserve_existing_cars", {"pending_seats": pending_roster["host"]["seats"]})
        await self.send({"type": "resume", "session": "s1"})
        mark = self.mark()
        await self.hold([controller("pad-a"), controller("pad-b", throttle=32767), controller("pad-c", throttle=32767)], 0.3)
        departed = await self.state(lambda s: s["host"]["phase"] == "running" and seat(s, 3)["input"]["throttle"] == 0, "departed player remains neutral", after=mark)
        self.passed("departed_driver_does_not_inherit_old_or_new_controller", {"old_seat_input": seat(departed, 3)["input"]})
        mark = self.mark()
        for key, value in (("stage_km", 3), ("contact", False), ("weather", "wet")):
            await self.send({"type": "setting_changed", "game": "sideways", "key": key, "value": value})
        pending = await self.state(lambda s: s.get("pending_settings", {}).get("stage_km") == 3, "pending stage settings", after=mark)
        assert pending["race_round"] == before["race_round"]
        assert pending["active_settings"]["stage_km"] == 2
        assert pending["active_settings"]["contact"] is True
        self.passed("route_settings_are_deferred", {"active": pending["active_settings"], "pending": pending["pending_settings"]})
        mark = self.mark()
        await self.send({"type": "action_triggered", "key": "new_stage", "player_id": "ada"})
        changed = await self.state(lambda s: s["race_round"] > before["race_round"] and s.get("active_settings", {}).get("stage_km") == 3, "next stage applies settings", after=mark, timeout=45)
        assert changed["active_settings"]["contact"] is False
        assert changed["active_settings"]["weather"] == "wet"
        assert len(changed["players"]) == 4
        assert sorted(p["seat"] for p in changed["players"]) == [0, 1, 2, 3], "Two AI seats lost distinct identities"
        assert seat(changed, 3)["player_id"] == "cy"
        assert len(changed["view_rects"]) == 4
        validate_rectangles(changed["view_rects"], self.args.size)
        assert all(p["camera_mode"] is False and abs(p.get("camera_yaw", 0.0)) < 1e-6 for p in changed["players"]), "New round inherited a previous player's camera state"
        self.passed("new_stage_action_applies_deferred_settings", {"active": changed["active_settings"], "round": changed["race_round"]})
        self.passed("next_stage_applies_join_leave_and_distinct_AI_seats", {"view_order": changed["view_order"], "view_rects": changed["view_rects"]})
        self.passed("new_round_resets_camera_state_for_all_seats", {"cameras": [{"seat": p["seat"], "mode": p["camera_mode"], "yaw": p.get("camera_yaw")} for p in changed["players"]]})

    async def run(self):
        report = {"schema": 1, "suite": "native_play_gamenight_live_loop", "status": "failed", "extended": self.args.extended, "host_protocol_version": 1, "host_sdk": 6, "checks": self.checks, "known_parity_gaps": self.gaps,
                  "not_certified": ["physical controller button operation", "visible display presentation", "120 FPS gameplay", "full SDK feature parity"]}
        log = None
        try:
            exe = Path(self.args.exe).resolve()
            report["executable"] = str(exe)
            report["sha256"] = hashlib.sha256(exe.read_bytes()).hexdigest()
            async with websockets.serve(self.handler, "127.0.0.1", 0) as server:
                port = server.sockets[0].getsockname()[1]
                env = dict(os.environ, GAMENIGHT="1", GAMENIGHT_ADDR=f"127.0.0.1:{port}", GAMENIGHT_GAME_ID="sideways", GAMENIGHT_TOKEN="acceptance-token")
                command = [str(exe), "play", "--smoke-render", "100000", "--size", self.args.size, "--km", "2", "--views", "1", "--seed", "1", "--state-trace", str(self.trace), "--state-out", str(self.out / "final-state.json")]
                report["command"] = command
                log = (self.out / "game.log").open("wb")
                self.proc = subprocess.Popen(command, cwd=self.args.cwd, env=env, stdout=log, stderr=subprocess.STDOUT, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
                report["pid"] = self.proc.pid
                await self.scenario()
                report["status"] = "passed_core_with_documented_gaps"
        except Exception as error:
            report["error"] = f"{type(error).__name__}: {error}"
            report["traceback"] = traceback.format_exc()
            print(report["error"], flush=True)
        finally:
            if self.proc is not None:
                report["cleanup_terminated_process"] = False
                try:
                    # GPU resource destruction can outlast the final dispose
                    # trace. A forced cleanup is not a successful normal exit.
                    self.proc.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    report["cleanup_terminated_process"] = True
                    self.proc.terminate()
                    try:
                        self.proc.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        self.proc.kill()
                        self.proc.wait(timeout=5)
                report["exit_code"] = self.proc.returncode
                if report["status"].startswith("passed") and (self.proc.returncode != 0 or report["cleanup_terminated_process"]):
                    report["status"] = "failed_process_shutdown_after_functional_checks"
                    report["error"] = "Functional checks completed, but the executable did not exit normally with code 0 after dispose"
            if log:
                log.close()
            self.read_states()
            self.wire.close()
            report["state_count"] = len(self.states)
            report["elapsed_s"] = time.monotonic() - self.started
            report["last_state"] = self.states[-1] if self.states else None
            (self.out / "acceptance.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
        return 0 if report["status"].startswith("passed") else 1


def seat(state, index):
    return next(p for p in state["players"] if p.get("seat") == index)


def positions(state):
    return {p["seat"]: p["position"] for p in state["players"]}


def angle_error(actual, expected):
    return abs((actual - expected + math.pi) % (2 * math.pi) - math.pi)


def validate_rectangles(rects, size):
    width, height = map(int, size.split("x"))
    for i, (x, y, w, h) in enumerate(rects):
        assert w > 0 and h > 0 and x >= 0 and y >= 0 and x + w <= width and y + h <= height
        for ox, oy, ow, oh in rects[:i]:
            assert x >= ox + ow or ox >= x + w or y >= oy + oh or oy >= y + h, "View rectangles overlap"
    assert sum(r[2] * r[3] for r in rects) <= width * height
    if len(rects) == 2:
        assert rects[0][0] == rects[1][0] == 0 and rects[0][2] == rects[1][2] == width, "Two-player layout must be top/bottom"
    if size == "3840x2160" and len(rects) >= 3:
        assert all(r[2:] == [1916, 1076] for r in rects), rects


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, help="Path to the already compiled native sideways executable")
    parser.add_argument("--out", required=True, help="Fresh evidence directory; existing state traces are rejected")
    parser.add_argument("--cwd", default=str(GAME_ROOT), help="Game working directory (default: this checkout's Sideways directory)")
    parser.add_argument("--size", default="640x360", help="Hidden render output size WIDTHxHEIGHT (default: 640x360)")
    parser.add_argument("--extended", action="store_true", help="Also test deferred roster/settings, next stage and camera reset")
    args = parser.parse_args()
    return asyncio.run(Probe(args).run())


if __name__ == "__main__":
    raise SystemExit(main())
