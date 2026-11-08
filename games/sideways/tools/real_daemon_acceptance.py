#!/usr/bin/env python3
"""Isolated installed-GameNight-daemon/native-game acceptance (no household UI).

The real daemon launches the native candidate and a headless replacement lobby
with generated launch tokens. Only the daemon samples physical controllers;
no client injects frames. Synthetic players exist only on a fresh loopback port.
Frame arrival can be neutral and does not certify live driving responsiveness.
No user profiles/catalogs are read or changed. Requires Windows and a connected
physical controller, but no button presses. Run serially with other GPU tests.
"""
import argparse
import asyncio
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import time
import traceback

import websockets
from play_lifecycle_acceptance import Probe, seat, positions

GAME = "sideways-kern"
GAME_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_DAEMON = str(Path(os.environ["LOCALAPPDATA"]) / "Ontola.GameNight.Preview/current/bin/gamenight-daemon.exe") if os.environ.get("LOCALAPPDATA") else None


class RealProbe(Probe):
    def __init__(self, args):
        super().__init__(args)
        self.daemon = None
        self.party = {}
        self.host_errors = []
        self.overlay = None
        self.replacement = None
        self.tasks = []
        self.finished = False
        self.candidate_pid = None
        self.candidate_handle = None

    async def observe_candidate_process(self):
        # The installed daemon can insert its own launcher wrapper. Follow
        # only descendants of our new daemon, then require the exact candidate
        # executable. Never identify a game by basename or unrelated PID.
        script = '''
        $all = @(Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,ExecutablePath)
        $owned = [System.Collections.Generic.HashSet[uint32]]::new()
        [void]$owned.Add(ROOT_PID)
        do {
            $next = @($all | Where-Object { $owned.Contains([uint32]$_.ParentProcessId) -and -not $owned.Contains([uint32]$_.ProcessId) })
            foreach ($child in $next) { [void]$owned.Add([uint32]$child.ProcessId) }
            if ($owned.Count -gt 64) { throw 'Fixture process tree unexpectedly large' }
        } while ($next.Count -gt 0)
        $children = @($all | Where-Object { $_.ProcessId -ne ROOT_PID -and $owned.Contains([uint32]$_.ProcessId) })
        ConvertTo-Json -InputObject $children -Compress
        '''.replace('ROOT_PID',str(self.daemon.pid))
        target = os.path.normcase(str(Path(self.args.exe).resolve()))
        end=time.monotonic()+15
        children=[]
        while True:
            raw = (await asyncio.to_thread(subprocess.check_output,["powershell.exe","-NoProfile","-NonInteractive","-Command",script],creationflags=getattr(subprocess,"CREATE_NO_WINDOW",0),timeout=10)).decode("utf-8-sig").strip()
            children = json.loads(raw) if raw else []
            if isinstance(children,dict): children=[children]
            matches = [p for p in children if p.get("ExecutablePath") and os.path.normcase(str(Path(p["ExecutablePath"]).resolve())) == target]
            if len(matches)>1: raise AssertionError(f"Multiple exact candidate descendants under fixture daemon: {matches}")
            if matches: break
            if time.monotonic()>=end: raise AssertionError(f"Cannot identify exact candidate descendant under fixture daemon: {children}")
            await asyncio.sleep(.2)
        self.candidate_pid = matches[0]["ProcessId"]
        api=ctypes.WinDLL("kernel32",use_last_error=True)
        api.OpenProcess.argtypes=[wintypes.DWORD,wintypes.BOOL,wintypes.DWORD]
        api.OpenProcess.restype=wintypes.HANDLE
        self.candidate_handle=api.OpenProcess(0x00100000|0x1000,False,self.candidate_pid)
        if not self.candidate_handle: raise ctypes.WinError(ctypes.get_last_error())

    async def wait_candidate_exit(self):
        api=ctypes.WinDLL("kernel32",use_last_error=True)
        api.WaitForSingleObject.argtypes=[wintypes.HANDLE,wintypes.DWORD]
        api.WaitForSingleObject.restype=wintypes.DWORD
        api.GetExitCodeProcess.argtypes=[wintypes.HANDLE,ctypes.POINTER(wintypes.DWORD)]
        api.GetExitCodeProcess.restype=wintypes.BOOL
        end=time.monotonic()+15
        while time.monotonic()<end:
            status=api.WaitForSingleObject(self.candidate_handle,0)
            if status==0:
                code=wintypes.DWORD()
                if not api.GetExitCodeProcess(self.candidate_handle,ctypes.byref(code)): raise ctypes.WinError(ctypes.get_last_error())
                assert code.value==0, f"Native candidate exited {code.value}"
                return code.value
            if status==0xffffffff: raise ctypes.WinError(ctypes.get_last_error())
            await asyncio.sleep(.05)
        raise AssertionError("Native candidate did not exit naturally within 15 seconds; fixture cleanup is not success")

    async def reader(self, connection, role):
        try:
            async for raw in connection:
                message = json.loads(raw)
                self.record_wire("daemon_to_" + role, message)
                if role == "overlay" and message.get("type") in ("welcome", "party_state"):
                    self.party = message.get("party", {})
                if message.get("type") == "error":
                    self.host_errors.append(message.get("message"))
                if role == "fixture_idle" and message.get("type") == "prepare":
                    ready = {"type":"ready", "session":message["session"]}
                    self.record_wire("fixture_idle_to_real_daemon", ready)
                    await connection.send(json.dumps(ready))
        except websockets.ConnectionClosed:
            pass

    async def command(self, message):
        self.record_wire("overlay_to_real_daemon", message)
        await self.overlay.send(json.dumps(message))

    async def wait_party(self, predicate, label, timeout=20):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if self.host_errors:
                raise AssertionError(f"Real daemon rejected fixture message: {self.host_errors}")
            if predicate(self.party):
                return self.party
            if self.daemon.poll() is not None:
                raise AssertionError(f"Real daemon exited while waiting for {label}")
            await asyncio.sleep(.03)
        raise AssertionError(f"Real daemon never reached {label}")

    async def runtime_controllers(self):
        path=self.out/'runtime-controller-frames.jsonl'
        deadline=time.monotonic()+20
        while time.monotonic()<deadline:
            if self.host_errors: raise AssertionError(self.host_errors)
            if (self.out/'lobby-error.json').exists(): raise AssertionError((self.out/'lobby-error.json').read_text())
            if path.exists():
                complete=path.read_bytes().split(b'\n')[:-1]
                if complete:
                    message=json.loads(complete[-1])
                    if message.get('controllers'): return message['controllers']
            await asyncio.sleep(.05)
        raise AssertionError('No physical controller snapshot from installed runtime; connect a pad before this test')

    async def scenario_real(self, address):
        deadline = time.monotonic() + 10
        while True:
            try:
                self.overlay = await websockets.connect("ws://" + address, open_timeout=1)
                break
            except (OSError, TimeoutError):
                if time.monotonic() > deadline:
                    raise
                await asyncio.sleep(.1)
        await self.overlay.send(json.dumps({"type":"hello", "role":"overlay"}))
        self.tasks.append(asyncio.create_task(self.reader(self.overlay, "overlay")))
        await self.wait_party(lambda p: "players" in p, "isolated welcome")
        assert self.party["players"] == [], "Fresh daemon unexpectedly has household players"
        # Launch the configured, token-authenticated headless lobby. Its API
        # declaration enables the installed runtime's real controller poller.
        await self.command({"type":"open_overlay"})
        controllers=await self.runtime_controllers()
        actual_token=controllers[0]['controller']
        self.passed('installed_runtime_samples_physical_controller_without_injection',{'controller':actual_token,'frame_source':'daemon_to_authenticated_headless_lobby','live_driving_asserted':False})
        await self.command({"type":"join_party", "name":"Fixture Ada", "seat":0, "color":"#3d8fe0"})
        await self.command({"type":"join_party", "name":"Fixture Bo", "seat":3, "color":"#e83a37"})
        await self.wait_party(lambda p: len(p.get("players",[])) == 2, "two synthetic players")
        by_name = {p["name"]:p["id"] for p in self.party["players"]}
        ada, bo = by_name["Fixture Ada"], by_name["Fixture Bo"]
        await self.command({"type":"bind_controller", "player_id":ada, "controller":actual_token})
        await self.wait_party(lambda p: any(s.get("controller") == actual_token for s in p.get("seats",[])), "canonical binding")
        # The first shelf entry deliberately has no executable. The actual
        # candidate is launched only now, after this isolated roster is ready.
        await self.command({"type":"set_playlist", "entries":[{"game":GAME, "title":"Sideways Kern isolated acceptance"}]})
        await self.command({"type":"play_next", "game":GAME})
        running = await self.state(lambda s: s.get("host",{}).get("phase") == "running", "real daemon start", timeout=65)
        assert running["hidden"] is True
        assert running["physics_hz"] == 120 and running["car_substeps"] == 4
        host = running["host"]
        assert host["game"] == GAME and host["session"]
        await self.observe_candidate_process()
        routed = {s["index"]:s["controller"] for s in host["seats"]}
        assert routed[0] == actual_token and routed[3] == "", routed
        self.passed("installed_daemon_launches_token_authenticated_native_game_and_starts_session", {"session":host["session"], "seats":host["seats"]})
        self.passed("installed_host_optional_capabilities_are_explicit", {"godot_sdk":host.get("godot_sdk"), "capabilities":host.get("capabilities",{})})
        if not host.get("capabilities",{}).get("actions",False):
            self.gaps.append({"feature":"host_menu_actions", "detail":"Installed host does not advertise SDK6; optional New stage menu action is not declared. Host settings and normal round transitions remain separate."})
        await self.state(lambda s: s["race_phase"] == "racing", "countdown ends", timeout=12)
        received=await self.state(lambda s: isinstance(s['host'].get('controller_age_ms'),int) and s['host']['controller_age_ms']<250 and actual_token in s['host'].get('controller_tokens',[]),'actual runtime controller frame reaches game')
        assert seat(received,3)['input']['throttle']==0 and seat(received,3)['input']['brake']==0
        self.passed('real_host_frame_reaches_bound_native_seat',{'controller':actual_token,'age_ms':received['host']['controller_age_ms'],'bound_seat':0,'input':seat(received,0)['input'],'unbound_seat_neutral':True,'live_driving_asserted':False})
        self.gaps.append({'feature':'human_driving_through_real_host','detail':'Real runtime frame arrival and canonical binding verified, possibly neutral. Physical button/stick mapping and stale-input cases are separate native/mocked-host tests.'})
        await self.command({"type":"pause"})
        paused = await self.state(lambda s: s["host"]["phase"] == "paused", "host pause")
        mark = self.mark()
        await asyncio.sleep(.35)
        frozen = await self.state(lambda s: s["host"]["phase"] == "paused" and s["race_frame"] == paused["race_frame"], "paused world stays fixed", after=mark)
        assert positions(frozen) == positions(paused) and frozen["race_clock"] == paused["race_clock"]
        await self.command({"type":"resume"})
        await self.state(lambda s: s["host"]["phase"] == "running", "host resume")
        self.passed("real_daemon_pause_resume_freezes_clock_and_cars", {"race_frame":paused["race_frame"]})
        await self.command({"type":"set_setting", "game":GAME, "key":"gearbox", "value":"manual"})
        await self.state(lambda s: all(p["manual"] for p in s["players"]), "real host setting applied")
        self.passed("real_daemon_validates_and_delivers_declared_setting", {"gearbox":"manual"})
        # Profile changes must not reconstruct the world. Pause to make the
        # invariance assertion independent of ordinary car movement.
        await self.command({"type":"pause"})
        before = await self.state(lambda s: s["host"]["phase"] == "paused", "profile update pause")
        await self.command({"type":"rename_player", "player_id":ada, "name":"Fixture Ada renamed"})
        renamed = await self.state(lambda s: seat(s,0)["name"] == "Fixture Ada renamed", "profile rename")
        assert positions(renamed) == positions(before) and renamed["race_round"] == before["race_round"]
        self.passed("real_host_profile_update_preserves_match", {})
        # A tiny synthetic replacement game acknowledges Prepare so the real
        # host can transition and dispose the native candidate. Nothing here
        # talks to a household daemon, catalog or actual lobby.
        self.replacement = await websockets.connect("ws://" + address)
        await self.replacement.send(json.dumps({"type":"hello", "role":"game", "game":"fixture-idle"}))
        self.tasks.append(asyncio.create_task(self.reader(self.replacement,"fixture_idle")))
        await self.command({"type":"set_playlist", "entries":[{"game":"fixture-idle","title":"Fixture idle"}]})
        await self.command({"type":"play_next", "game":"fixture-idle"})
        await self.command({"type":"next"})
        end = time.monotonic() + 8
        while time.monotonic() < end and not (self.out / "final-state.json").exists():
            await asyncio.sleep(.1)
        assert (self.out / "final-state.json").exists(), "Host never disposed native game after replacement became ready"
        final = json.loads((self.out / "final-state.json").read_text())
        assert final.get("exit_reason") == "host_dispose", final.get("exit_reason")
        exit_code=await self.wait_candidate_exit()
        self.passed("real_host_session_dispose_exits_hidden_candidate", {"exit_reason":final.get("exit_reason"),"candidate_pid":self.candidate_pid,"exit_code":exit_code,"forced_cleanup":False})
        self.finished = True

    async def run_real(self):
        exe, daemon = Path(self.args.exe).resolve(), Path(self.args.daemon).resolve()
        if not exe.is_file() or not daemon.is_file():
            raise ValueError("Both executable paths must exist")
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1",0))
            port = reservation.getsockname()[1]
        address = f"127.0.0.1:{port}"
        launch_args = ["play","--smoke-render","100000","--size",self.args.size,"--km","2","--views","1","--seed","1","--state-trace",str(self.trace),"--state-out",str(self.out / "final-state.json")]
        library = [
            {"id":"fixture-idle","title":"Fixture idle","min_players":1,"max_players":4},
            {"id":"fixture-lobby","title":"Headless protocol fixture lobby","launch":{"command":sys.executable,"cwd":str(self.out),"args":[str(Path(__file__).with_name('headless_lobby_fixture.py')),"--out",str(self.out)],"env":{"GAMENIGHT_LOBBY_API":"1"}}},
            {"id":GAME,"title":"Sideways Kern isolated acceptance","min_players":1,"max_players":4,"launch":{"command":str(exe),"cwd":str(Path(self.args.cwd).resolve()),"args":launch_args}}
        ]
        library_path = self.out / "isolated-library.json"
        library_path.write_text(json.dumps(library,indent=2),encoding="utf-8")
        env = {k:v for k,v in os.environ.items() if not k.startswith("GAMENIGHT_") and k != "GAMENIGHT"}
        env.update({"GAMENIGHT_LIBRARY":str(library_path),"GAMENIGHT_DATA_DIR":str(self.out / "isolated-data"),"GAMENIGHT_NO_PREWARM":"1","GAMENIGHT_LOBBY_GAME":"fixture-lobby","GAMENIGHT_NO_MUSIC":"1"})
        report = {"status":"failed","daemon":str(daemon),"daemon_sha256":hashlib.sha256(daemon.read_bytes()).hexdigest(),"executable":str(exe),"executable_sha256":hashlib.sha256(exe.read_bytes()).hexdigest(),"address":address,"library":library,"checks":self.checks,"gaps":self.gaps,
            "limits":["Isolated installed daemon and authenticated headless lobby; synthetic players, real runtime-owned controller snapshots, no input injection.","Neutral frame arrival does not certify live driving or visible presentation.","No published catalog or local-games registration modified."]}
        (self.out / "command.json").write_text(json.dumps({"daemon":[str(daemon),address],"env_overrides":{k:v for k,v in env.items() if k.startswith("GAMENIGHT")}},indent=2))
        with (self.out / "daemon-and-game.log").open("w",encoding="utf-8") as log:
            try:
                self.daemon = subprocess.Popen([str(daemon),address],cwd=daemon.parent,env=env,stdout=log,stderr=subprocess.STDOUT,creationflags=getattr(subprocess,"CREATE_NO_WINDOW",0))
                report["daemon_pid"] = self.daemon.pid
                await asyncio.wait_for(self.scenario_real(address),timeout=120)
                if self.host_errors:
                    raise AssertionError(self.host_errors)
                report["status"] = "passed"
            except Exception as error:
                report["error"] = f"{type(error).__name__}: {error}"
                report["traceback"] = traceback.format_exc()
                print(report["error"],flush=True)
            finally:
                for ws in (self.replacement,self.overlay):
                    if ws is not None:
                        await ws.close()
                for task in self.tasks:
                    task.cancel()
                # Only this invocation's freshly created process tree. Never
                # terminate by game name or touch an existing user daemon.
                if self.daemon is not None and self.daemon.poll() is None:
                    subprocess.run(["taskkill","/PID",str(self.daemon.pid),"/T","/F"],stdout=subprocess.PIPE,stderr=subprocess.STDOUT,creationflags=getattr(subprocess,"CREATE_NO_WINDOW",0),timeout=10)
                    self.daemon.wait(timeout=10)
                if self.candidate_handle:
                    api=ctypes.WinDLL("kernel32",use_last_error=True)
                    api.CloseHandle.argtypes=[wintypes.HANDLE]
                    api.CloseHandle.restype=wintypes.BOOL
                    api.CloseHandle(self.candidate_handle)
                self.wire.close()
                report["elapsed_s"] = time.monotonic() - self.started
                report["host_errors"] = self.host_errors
                (self.out / "report.json").write_text(json.dumps(report,indent=2),encoding="utf-8")
        return 0 if report["status"] == "passed" else 1


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--exe",required=True)
    p.add_argument("--out",required=True)
    p.add_argument("--cwd",default=str(GAME_ROOT))
    p.add_argument("--daemon",default=DEFAULT_DAEMON,help="Installed Gamepad Party daemon executable; defaults to the current user's Preview install")
    p.add_argument("--size",default="640x360")
    args=p.parse_args()
    if os.name != "nt": p.error("This fixture requires Windows")
    if not args.daemon or not Path(args.daemon).is_file(): p.error("Installed Gamepad Party daemon not found; provide --daemon <absolute executable path>")
    return asyncio.run(RealProbe(args).run_real())


if __name__ == "__main__":
    raise SystemExit(main())
