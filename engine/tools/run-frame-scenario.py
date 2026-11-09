#!/usr/bin/env python3
"""Run one bounded scenario and analyze its profile. Python 3.11+, no auto-build."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import subprocess
import sys
import struct
import stat
import time
import tomllib


# The exact disk-override registry in render/src/shaders.rs. Never scan an
# arbitrary directory, hash source-tree fallbacks, or read unrelated files.
SHADER_OVERRIDE_NAMES = (
    "foliage_volume.wgsl",
    "foliage_sun_pages.wgsl", "terrain_trace.wgsl", "world_visibility.wgsl",
    "wind_common.wgsl", "meadow_field.wgsl", "shade.wgsl", "car_material.wgsl", "car_lamps.wgsl",
    "grass_lighting.wgsl", "incident_lights.wgsl", "headlights.wgsl", "transparency.wgsl",
    "grass_transition.wgsl", "far_grass.wgsl", "voxel_forest.wgsl",
)
SHADER_FILE_LIMIT = 2 * 1024 * 1024


def shader_provenance(environment, cwd, executable_sha256):
    """Bounded input snapshot, not proof of which live-reload generation ran."""
    requested = environment.get("KERN_SHADER_DIR")
    directory = None
    directory_status = "unset"
    if requested is not None:
        candidate = Path(requested)
        if not candidate.is_absolute():
            candidate = Path(cwd) / candidate
        try:
            # Rust Path("").is_dir() is false; Python Path("") means cwd.
            if requested and candidate.is_dir():
                directory = candidate.resolve()
                directory_status = "active"
            else:
                directory_status = "missing_or_not_directory"
        except OSError:
            directory_status = "unreadable"
    files = []
    verified = directory_status != "unreadable"
    for name in sorted(SHADER_OVERRIDE_NAMES):
        entry = {"name": name, "source": "embedded", "sha256": None, "bytes": None}
        if directory is not None:
            path = directory / name
            try:
                resolved = path.resolve()
                if not resolved.is_relative_to(directory):
                    raise ValueError("outside_override_directory")
                info = path.stat()
                if not stat.S_ISREG(info.st_mode):
                    raise ValueError("not_regular_file")
                if info.st_size > SHADER_FILE_LIMIT:
                    raise ValueError("over_size_limit")
                with path.open("rb") as source:
                    data = source.read(SHADER_FILE_LIMIT + 1)
                if len(data) > SHADER_FILE_LIMIT:
                    raise ValueError("over_size_limit")
                data.decode("utf-8")  # Renderer overrides require valid UTF-8.
                entry.update(source="override", sha256=hashlib.sha256(data).hexdigest(), bytes=len(data))
            except FileNotFoundError:
                pass  # Missing file falls back to this executable's built-in.
            except (OSError, UnicodeError, ValueError) as error:
                entry.update(source="unverified", reason=str(error) if isinstance(error, ValueError) and not isinstance(error, UnicodeError) else type(error).__name__)
                verified = False
        files.append(entry)
    identity = {"registry": 1, "executable_sha256": executable_sha256, "files": files}
    digest = hashlib.sha256(json.dumps(identity, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    return {"schema": 1, "content_sha256": digest, "executable_sha256": executable_sha256,
            "directory": str(directory) if directory else requested, "directory_status": directory_status,
            "verified": verified, "files": files, "per_file_limit_bytes": SHADER_FILE_LIMIT,
            "scope": "Exact bytes of registered UTF-8 override files only; embedded fallbacks are identified by executable SHA256. Before/after snapshots cannot detect edits reverted during a run or prove accepted live-reload generations."}


def renderer_key(key):
    return (key.startswith(("KERN_", "SIDEWAYS_")) or key == "WGPU_BACKEND") and not any(
        secret in key.upper() for secret in ("TOKEN", "PASSWORD", "SECRET", "CREDENTIAL", "API_KEY"))


def git_state(engine):
    result = {}
    for key, arguments in [("head", ["rev-parse", "HEAD"]), ("status", ["status", "--porcelain=v1"])]:
        try:
            run = subprocess.run(["git", *arguments], cwd=engine, text=True, capture_output=True, timeout=15)
            result[key] = run.stdout.strip() if run.returncode == 0 else None
        except (OSError, subprocess.TimeoutExpired):
            result[key] = None
    result["dirty"] = bool(result["status"]) if result["status"] is not None else None
    return result


def startup_records(path):
    records = []
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        try:
            value = json.loads(line)
        except ValueError:
            continue
        if isinstance(value, dict) and value.get("kind") in ("renderer_startup", "cli_startup"):
            records.append(value)
    return records


def image_size(path):
    with path.open("rb") as image:
        header = image.read(24)
        if len(header) == 24 and header[:8] == b'\x89PNG\r\n\x1a\n':
            return list(struct.unpack(">II", header[16:24]))
        image.seek(0)
        if image.read(2) != b'\xff\xd8':
            return None
        while True:
            if image.read(1) != b'\xff':
                return None
            marker = image.read(1)
            while marker == b'\xff':
                marker = image.read(1)
            if not marker or marker[0] in (0xD9, 0xDA):
                return None
            length = image.read(2)
            if len(length) != 2:
                return None
            length = struct.unpack(">H", length)[0]
            if length < 2:
                return None
            if marker[0] in (0xC0, 0xC1, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF):
                shape = image.read(5)
                if len(shape) != 5:
                    return None
                height, width = struct.unpack(">HH", shape[1:])
                return [width, height]
            image.seek(length - 2, 1)


def play_controls(wind=None, wind_time=None, debug_ui_script=None):
    """Validate bounded replay controls before creating output or launching a child."""
    command, metadata, script_bytes = [], {}, None
    for flag, value, maximum in [("wind", wind, 1.0), ("wind-time", wind_time, 1.0e9)]:
        if value is not None:
            if not math.isfinite(value) or not 0 <= value <= maximum:
                raise ValueError(f"--{flag} must be finite and in 0..{maximum:g}")
            command += [f"--{flag}", str(value)]
            metadata[flag.replace("-", "_")] = value
    if debug_ui_script is not None:
        path = Path(debug_ui_script).resolve()
        with path.open("rb") as source:
            script_bytes = source.read(65_537)
        if len(script_bytes) > 65_536:
            raise ValueError("debug UI script exceeds 64 KiB")
        rows = json.loads(script_bytes.decode("utf-8"))
        if not isinstance(rows, list) or len(rows) > 256:
            raise ValueError("debug UI script must contain at most 256 events")
        previous = 0
        for row in rows:
            if not isinstance(row, dict):
                raise ValueError("debug UI event must be an object")
            frame = row.get("frame")
            if type(frame) is not int or not previous <= frame <= 4096 or frame < 1:
                raise ValueError("debug UI event frames must be sorted integers in 1..4096")
            previous = frame
            actions = [key for key in ("key", "pointer", "pointer_down", "pointer_up") if key in row]
            if len(actions) != 1:
                raise ValueError("debug UI event requires exactly one input action")
            action = actions[0]
            if action == "key" and row[action] not in (
                "toggle", "close", "up", "down", "left", "right", "enter", "page_up", "page_down"
            ):
                raise ValueError("unknown debug UI key")
            if action == "pointer":
                point = row[action]
                if (not isinstance(point, list) or len(point) != 2 or any(
                    type(v) not in (int, float) or not math.isfinite(v) or abs(v) > 32768 for v in point
                )):
                    raise ValueError("debug UI pointer requires two finite coordinates within +/-32768")
            if action in ("pointer_down", "pointer_up") and row[action] is not True:
                raise ValueError("debug UI pointer press/release must be true")
        metadata["debug_ui_script"] = {
            "source": str(path), "sha256": hashlib.sha256(script_bytes).hexdigest(), "events": len(rows)
        }
    return command, metadata, script_bytes


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scenario", type=Path)
    parser.add_argument("--exe", required=True, type=Path, help="already built Sideways executable")
    parser.add_argument("--out", required=True, type=Path, help="new output directory; never reused")
    visibility = parser.add_mutually_exclusive_group()
    visibility.add_argument("--visible", dest="visible", action="store_true", help="show bounded play window (default; measures real presentation cadence)")
    visibility.add_argument("--hidden", dest="visible", action="store_false", help="hidden synchronized diagnostic mode; not gameplay pacing")
    parser.set_defaults(visible=True)
    parser.add_argument("--windowed", action="store_true", help="allow a visible window; OS chrome may clamp requested output and will be checked")
    parser.add_argument("--env-file", type=Path, help="JSON object of explicit nonsecret KERN_/SIDEWAYS_/WGPU_BACKEND values")
    parser.add_argument("--wind", type=float, help="explicit wind strength in 0..1")
    parser.add_argument("--wind-time", type=float, help="hold wind at this timestamp in seconds (0..1e9)")
    parser.add_argument("--debug-ui-script", type=Path, help="bounded frame/input JSON; copied and hashed with the run")
    parser.add_argument("--timeout", type=float, default=300, help="maximum process seconds; kills only this spawned child on timeout")
    parser.add_argument("--build-ms", type=float, help="optional externally measured build duration, no build is performed")
    args = parser.parse_args(argv)
    runner_start = time.perf_counter()
    engine = Path(__file__).resolve().parents[1]
    exe, scenario, out = args.exe.resolve(), args.scenario.resolve(), args.out.resolve()
    try:
        if not exe.is_file() or not scenario.is_file():
            raise ValueError("--exe and scenario must be existing files")
        if not math.isfinite(args.timeout) or args.timeout <= 0:
            raise ValueError("--timeout must be finite and positive")
        if args.build_ms is not None and (not math.isfinite(args.build_ms) or args.build_ms < 0):
            raise ValueError("--build-ms must be finite and nonnegative")
        control_args, controls, script_bytes = play_controls(args.wind, args.wind_time, args.debug_ui_script)
        case_bytes = scenario.read_bytes()
        case = tomllib.loads(case_bytes.decode("utf-8-sig"))
        frames = sum(section["frames"] for section in case.get("sections", []))
        if not isinstance(frames, int) or not 1 <= frames <= 4096:
            raise ValueError("scenario must contain 1..4096 total frames")
        capture_config = case.get("capture", {})
        capture_format = capture_config.get("format", "jpg")
        if capture_format == "jpeg":
            capture_format = "jpg"
        if capture_format not in ("jpg", "png"):
            raise ValueError("capture.format must be jpg, jpeg or png")
        environment = dict(os.environ)
        overrides = {}
        env_bytes = None
        if args.env_file:
            env_bytes = args.env_file.read_bytes()
            overrides = json.loads(env_bytes)
            if not isinstance(overrides, dict):
                raise ValueError("--env-file must contain a JSON object")
            for key, value in overrides.items():
                if not renderer_key(key) or (value is not None and not isinstance(value, (str, int, float, bool))):
                    raise ValueError(f"unsupported renderer environment setting: {key}")
                if isinstance(value, float) and not math.isfinite(value):
                    raise ValueError(f"nonfinite environment setting: {key}")
                if value is None:
                    environment.pop(key, None)
                else:
                    environment[key] = str(int(value)) if isinstance(value, bool) else str(value)
        environment["KERN_STARTUP_PROFILE"] = "1"
        out.mkdir(parents=True, exist_ok=False)
        profile = out / "frames.jsonl"
        command = [str(exe), "play", "--scenario", str(scenario), "--frame-profile", str(profile),
                   "--scenario-output", str(out / "captures"), "--state-out", str(out / "final-state.json")]
        command += control_args
        if script_bytes is not None:
            script_path = out / "debug-ui-script.json"
            script_path.write_bytes(script_bytes)
            command += ["--debug-ui-script", str(script_path)]
            controls["debug_ui_script"]["copied_path"] = str(script_path)
        if not args.visible:
            command += ["--smoke-render", str(frames)]
        elif not args.windowed:
            command += ["--fullscreen"]
        executable_sha256 = hashlib.sha256(exe.read_bytes()).hexdigest()
        manifest = {"schema": 1, "command": command, "cwd": str(engine),
                    "started_utc": datetime.now(timezone.utc).isoformat(),
                    "platform": platform.platform(), "python": sys.version,
                    "executable_sha256": executable_sha256,
                    "shader_provenance": shader_provenance(environment, engine, executable_sha256),
                    "scenario_sha256": hashlib.sha256(case_bytes).hexdigest(),
                    "env_file_sha256": hashlib.sha256(env_bytes).hexdigest() if env_bytes else None,
                    "renderer_environment": {k: v for k, v in sorted(environment.items()) if renderer_key(k)},
                    "environment_overrides": overrides, "git": git_state(engine),
                    "play_controls": controls,
                    "build_ms_external": args.build_ms, "visible": args.visible, "fullscreen": args.visible and not args.windowed,
                    "expected_frames": frames,
                    "scope": "Explicit bounded scenario. No build or existing-process termination. Timeout kills only the spawned child. Selected renderer environment is recorded; other process environment is inherited, not disclosed. Hidden cadence is event-loop throughput, not visible presentation."}
        (out / "scenario.toml").write_bytes(case_bytes)
        if env_bytes:
            (out / "environment.json").write_bytes(env_bytes)
        (out / "run.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        print("Running:", subprocess.list2cmdline(command), flush=True)
        process_start = time.perf_counter()
        with (out / "stdout.log").open("w", encoding="utf-8") as stdout, (out / "stderr.log").open("w", encoding="utf-8") as stderr:
            try:
                run = subprocess.run(command, cwd=engine, env=environment, stdout=stdout, stderr=stderr,
                                     check=False, timeout=args.timeout)
                manifest["exit_code"], manifest["timed_out"] = run.returncode, False
            except subprocess.TimeoutExpired:
                manifest["exit_code"], manifest["timed_out"] = None, True
            except OSError as error:
                manifest["exit_code"], manifest["launch_error"] = None, str(error)
        manifest["process_elapsed_ms"] = (time.perf_counter() - process_start) * 1000
        manifest["finished_utc"] = datetime.now(timezone.utc).isoformat()
        manifest["shader_provenance_after"] = shader_provenance(environment, engine, executable_sha256)
        before, after = manifest["shader_provenance"], manifest["shader_provenance_after"]
        manifest["shader_provenance_unchanged"] = before["content_sha256"] == after["content_sha256"]
        manifest["shader_provenance_stable"] = before["verified"] and after["verified"] and manifest["shader_provenance_unchanged"]
        startup = startup_records(out / "stderr.log")
        (out / "startup.json").write_text(json.dumps(startup, indent=2) + "\n", encoding="utf-8")
        manifest["renderer_startup_records"] = sum(s["kind"] == "renderer_startup" for s in startup)
        manifest["cli_startup_records"] = sum(s["kind"] == "cli_startup" for s in startup)
        if (out / "final-state.json").is_file():
            try:
                final = json.loads((out / "final-state.json").read_text(encoding="utf-8"))
                manifest["startup_report"] = final.get("startup_report")
                manifest["exit_reason"] = final.get("exit_reason")
                manifest["rendered_frames"] = final.get("rendered_frames")
            except ValueError:
                manifest["final_state_error"] = "invalid JSON; raw file retained"
        analysis_start = time.perf_counter()
        manifest["analysis_exit_code"] = subprocess.run([sys.executable, str(Path(__file__).with_name("frame-budget.py")),
            str(profile), "--out", str(out / "budget"), "--exclude-diagnostics"], check=False).returncode if profile.exists() else None
        manifest["analysis_excludes_diagnostics"] = True
        manifest["analysis_elapsed_ms"] = (time.perf_counter() - analysis_start) * 1000
        manifest["completed_expected_frames"] = (manifest.get("rendered_frames") == frames and
            manifest.get("exit_reason") in ("scenario_frame_limit", "smoke_frame_limit"))
        manifest["captures"] = [{"frame_id": frame,
            "image": str(out / "captures" / f"frame-{frame:06}.{capture_format}"),
            "image_exists": (out / "captures" / f"frame-{frame:06}.{capture_format}").is_file(),
            "metadata_exists": (out / "captures" / f"frame-{frame:06}.json").is_file()}
            for frame in case.get("capture_frames", [])]
        manifest["capture_config"] = {"format": capture_format, "max_width": capture_config.get("max_width", 1600), "quality": capture_config.get("quality", 85)}
        manifest["completed_expected_captures"] = all(c["image_exists"] and c["metadata_exists"] for c in manifest["captures"])
        actual_sizes = set()
        for capture in manifest["captures"]:
            capture["metadata_matches_image"] = False
            if capture["image_exists"]:
                capture["actual_capture_size"] = image_size(Path(capture["image"]))
            if capture["metadata_exists"]:
                metadata = json.loads((out / "captures" / f"frame-{capture['frame_id']:06}.json").read_text(encoding="utf-8"))
                capture["source_output_size"] = metadata.get("source_output_size")
                capture["declared_capture_size"] = metadata.get("capture_size")
                capture["declared_capture_format"] = metadata.get("capture_format")
                capture["metadata_matches_image"] = (capture.get("actual_capture_size") is not None and
                    capture["actual_capture_size"] == capture["declared_capture_size"] and
                    capture["declared_capture_format"] in (("jpeg", "jpg") if capture_format == "jpg" else ("png",)))
                if isinstance(capture["source_output_size"], list):
                    actual_sizes.add(tuple(capture["source_output_size"]))
        manifest["completed_expected_captures"] &= all(c["metadata_matches_image"] for c in manifest["captures"])
        if profile.exists():
            with profile.open(encoding="utf-8-sig") as rows:
                for line in rows:
                    try:
                        row = json.loads(line)
                        if row.get("kind") == "gpu_frame" and isinstance(row.get("output_size"), list):
                            actual_sizes.add(tuple(row["output_size"]))
                    except (ValueError, TypeError):
                        continue  # Analyzer separately reports malformed/truncated records.
        requested_size = [int(n) for n in case.get("scene", {}).get("size", "1280x720").split("x")]
        manifest["requested_output_size"] = requested_size
        manifest["actual_output_sizes"] = [list(size) for size in sorted(actual_sizes)]
        manifest["resolution_verified"] = actual_sizes == {tuple(requested_size)}
        manifest["runner_elapsed_ms"] = (time.perf_counter() - runner_start) * 1000
        (out / "run.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        if manifest["exit_code"] != 0:
            print(f"Scenario failed/timeout ({manifest['exit_code']}); partial evidence retained in {out}", file=sys.stderr)
            return 1
        if manifest["analysis_exit_code"] != 0:
            print(f"Scenario exited but usable profile/report missing; see {out}", file=sys.stderr)
            return 2
        if not manifest["completed_expected_frames"] or not manifest["completed_expected_captures"]:
            print(f"Scenario lacks verified frame limit or requested captures; partial report retained in {out}", file=sys.stderr)
            return 1
        if not manifest["resolution_verified"]:
            print(f"Requested resolution not verified: {requested_size} vs {manifest['actual_output_sizes']}; report retained", file=sys.stderr)
            return 1
        print(f"Report: {out / 'budget' / 'report.html'}\nImages: {out / 'captures'}")
        return 0
    except (OSError, ValueError) as error:
        print(f"run-frame-scenario: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
