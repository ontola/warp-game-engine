#!/usr/bin/env python3
"""Vibe studio: a small HTTP service that turns a prompt into a tested Warp game.

The Splitscreen Club cloud calls it while a couch is playing: a phone sends
"a game where ...", the studio has an agent (Claude Code by default) write
the game in Luau with the game kit, plays it headless with bots, sends the
errors back to the agent until it passes, and hands over the game's files.
A later prompt ("higher jumps") changes that game into a new version.

  vibe-studio.py serve [--port 8795] [--data DIR] [--workers 2]
  vibe-studio.py make "a game where ..." [--template NAME]   one game, no server

API (JSON; with VIBE_STUDIO_TOKEN set, every call needs
`Authorization: Bearer TOKEN`):

  POST /v1/games              {"prompt", "room"?, "template"?}  -> a job
  POST /v1/games/ID/revise    {"prompt"}                        -> the same job, next version
  GET  /v1/games/ID                                             -> the job
  GET  /v1/games/ID/files[?version=N]                           -> {"title", "version", "files": {path: text}}

A job: {"id", "status", "stage", "version", "title", "score", "error",
"attempts", "updated"}. status is queued, working, ready, failed or
refused. stage says what happens now, for the TV: "writing the game",
"playtesting with bots", "fixing: <first error>". A revise keeps the last
ready version playable until the new one is ready.

Limits: prompts up to 600 characters; per room at most VIBE_STUDIO_ROOM_LIMIT
(default 6) new games or revisions an hour; an agent run gets
VIBE_STUDIO_AGENT_TIMEOUT seconds (default 900); up to 2 fix rounds.

The game passes when vibe-eval scores it 4 of 4 (loads, runs 30 s with
three bots without errors, its state changes, a round ends within three
minutes). Needs the `warp` executable (WARP_BIN, else target/release/warp)
and, for the default agent, `claude` on PATH. VIBE_STUDIO_AGENT replaces
the agent command: {prompt} is the brief's path, {dir} the game folder,
{tools} the allowed tools.
"""
import importlib.util, json, os, pathlib, re, shlex, shutil, subprocess, sys, threading, time, uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from queue import Queue
from urllib.parse import urlparse, parse_qs

HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parents[2]
_spec = importlib.util.spec_from_file_location("vibe_eval", HERE.parent / "vibe-eval" / "vibe-eval.py")
vibe_eval = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(vibe_eval)
WARP = vibe_eval.WARP

MAX_PROMPT = 600
ROOM_LIMIT = int(os.environ.get("VIBE_STUDIO_ROOM_LIMIT", "6"))
AGENT_TIMEOUT = int(os.environ.get("VIBE_STUDIO_AGENT_TIMEOUT", "900"))
FIX_ROUNDS = 2
TOOLS = ["Read", "Write", "Edit", "Glob", "Grep", f"Bash({WARP} check:*)", f"Bash({WARP} script:*)", f"Bash({WARP} templates)"]
AGENT = os.environ.get("VIBE_STUDIO_AGENT") or (
    'claude -p "$(cat {prompt})" --model sonnet --permission-mode acceptEdits --allowedTools {tools}')
# Not part of a game handed over: the agent's notes, the docs copied in, renders.
SKIP = {"PROMPT.md", "KIT.md", "README.md", "WARP-CLI.md", "types.d.luau", "REFUSED.md", "agent.log", "look.json"}
SKIP_DIRS = {"shots", "scripts", ".claude", "docs"}

RULES = """Rules: write Luau only, in this folder; main.luau is the game and
game.toml its settings (title, players). Read KIT.md first (the game kit:
controllers, cameras, HUD, round flow, bots, weapons), types.d.luau (the
whole API) and WARP-CLI.md. Use the kit's controllers, camera rigs, ui and
round instead of writing your own.

It is played on a TV by 1 to 4 people on one couch, in split screen, with
bots filling empty seats: make it fun at once, readable from the sofa
(bright sky or lights, big HUD), and end a round with game.finish() after
a normal round (two or three minutes, or a score).

Check it with `{warp} check . --ticks 1800 --bots 3` until it shows no
errors. Look at it: `{warp} script . look.json --out shots` with look.json
`{{"size": [960, 540], "steps": [{{"ticks": 600}}, {{"capture": "a.png"}}]}}`
renders a frame; read the PNG and fix what looks wrong (dark, camera inside
something, unreadable text).

Refuse, by writing only REFUSED.md with one short reason, when the request
is not fit for a family couch (sexual content, real-world hate, gore for
its own sake) or asks to copy a specific existing game, its characters or
its look by name. A game in the same genre is fine: make it your own.
"""

