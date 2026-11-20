"""Room-bound feedback, checkpoints, and the local Warp MCP connection."""
import json
import sqlite3
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import urlsplit


def timestamp():
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


class LiveClient:
    def __init__(self, token_file):
        self.token_file = Path(token_file)

    def call(self, name, args):
        info = json.loads(self.token_file.read_text(encoding="utf-8-sig"))
        if urlsplit(info["url"]).hostname not in ("127.0.0.1", "localhost", "::1"):
            raise ValueError("Expected a local Warp instance.")
        data = {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": name, "arguments": args}}
        req = urllib.request.Request(info["url"], data=json.dumps(data).encode(),
                                     headers={"Content-Type": "application/json",
                                              "Authorization": "Bearer "+info["token"]})
        with urllib.request.urlopen(req, timeout=3) as res:
            reply = json.load(res)
        if "error" in reply:
            raise RuntimeError(reply["error"].get("message", "Warp request failed"))
        result = reply["result"]
        message = "\n".join(p.get("text", "") for p in result["content"] if p["type"]=="text")
        if result.get("isError"):
            raise RuntimeError(message)
        return message

    def eval(self, code):
        value = self.call("run_luau", {"code": code})
        try:
            return json.loads(value)
        except json.JSONDecodeError:
            return value

    def review(self, command, room=None):
        args = json.dumps(command) + (f",{int(room)}" if room is not None else "")
        data = self.eval(f"return debug.playtest({args})")
        if not isinstance(data, dict) or "active" not in data:
            raise RuntimeError(f"Playtest command failed: {str(data)[:250]}")
        return data


def connect_log(log_dir):
    directory = Path(log_dir)
    directory.mkdir(parents=True, exist_ok=True)
    db = sqlite3.connect(directory/"session.sqlite3", timeout=5)
    db.row_factory = sqlite3.Row
    db.executescript("""
        CREATE TABLE IF NOT EXISTS checkpoint(
            id INTEGER PRIMARY KEY CHECK(id=1), room INTEGER NOT NULL, finished INTEGER NOT NULL);
        INSERT OR IGNORE INTO checkpoint VALUES(1,1,0);
        CREATE TABLE IF NOT EXISTS notes(
            id INTEGER PRIMARY KEY, room INTEGER NOT NULL, name TEXT NOT NULL,
            kind TEXT NOT NULL, text TEXT NOT NULL, created TEXT NOT NULL, falls INTEGER NOT NULL,
            cleared INTEGER NOT NULL, response TEXT, handled TEXT);
        CREATE TABLE IF NOT EXISTS advance(
            id INTEGER PRIMARY KEY CHECK(id=1), source INTEGER NOT NULL, target INTEGER NOT NULL,
            finish INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS visits(
            id INTEGER PRIMARY KEY, room INTEGER NOT NULL, name TEXT NOT NULL,
            outcome TEXT NOT NULL, created TEXT NOT NULL);
    """)
    return db


class PlaytestSession:
    def __init__(self, client, log_dir):
        self.client = client
        self.db = connect_log(log_dir)
        self.displayed = None

    def _finish_advance(self, live):
        pending = self.db.execute("SELECT source,target,finish FROM advance").fetchone()
        if not pending:
            return None
        source, target, finish = pending
        if not live["active"] or (live["finished"] and live["room"]==source and target!=source):
            live = self.client.review("begin", target)
        # Repeat delivery after an interrupted request without moving twice.
        if live["room"] == source and not live["finished"]:
            live = self.client.eval(f'local s=debug.playtest("status"); if s.room=={source} and not s.finished then return debug.playtest("next") end; return s')
        if live["room"] != target or bool(live["finished"]) != bool(finish):
            raise RuntimeError("Warp and the review checkpoint disagree. Reconnect the desk.")
        with self.db:
            self.db.execute("DELETE FROM advance WHERE source=? AND target=?", (source,target))
        return self._display(live)

    def start(self):
        return self.status()

    def _display(self, live):
        if not live["active"]:
            raise RuntimeError("Review session disconnected. Reopen the playtest companion.")
        reviewed, = self.db.execute(
            "SELECT count(*) FROM (SELECT room FROM notes UNION SELECT room FROM visits WHERE outcome IN ('cleared','skipped'))"
        ).fetchone()
        latest = self.db.execute(
            "SELECT room,name,response FROM notes WHERE handled IS NOT NULL ORDER BY handled DESC,id DESC LIMIT 1"
        ).fetchone()
        live["reviewed"] = reviewed
        live["update"] = f"Room {latest['room']} · {latest['name']} — {latest['response']}" if latest else ""
        self.displayed = live.copy()
        return live

    def status(self):
        live = self.client.review("status")
        saved = self.db.execute("SELECT room,finished FROM checkpoint").fetchone()
        if saved["finished"] and saved["room"] < live["total"]:
            source = saved["room"]
            target = source + 1
            with self.db:
                changed = self.db.execute(
                    "UPDATE checkpoint SET room=?,finished=0 WHERE room=? AND finished=1", (target,source)
                ).rowcount
                if changed:
                    # An undelivered finish for the former last room is now an advance.
                    self.db.execute("INSERT OR REPLACE INTO advance VALUES(1,?,?,0)", (source,target))
        recovered = self._finish_advance(live)
        if recovered:
            return recovered
        if not live["active"]:
            # A newly prepared GameNight match installs a fresh VM.
            saved = self.db.execute("SELECT room,finished FROM checkpoint").fetchone()
            live = self.client.review("begin", min(saved["room"],live["total"]))
            if saved["finished"]:
                live = self.client.review("next")
        return self._display(live)

    def _advance(self, previous, good=False):
        n = previous["room"]
        finish = int(n==previous["total"])
        target = n if finish else n+1
        with self.db:
            self.db.execute("BEGIN IMMEDIATE")
            saved = self.db.execute("SELECT room,finished FROM checkpoint").fetchone()
            if saved["room"] == n and not saved["finished"]:
                if good:
                    self.db.execute(
                        "INSERT INTO notes(room,name,kind,text,created,falls,cleared) VALUES(?,?,?,?,?,?,1)",
                        (n,previous["name"],"good","Both players cleared the room; marked good on the controller.",
                         timestamp(),previous["falls"]))
                self.db.execute("INSERT INTO visits(room,name,outcome,created) VALUES(?,?,?,?)",
                                (n,previous["name"],"cleared" if previous["cleared"] else "skipped",timestamp()))
                self.db.execute("UPDATE checkpoint SET room=?,finished=? WHERE id=1",(target,finish))
                self.db.execute("INSERT INTO advance VALUES(1,?,?,?)",(n,target,finish))
        return self.status()

    def approve(self):
        live=self.client.review("status")
        if live.get("approve") and live["cleared"] and not live["finished"]:
            return self._advance(live,good=True)
        return self._display(live)

    def action(self, command):
        if command not in ("pause","resume","retry","next"):
            raise ValueError("Unknown review command.")
        previous = self.client.review("status")
        if self.displayed and previous["room"] != self.displayed["room"]:
            raise RuntimeError("The room changed. Check the displayed room and try again.")
        if previous["finished"] and command == "next":
            return self._display(previous)
        if command == "next":
            return self._advance(previous)
        live = self.client.review(command)
        with self.db:
            if command == "retry":
                result = "retry"
                self.db.execute("INSERT INTO visits(room,name,outcome,created) VALUES(?,?,?,?)",
                                (previous["room"],previous["name"],result,timestamp()))
            self.db.execute("UPDATE checkpoint SET room=?,finished=? WHERE id=1",
                            (live["room"],int(live["finished"])))
        return self._display(live)

    def note(self, kind, text):
        if kind not in ("good","issue","blocked"):
            raise ValueError("Choose a feedback disposition.")
        current = self.client.review("status")
        if self.displayed and current["room"] != self.displayed["room"]:
            raise RuntimeError("The room changed. Confirm the room before saving.")
        with self.db:
            self.db.execute("INSERT INTO notes(room,name,kind,text,created,falls,cleared) VALUES(?,?,?,?,?,?,?)",
                            (current["room"],current["name"],kind,text.strip(),timestamp(),
                             current["falls"],int(current["cleared"])))

    def last_note(self):
        row = self.db.execute("SELECT * FROM notes ORDER BY id DESC LIMIT 1").fetchone()
        return dict(row) if row else None

    def close(self):
        self.db.close()
