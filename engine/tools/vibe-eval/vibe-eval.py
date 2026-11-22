#!/usr/bin/env python3
"""Vibecode eval: how well can a model make a Warp game from one prompt?

  vibe-eval.py score GAME...             score games that already exist
  vibe-eval.py make --agent CMD [--only ID,ID] [--out DIR]
                                         ask an agent for every prompt, then score

`make` creates OUT/ID/ for each prompt in prompts.json, writes the prompt to
OUT/ID/PROMPT.md (with the rules in BRIEF below) and runs CMD in that folder
with {prompt} replaced by the prompt file's path and {dir} by the folder. The
agent must leave a game there: main.luau (a folder game) or game.luau.
Example: --agent 'claude -p "$(cat {prompt})" --permission-mode acceptEdits'

Scoring needs no GPU (warp check). Per game:
  loads     the scripts load without errors
  runs      30 s with three bots and no runtime error
  lives     the game's state changes between 5 s and 10 s (bots play)
  finishes  a round ends (state shows a results phase or game.finish ran) in 3 min
  kit       it uses the game kit (require("kit/...")), lines of Luau
Writes OUT/results.json and OUT/report.md. Engine docs: engine/docs/kit.md.
"""
import json, os, re, shlex, subprocess, sys, pathlib

HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parents[2]
WARP = os.environ.get("WARP_BIN") or str(REPO / "target/release/warp")

BRIEF = """Make this game with the Warp engine's `warp` executable: {prompt}

Rules: write Luau only, in this folder (main.luau for a folder game, or a
single game.luau). Read these first: {repo}/engine/docs/kit.md (the game kit:
controllers, cameras, HUD, round flow, bots), {repo}/engine/crates/warp/starter/types.d.luau
(the whole API) and {repo}/engine/docs/warp-cli.md. Templates: `{warp} templates`.
Check your work with `{warp} check . --ticks 1800 --bots 3` (no GPU) until it
shows no errors. It must work with bots filling empty seats, for 1 to 4 people
in split screen, and end a round with game.finish().
"""


def check(game, ticks, extra=()):
    cmd = [WARP, "check", str(game), "--ticks", str(ticks), *extra]
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
    out = p.stdout
    i = out.find("{")
    try:
        j = json.loads(out[i:]) if i >= 0 else None
    except json.JSONDecodeError:
        j = None
    return p.returncode, j, (p.stdout + p.stderr)[-2000:]


def luau_files(game):
    g = pathlib.Path(game)
    if g.is_file():
        return [g]
    return [p for p in g.rglob("*.luau") if "sounds" not in p.parts and not p.name.endswith(".d.luau")]


def finished(j):
    s = json.dumps(j.get("state", {})) if j else ""
    return bool(re.search(r'"phase": "(results|done|over|finished)"', s)) or bool(re.search(r'"(winner|winners)": (?!null|\[\]|\{\})', s))


def score(game):
    game = pathlib.Path(game)
    r = {"game": str(game)}
    files = luau_files(game)
    text = "\n".join(f.read_text(errors="replace") for f in files)
    r["lines"] = sum(1 for l in text.splitlines() if l.strip() and not l.strip().startswith("--"))
    r["kit"] = sorted(set(re.findall(r'require\("(kit/[a-z_]+)"\)', text)))
    code, j, log = check(game, 2, ["--bots", "3"])
    r["loads"] = code == 0 and j is not None and j.get("errors") == []
    if not r["loads"]:
        r["error"] = (j or {}).get("errors") or log
        r["score"] = 0
        return r
    _, j5, _ = check(game, 300, ["--bots", "3"])
    _, j10, _ = check(game, 600, ["--bots", "3"])
    code, j30, log = check(game, 1800, ["--bots", "3"])
    r["runs"] = code == 0 and j30 is not None and j30.get("errors") == []
    if not r["runs"]:
        r["error"] = (j30 or {}).get("errors") or log
    r["lives"] = bool(j5 and j10 and json.dumps(j5.get("state")) != json.dumps(j10.get("state")))
    _, j180, _ = check(game, 180 * 60, ["--bots", "3"])
    r["finishes"] = bool(j180 and finished(j180))
    r["entities"] = (j30 or {}).get("entities", 0)
    r["score"] = sum([r["loads"], r["runs"], r["lives"], r["finishes"]])
    return r


def report(results, out):
    out.mkdir(parents=True, exist_ok=True)
    (out / "results.json").write_text(json.dumps(results, indent=2))
    rows = ["| Game | Score /4 | Loads | Runs 30 s | Lives | Finishes | Kit modules | Lines |", "| --- | --- | --- | --- | --- | --- | --- | --- |"]
    yes = lambda b: "yes" if b else "no"
    for r in results:
        rows.append(f"| {pathlib.Path(r['game']).name} | {r['score']} | {yes(r.get('loads'))} | {yes(r.get('runs'))} | {yes(r.get('lives'))} | {yes(r.get('finishes'))} | {', '.join(m[4:] for m in r.get('kit', [])) or '-'} | {r.get('lines', 0)} |")
    total = sum(r["score"] for r in results)
    rows.append(f"\nTotal {total} of {4 * len(results)}.")
    errors = [f"- {pathlib.Path(r['game']).name}: {str(r['error'])[:300]}" for r in results if r.get("error")]
    if errors:
        rows.append("\nErrors:\n" + "\n".join(errors))
    (out / "report.md").write_text("\n".join(rows) + "\n")
    print("\n".join(rows))


def main(argv):
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return 0
    if not pathlib.Path(WARP).exists():
        print(f"no warp executable at {WARP}: cargo build --release -p warp (or set WARP_BIN)")
        return 1
    if argv[0] == "score":
        games = argv[1:]
        report([score(g) for g in games], pathlib.Path("build/vibe-eval"))
        return 0
    if argv[0] == "make":
        args = dict(zip(argv[1::2], argv[2::2]))
        agent = args.get("--agent")
        if not agent:
            print("make needs --agent CMD")
            return 1
        out = pathlib.Path(args.get("--out", "build/vibe-eval")).resolve()
        only = set(args["--only"].split(",")) if "--only" in args else None
        prompts = [p for p in json.loads((HERE / "prompts.json").read_text()) if not only or p["id"] in only]
        results = []
        for p in prompts:
            d = out / p["id"]
            d.mkdir(parents=True, exist_ok=True)
            brief = d / "PROMPT.md"
            brief.write_text(BRIEF.format(prompt=p["prompt"], repo=REPO, warp=WARP))
            cmd = agent.replace("{prompt}", shlex.quote(str(brief))).replace("{dir}", shlex.quote(str(d)))
            print(f"== {p['id']}: {cmd}", flush=True)
            subprocess.run(cmd, shell=True, cwd=d)
            game = d / "game.luau" if (d / "game.luau").exists() else d
            r = score(game)
            r["id"], r["genre"] = p["id"], p["genre"]
            results.append(r)
        report(results, out)
        return 0
    print(__doc__)
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