NEW = "Make this couch game with the Warp engine: {prompt}\n\nGive it a short fun title in game.toml.\n\n" + RULES
REVISE = ("This folder holds a Warp couch game the players are playing. They ask: {prompt}\n\n"
          "Change the game that way and keep everything else working.\n\n" + RULES)
FIX = ("The game in this folder failed its playtest (three bots, headless):\n\n{problems}\n\n"
       "Fix that, then check it again as below.\n\n" + RULES)


def pick_template(prompt):
    p = prompt.lower()
    if re.search(r"\b(fps|first[- ]person|shoot(er|ing)?|deathmatch|frag)\b", p):
        return "fps"
    if re.search(r"\b(platform(er)?|jump and run|side[- ]?scroll|ledges?)\b", p):
        return "platformer"
    return None


def problems(r):
    out = []
    if not r.get("loads"):
        out.append(f"It does not load: {str(r.get('error'))[:800]}")
    elif not r.get("runs"):
        out.append(f"It stops with an error within 30 s: {str(r.get('error'))[:800]}")
    if r.get("loads") and not r.get("lives"):
        out.append("Nothing changes between 5 s and 10 s: the bots do not play (give bots inputs, kit/bot).")
    if r.get("loads") and not r.get("finishes"):
        out.append("No round ends within three minutes: use kit/round (length or win_score) so game.finish() runs.")
    return "\n".join(f"- {x}" for x in out)


def bundle(d):
    files = {}
    for p in sorted(d.rglob("*")):
        rel = p.relative_to(d)
        if not p.is_file() or rel.name in SKIP or set(rel.parts[:-1]) & SKIP_DIRS or p.suffix not in (".luau", ".toml", ".json"):
            continue
        files[rel.as_posix()] = p.read_text(errors="replace")
    return files


def title_of(d):
    m = re.search(r'^title\s*=\s*"([^"]*)"', (d / "game.toml").read_text(errors="replace"), re.M) if (d / "game.toml").exists() else None
    return m.group(1) if m else "Untitled"


