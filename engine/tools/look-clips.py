#!/usr/bin/env python3
"""Short clips of a camera moving at things, so pops and transitions show.

    look-clips.py after --exe target/release/examples/terrain_fly.exe

Reads "clips" from tools/look-suite.json, dumps frames headless with terrain_fly
--dump and encodes build/render-suite/LABEL/clips/NAME.mp4 (960 px, 30 fps, H.264)
with ffmpeg. look-report.py copies them into its report.
"""
import argparse
import json
import os
import platform
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("label")
    parser.add_argument("--exe", default=str(ROOT / "target/release/examples/terrain_fly"))
    parser.add_argument("--suite", default=str(ROOT / "engine/tools/look-suite.json"))
    parser.add_argument("--only", help="comma-separated clip names")
    args = parser.parse_args()
    suite = json.loads(Path(args.suite).read_text())
    out = ROOT / "build" / "render-suite" / args.label / "clips"
    out.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ)
    if "microsoft" in platform.release().lower():
        env["WSLENV"] = ":".join(filter(None, [env.get("WSLENV", ""), "WGPU_BACKEND", "KERN_ADAPTER", "KERN_DLSS_RUNTIME_DIR"]))
    for clip in suite.get("clips", []):
        if args.only and clip["name"] not in args.only.split(","):
            continue
        frames = out / clip["name"]
        shutil.rmtree(frames, ignore_errors=True)
        relative = frames.relative_to(ROOT).as_posix()
        command = [str(Path(args.exe).resolve()), *clip.get("world", suite["world"]), "--no-hud", "--at", clip["at"], "--speed", str(clip["speed"]), "--size", "960x540",
                   "--dump", str(clip["seconds"] * 60), relative, "--dump-warm", "150", "--dump-every", "2", "--dump-forward"]
        done = subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True, errors="replace")
        if done.returncode:
            raise SystemExit(f"{clip['name']} failed ({done.returncode})\n{done.stderr[-800:]}")
        target = out / f"{clip['name']}.mp4"
        subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-framerate", "30", "-i", str(frames / "%03d.png"), "-c:v", "libx264", "-crf", "27", "-preset", "slow",
                        "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(target)], check=True)
        shutil.rmtree(frames)
        print(f"{target}: {target.stat().st_size / 2**20:.1f} MiB", flush=True)


if __name__ == "__main__":
    main()
