"""Launch the two-player desk, or read its durable feedback queue from this chat."""
import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
from contextlib import closing
from pathlib import Path

from playtest_session import LiveClient, PlaytestSession, connect_log, timestamp

GAME = Path(__file__).resolve().parents[1]
LOCAL = Path(os.environ.get("LOCALAPPDATA", Path.home()))
SESSION = LOCAL / "Relay Keepers" / "Playtest"
STANDALONE_TOKEN = SESSION / "standalone-live.json"


def live_client(token=None):
    if token is not None:
        candidates = [Path(token)]
    else:
        discovery = Path(os.environ.get("WARP_LIVE_DIR", Path(tempfile.gettempdir())/"warp-live"))
        candidates = [discovery/"game.json", discovery/"relay-keepers.json", STANDALONE_TOKEN]
    for file in candidates:
        if not file.is_file():
            continue
        try:
            client = LiveClient(file)
            if json.loads(client.call("query", {"pointer":"/title"})).get("/title") == "Relay Keepers":
                return client
        except (OSError, ValueError, RuntimeError):
            continue
    raise RuntimeError("Waiting for Relay Keepers. Join two players in GameNight and launch the game.")


def panel(args):
    import tkinter as tk
    from tkinter import ttk
    from playtest_panel import run_panel

    root = tk.Tk()
    root.title("Relay Keepers · Connect playtest")
    frame = ttk.Frame(root,padding=18)
    frame.pack(fill="both",expand=True)
    ttk.Label(frame,text="Connect your playtest",font=("Segoe UI",16,"bold")).pack(anchor="w")
    message = tk.StringVar(value="Join both controllers in GameNight, then launch Relay Keepers. This desk connects automatically.")
    ttk.Label(frame,textvariable=message,wraplength=400).pack(anchor="w",pady=10)
    buttons=ttk.Frame(frame)
    buttons.pack(anchor="w")
    companion=None

    def launch(standalone):
        try:
            if standalone:
                program = GAME.parent/"RelayKeepers.exe"
                STANDALONE_TOKEN.parent.mkdir(parents=True,exist_ok=True)
                subprocess.Popen([str(program),"run",str(GAME),"--players","2","--windowed",
                                  "--live","0","--live-token-file",str(STANDALONE_TOKEN)],
                                 cwd=GAME.parent,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
                message.set("Starting a windowed session for keyboard and one controller.")
            else:
                launcher=LOCAL/"Ontola.GameNight.Preview"/"GameNight.exe"
                if not launcher.is_file():
                    raise FileNotFoundError("Open your installed GameNight launcher and start Relay Keepers.")
                os.startfile(launcher)
                message.set("Join both controllers and launch Relay Keepers in GameNight.")
            for button in buttons.winfo_children():
                button.state(["disabled"])
            root.after(6000,lambda: [b.state(["!disabled"]) for b in buttons.winfo_children()])
        except OSError as exc:
            message.set(str(exc))

    ttk.Button(buttons,text="Open GameNight",command=lambda:launch(False)).pack(side="left",padx=(0,6))
    ttk.Button(buttons,text="Keyboard + controller",command=lambda:launch(True)).pack(side="left")
    def detect():
        nonlocal companion
        try:
            client=live_client(args.token_file)
            companion=PlaytestSession(client,args.log_dir)
            companion.start()
        except Exception as exc:
            if companion:
                companion.close()
                companion=None
                message.set(str(exc))
            root.after(900,detect)
            return
        root.destroy()
    root.after(50,detect)
    root.mainloop()
    if companion:
        run_panel(companion)


def notes_command(args):
    with closing(connect_log(args.log_dir)) as db:
        if args.command=="respond":
            with db:
                changed=db.execute("UPDATE notes SET handled=?,response=? WHERE id=? AND handled IS NULL",
                                   (timestamp(),args.summary,args.note_id)).rowcount
            if not changed:
                raise ValueError("Note was already handled or does not exist.")
            return {"handled":args.note_id}
        if args.command=="report":
            return {"checkpoint":dict(db.execute("SELECT room,finished FROM checkpoint").fetchone()),
                    "visits":[dict(v) for v in db.execute("SELECT * FROM visits ORDER BY id")],
                    "notes":[dict(v) for v in db.execute("SELECT * FROM notes ORDER BY id")]}
        deadline=time.monotonic()+max(0,min(args.seconds,60))
        while True:
            rows=db.execute("SELECT id,room,name,kind,text,cleared,falls,created FROM notes WHERE handled IS NULL ORDER BY id").fetchall()
            if rows or time.monotonic()>=deadline:
                return [dict(row) for row in rows]
            time.sleep(0.5)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--log-dir",type=Path,default=SESSION)
    parser.add_argument("--token-file",type=Path)
    commands=parser.add_subparsers(dest="command",required=True)
    for name in ("panel","status","reload","report"):
        commands.add_parser(name)
    wait=commands.add_parser("pending")
    wait.add_argument("--seconds",type=float,default=0)
    respond=commands.add_parser("respond")
    respond.add_argument("note_id",type=int)
    respond.add_argument("summary")
    args=parser.parse_args()
    if args.command=="panel":
        return panel(args)
    if args.command in ("pending","respond","report"):
        result=notes_command(args)
    else:
        client=live_client(args.token_file)
        if args.command=="reload":
            result={"reload":client.call("reload",{}),"errors":json.loads(client.call("query",{"pointer":"/errors"})).get("/errors"),
                    "room":client.review("status")}
        else:
            result=client.review("status")
    print(json.dumps(result,ensure_ascii=False,indent=2))

if __name__=="__main__":
    try:
        main()
    except (OSError,RuntimeError,ValueError) as error:
        sys.exit(str(error))