class Studio:
    def __init__(self, data, workers=2):
        self.data = pathlib.Path(data).resolve()
        self.data.mkdir(parents=True, exist_ok=True)
        self.lock = threading.Lock()
        self.jobs = {}
        self.room_log = {}
        self.queue = Queue()
        for f in self.data.glob("*/job.json"):
            j = json.loads(f.read_text())
            if j["status"] in ("queued", "working"):  # the server stopped mid-run
                j.update(status="failed" if not j.get("ready_version") else "ready", stage="", error="the studio restarted")
            self.jobs[j["id"]] = j
        for _ in range(workers):
            threading.Thread(target=self._work, daemon=True).start()

    # -- jobs
    def _save(self, j):
        j["updated"] = time.time()
        (self.data / j["id"]).mkdir(exist_ok=True)
        (self.data / j["id"] / "job.json").write_text(json.dumps(j, indent=1))

    def _set(self, j, **kv):
        with self.lock:
            j.update(kv)
            self._save(j)

    def view(self, j):
        keys = ("id", "status", "stage", "version", "ready_version", "title", "score", "error", "attempts", "room", "updated")
        return {k: j.get(k) for k in keys}

    def _allow(self, room):
        now = time.time()
        log = [t for t in self.room_log.get(room, []) if now - t < 3600]
        if len(log) >= ROOM_LIMIT:
            return False
        self.room_log[room] = log + [now]
        return True

    def create(self, prompt, room="", template=None):
        prompt = (prompt or "").strip()
        if not prompt or len(prompt) > MAX_PROMPT:
            raise ValueError(f"a prompt of 1 to {MAX_PROMPT} characters")
        if template is not None and template not in ("fps", "platformer", ""):
            raise ValueError("unknown template")
        with self.lock:
            if not self._allow(room):
                raise PermissionError(f"this room made {ROOM_LIMIT} games this hour; try again later")
            j = {"id": uuid.uuid4().hex[:12], "room": room, "status": "queued", "stage": "waiting for a free studio",
                 "version": 1, "ready_version": None, "title": None, "score": None, "error": None, "attempts": 0,
                 "prompts": [prompt], "template": template if template is not None else pick_template(prompt), "created": time.time()}
            self.jobs[j["id"]] = j
            self._save(j)
        self.queue.put((j["id"], 1))
        return j

    def revise(self, id, prompt):
        prompt = (prompt or "").strip()
        if not prompt or len(prompt) > MAX_PROMPT:
            raise ValueError(f"a prompt of 1 to {MAX_PROMPT} characters")
        with self.lock:
            j = self.jobs.get(id)
            if j is None:
                raise KeyError(id)
            if j["status"] in ("queued", "working"):
                raise RuntimeError("this game is still being made")
            if not j.get("ready_version"):
                raise RuntimeError("this game has no playable version to change")
            if not self._allow(j["room"]):
                raise PermissionError(f"this room made {ROOM_LIMIT} games this hour; try again later")
            j["prompts"].append(prompt)
            j.update(status="queued", stage="waiting for a free studio", version=j["ready_version"] + 1, error=None, attempts=0)
            self._save(j)
        self.queue.put((id, j["version"]))
        return j

    def files(self, id, version=None):
        j = self.jobs.get(id)
        if j is None or not j.get("ready_version"):
            return None
        v = version or j["ready_version"]
        d = self.data / id / f"v{v}"
        if not d.is_dir() or v > j["ready_version"]:
            return None
        return {"id": id, "title": title_of(d), "version": v, "files": bundle(d)}

    # -- making a game
    def _work(self):
        while True:
            id, version = self.queue.get()
            j = self.jobs[id]
            try:
                self._make(j, version)
            except Exception as e:  # keep the worker alive; the job says what broke
                self._set(j, status="ready" if j.get("ready_version") else "failed", stage="", error=f"studio error: {e}")

    def _agent(self, d, brief):
        (d / "PROMPT.md").write_text(brief)
        cmd = AGENT.replace("{prompt}", shlex.quote(str(d / "PROMPT.md"))).replace("{dir}", shlex.quote(str(d))).replace(
            "{tools}", " ".join(shlex.quote(t) for t in TOOLS))
        with open(d / "agent.log", "a") as log:
            log.write(f"\n== {time.strftime('%H:%M:%S')} {cmd}\n")
            log.flush()
            try:
                subprocess.run(cmd, shell=True, cwd=d, stdout=log, stderr=subprocess.STDOUT, timeout=AGENT_TIMEOUT)
            except subprocess.TimeoutExpired:
                log.write("\n== timed out\n")

    def _make(self, j, version):
        d = self.data / j["id"] / f"v{version}"
        if d.exists():
            shutil.rmtree(d)
        if version == 1:
            cmd = [WARP, "new", str(d), "--title", "New game"] + (["--template", j["template"]] if j.get("template") else [])
            subprocess.run(cmd, check=True, capture_output=True, text=True)
            brief = NEW.format(prompt=j["prompts"][-1], warp=WARP)
        else:
            shutil.copytree(self.data / j["id"] / f"v{j['ready_version']}", d,
                            ignore=shutil.ignore_patterns("shots", "agent.log", "PROMPT.md", ".claude"))
            brief = REVISE.format(prompt=j["prompts"][-1], warp=WARP)
        shutil.copy(REPO / "engine/docs/kit.md", d / "KIT.md")
        shutil.copy(REPO / "engine/docs/warp-cli.md", d / "WARP-CLI.md")
        r = None
        for attempt in range(1 + FIX_ROUNDS):
            self._set(j, status="working", attempts=attempt + 1,
                      stage="writing the game" if attempt == 0 else f"fixing: {problems(r).splitlines()[0][2:120]}")
            self._agent(d, brief)
            if (d / "REFUSED.md").exists():
                reason = (d / "REFUSED.md").read_text(errors="replace").strip()[:300]
                self._set(j, status="ready" if j.get("ready_version") else "refused", stage="", error=f"refused: {reason}")
                return
            self._set(j, stage="playtesting with bots")
            r = vibe_eval.score(d)
            if r["score"] == 4:
                self._set(j, status="ready", stage="", ready_version=version, title=title_of(d), score=r, error=None)
                return
            brief = FIX.format(problems=problems(r), warp=WARP)
        self._set(j, status="ready" if j.get("ready_version") else "failed", stage="", score=r,
                  error=f"failed its playtest: {problems(r)[:400]}")


