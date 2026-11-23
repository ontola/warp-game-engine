"""Package the two existing Godot HTML exports under the lobby's godot/ path."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import re
import shutil
import tempfile

RUNTIME = ("index.js", "index.wasm", "index.audio.worklet.js", "index.audio.position.worklet.js")
CONFIG = re.compile(r"^const GODOT_CONFIG = (.+);$", re.MULTILINE)


def install_game(source, name, root):
    page = (source / "index.html").read_text(encoding="utf-8")
    config_match = CONFIG.search(page)
    if not config_match or page.count('src="index.js"') != 1:
        raise ValueError(f"{name}: unexpected Godot HTML shell")
    config = json.loads(config_match[1])
    if config["executable"] != "index" or (source / "index.pck").stat().st_size != config["fileSizes"]["index.pck"]:
        raise ValueError(f"{name}: unexpected Godot export files")

    digest = hashlib.sha256()
    for filename in RUNTIME:
        with (source / filename).open("rb") as data:
            digest.update(hashlib.file_digest(data, "sha256").digest())
    fingerprint = digest.hexdigest()[:24]
    shared = root / "runtime" / fingerprint
    if not shared.exists():
        shared.mkdir(parents=True)
        for filename in RUNTIME:
            shutil.copyfile(source / filename, shared / filename)
        # Static hosts can serve this sidecar with Content-Encoding: gzip.
        wasm = shared / "index.wasm"
        (shared / "index.wasm.gz").write_bytes(gzip.compress(wasm.read_bytes(), compresslevel=6, mtime=0))

    game = root / name
    game.mkdir()
    for item in source.iterdir():
        if item.name == "index.html" or item.name in RUNTIME:
            continue
        if item.is_dir():
            shutil.copytree(item, game / item.name)
        else:
            shutil.copyfile(item, game / item.name)

    # Godot preloads mainPack into its virtual filesystem using the same name.
    # Keep it local to each game and use one cache key for the engine runtime.
    engine = f"../runtime/{fingerprint}/index"
    config["mainPack"] = "index.pck"
    config["executable"] = engine
    config["fileSizes"][engine + ".wasm"] = config["fileSizes"].pop("index.wasm")
    page = page[:config_match.start(1)] + json.dumps(config, separators=(",", ":")) + page[config_match.end(1):]
    (game / "index.html").write_text(page.replace('src="index.js"', f'src="{engine}.js"'), encoding="utf-8")
    return fingerprint


def pack_games(exports, dest, *, refresh=False):
    dest = dest.resolve()
    if dest.exists() and not refresh:
        raise SystemExit(f"Choose a new output directory: {dest}")
    dest.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=dest.parent, prefix=".godot-build-") as scratch:
        stage = Path(scratch) / "godot"
        stage.mkdir()
        for name, source in exports:
            fingerprint = install_game(source, name, stage)
            print(f"{name}/index.html  runtime/{fingerprint}")
        if not dest.exists():
            stage.rename(dest)
            return
        # Refresh only generated files in the staging tree. Retain other games
        # and immutable runtime URLs that existing pages may still reference.
        for file in stage.rglob("*"):
            if file.is_file():
                target = dest / file.relative_to(stage)
                target.parent.mkdir(parents=True, exist_ok=True)
                file.replace(target)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ballkickers", type=Path, required=True)
    parser.add_argument("--frog-fighter", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="New godot/ directory inside the lobby site")
    args = parser.parse_args()
    pack_games([("ballkickers", args.ballkickers), ("frog-fighter", args.frog_fighter)], args.output)


if __name__ == "__main__":
    main()