class Handler(BaseHTTPRequestHandler):
    studio: Studio = None
    token = os.environ.get("VIBE_STUDIO_TOKEN", "")

    def _send(self, code, body):
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _authorized(self):
        if self.token and self.headers.get("Authorization", "") != f"Bearer {self.token}":
            self._send(401, {"error": "unauthorized"})
            return False
        return True

    def _body(self):
        n = int(self.headers.get("Content-Length") or 0)
        if n > 16384:
            raise ValueError("body too large")
        return json.loads(self.rfile.read(n) or b"{}")

    def do_GET(self):
        if not self._authorized():
            return
        u = urlparse(self.path)
        parts = u.path.strip("/").split("/")
        if parts[:2] != ["v1", "games"] or len(parts) not in (3, 4):
            return self._send(404, {"error": "not found"})
        j = self.studio.jobs.get(parts[2])
        if j is None:
            return self._send(404, {"error": "no such game"})
        if len(parts) == 3:
            return self._send(200, self.studio.view(j))
        if parts[3] == "files":
            v = parse_qs(u.query).get("version")
            f = self.studio.files(parts[2], int(v[0]) if v else None)
            return self._send(200, f) if f else self._send(409, {"error": "no playable version yet", "status": j["status"]})
        self._send(404, {"error": "not found"})

    def do_POST(self):
        if not self._authorized():
            return
        parts = urlparse(self.path).path.strip("/").split("/")
        try:
            body = self._body()
            if parts == ["v1", "games"]:
                j = self.studio.create(body.get("prompt"), str(body.get("room", ""))[:64], body.get("template"))
                return self._send(202, self.studio.view(j))
            if len(parts) == 4 and parts[:2] == ["v1", "games"] and parts[3] == "revise":
                return self._send(202, self.studio.view(self.studio.revise(parts[2], body.get("prompt"))))
            self._send(404, {"error": "not found"})
        except (ValueError, json.JSONDecodeError) as e:
            self._send(400, {"error": str(e)})
        except PermissionError as e:
            self._send(429, {"error": str(e)})
        except KeyError:
            self._send(404, {"error": "no such game"})
        except RuntimeError as e:
            self._send(409, {"error": str(e)})

    def log_message(self, fmt, *args):
        sys.stderr.write(f"{self.address_string()} {fmt % args}\n")


def main(argv):
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return 0
    if not pathlib.Path(WARP).exists():
        print(f"no warp executable at {WARP}: cargo build --release -p warp (or set WARP_BIN)")
        return 1
    args = dict(zip(argv[1::2], argv[2::2])) if argv[0] == "serve" else dict(zip(argv[2::2], argv[3::2]))
    data = args.get("--data", "build/vibe-studio")
    if argv[0] == "serve":
        Handler.studio = Studio(data, int(args.get("--workers", "2")))
        port = int(args.get("--port", "8795"))
        print(f"vibe studio on http://127.0.0.1:{port} (data {Handler.studio.data})", flush=True)
        ThreadingHTTPServer(("0.0.0.0", port), Handler).serve_forever()
        return 0
    if argv[0] == "make" and len(argv) > 1:
        studio = Studio(data, 1)
        j = studio.create(argv[1], "cli", args.get("--template"))
        last = None
        while j["status"] in ("queued", "working"):
            if j["stage"] != last:
                print(f"{time.strftime('%H:%M:%S')} {j['stage']}", flush=True)
                last = j["stage"]
            time.sleep(1)
        print(json.dumps(studio.view(j), indent=1))
        print(f"game: {studio.data / j['id'] / ('v' + str(j['ready_version']))}" if j.get("ready_version") else "")
        return 0 if j["status"] == "ready" else 1
    print(__doc__)
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
