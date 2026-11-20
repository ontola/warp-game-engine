"""Local two-player review controls. Start with run_panel(session)."""

import tkinter as tk
from tkinter import messagebox, ttk


def run_panel(session):
    root = tk.Tk()
    root.title("Relay Keepers · Playtest")
    root.minsize(428, 380)
    frame = ttk.Frame(root, padding=(14, 12))
    frame.pack(fill="both", expand=True)
    frame.columnconfigure(0, weight=1)
    frame.rowconfigure(6, weight=1)

    heading = tk.StringVar(value="Connecting to Relay Keepers…")
    progress = tk.StringVar()
    notice = tk.StringVar()
    agent_update = tk.StringVar()
    disposition = tk.StringVar()
    latest = None
    connected = False
    draft_room = None
    polling = None
    approved_room = None
    dialog_open = False

    ttk.Label(frame, textvariable=heading, font=("Segoe UI", 15, "bold")).grid(
        row=0, column=0, sticky="w"
    )
    ttk.Label(frame, textvariable=progress).grid(row=1, column=0, sticky="w", pady=(2, 9))
    controls = ttk.Frame(frame)
    controls.grid(row=2, column=0, sticky="ew")
    buttons = {}
    for col, (action, label) in enumerate(
        (("pause", "Pause"), ("resume", "Resume"), ("retry", "Retry room"), ("next", "Next room"))
    ):
        controls.columnconfigure(col, weight=1)
        buttons[action] = ttk.Button(controls, text=label, command=lambda a=action: act(a))
        buttons[action].grid(row=0, column=col, sticky="ew", padx=(0, 4 if col < 3 else 0))

    ttk.Separator(frame).grid(row=3, column=0, sticky="ew", pady=(12, 8))
    ratings = ttk.Frame(frame)
    ratings.grid(row=4, column=0, sticky="w")
    for col, (value, label) in enumerate(
        (("good", "Good"), ("issue", "Issue"), ("blocked", "Blocked"))
    ):
        ttk.Radiobutton(
            ratings, text=label, value=value, variable=disposition, command=lambda: edited()
        ).grid(row=0, column=col, padx=(0, 16))

    ttk.Label(frame, text="Room feedback").grid(row=5, column=0, sticky="w", pady=(7, 3))
    editor = tk.Text(frame, height=4, width=40, wrap="word", undo=True, font="TkDefaultFont")
    editor.grid(row=6, column=0, sticky="nsew")
    footer = ttk.Frame(frame)
    footer.grid(row=7, column=0, sticky="ew", pady=(7, 0))
    save = ttk.Button(footer, text="Save note", command=lambda: save_note())
    save.pack(side="left")
    ttk.Button(footer, text="Copy for chat", command=lambda: copy_note()).pack(side="left", padx=5)
    discard = ttk.Button(footer, text="Clear draft", command=lambda: clear_requested())
    discard.pack(side="right")
    ttk.Label(frame, text="Say “Start live playtest” in this chat once. During a live session, Save note queues feedback for Codex. Otherwise use Copy for chat.",
              wraplength=395).grid(row=8, column=0, sticky="w", pady=(8, 3))
    ttk.Label(frame, textvariable=notice, wraplength=395, foreground="#286454").grid(
        row=9, column=0, sticky="w"
    )

    ttk.Label(frame, textvariable=agent_update, wraplength=395).grid(
        row=10, column=0, sticky="w"
    )

    def room_of(status):
        return status["room"], status["name"]

    def room_label(room):
        return f"Room {room[0]} · {room[1]}"

    def content():
        return editor.get("1.0", "end-1c").strip()

    def dirty():
        return bool(disposition.get() or content())

    def clear():
        nonlocal draft_room
        editor.delete("1.0", "end")
        editor.edit_reset()
        disposition.set("")
        draft_room = None
        update_controls()

    def edited(*_):
        nonlocal draft_room
        if dirty() and draft_room is None and latest:
            draft_room = room_of(latest)
        if not dirty():
            draft_room = None
        update_controls()

    def on_text(_):
        if editor.edit_modified():
            editor.edit_modified(False)
            edited()

    def clear_requested():
        if confirm_discard("clear this draft"):
            clear()
            notice.set("Draft cleared.")

    def confirm(title, question):
        nonlocal dialog_open
        dialog_open = True
        try:
            return messagebox.askyesno(title, question, default=messagebox.NO, parent=root)
        finally:
            dialog_open = False

    def confirm_discard(reason):
        if not dirty():
            return True
        owner = room_label(draft_room) if draft_room else "this room"
        return confirm("Unsaved feedback", f"Discard the unsaved feedback for {owner} and {reason}?")

    def update_controls():
        held = bool(latest and latest["held"])
        ended = bool(latest and latest["finished"])
        cleared = bool(latest and latest["cleared"])
        states = {
            "pause": connected and not (held or ended or cleared),
            "resume": connected and held and not (ended or cleared),
            "retry": connected and not ended,
            "next": connected and not ended,
        }
        for key, enabled in states.items():
            buttons[key].state(["!disabled" if enabled else "disabled"])
        if latest:
            last = latest["room"] == latest["total"]
            buttons["next"].configure(text="Finish review" if last else "Next room")
        can_save = (connected and latest and disposition.get() and draft_room == room_of(latest))
        save.state(["!disabled" if can_save else "disabled"])
        discard.state(["!disabled" if dirty() else "disabled"])

    def show(status):
        nonlocal latest, connected, draft_room
        was_connected = connected
        latest, connected = status, True
        current = room_of(status)
        if dirty() and draft_room is None:
            draft_room = current
        heading.set(room_label(current))
        agent_update.set(status.get("update") or "")
        if status["finished"]:
            phase = "Finished"
        elif status["cleared"]:
            phase = "Cleared · RB marks good and continues"
        else:
            phase = "Paused" if status["held"] else "Playing"
        progress.set(f"{phase}    ·    {status['reviewed']} / {status['total']} reviewed"
                     f"    ·    {status['falls']} falls")
        if dirty() and draft_room != current:
            notice.set(f"Draft belongs to {room_label(draft_room)}. Copy it for chat or clear it.")
        elif not was_connected:
            notice.set("")
        update_controls()

    def failed(error):
        nonlocal connected
        connected = False
        progress.set("Connection unavailable · Reconnecting…")
        notice.set(str(error) or "Check that the playtest game is still running.")
        update_controls()

    def refresh():
        try:
            show(session.status())
            return True
        except Exception as error:
            failed(error)
            return False

    def poll():
        nonlocal polling, approved_room
        if refresh():
            if not latest.get("approve"):
                approved_room = None
            elif not (latest["finished"] or dirty() or dialog_open):
                room = room_of(latest)
                if room != approved_room:
                    try:
                        result = session.approve()
                        approved_room = room
                        if result is None:
                            refresh()
                        else:
                            show(result)
                        notice.set(f"Controllers marked {room_label(room)} good.")
                    except Exception as error:
                        failed(error)
        polling = root.after(900, poll)

    def save_note():
        if not refresh() or draft_room != room_of(latest) or not disposition.get():
            return
        owner = room_label(draft_room)
        try:
            session.note(disposition.get(), content())
        except Exception as error:
            failed(error)
            return
        clear()
        notice.set(f"Saved feedback for {owner}.")
        refresh()

    def copy_note():
        if dirty():
            if draft_room is None:
                notice.set("Connect to the playtest to identify the room for this feedback.")
                return
            room = draft_room
            kind, body = disposition.get() or "Feedback", content()
        else:
            try:
                last = session.last_note()
            except Exception as error:
                notice.set(f"Could not read the last saved note. {error}")
                return
            if not last:
                notice.set("Write feedback or save a note first.")
                return
            room, kind, body = room_of(last), last["kind"], last["text"]
        text = f"Relay Keepers · {room_label(room)} · {kind.title()}"
        if body:
            text += "\n" + body
        root.clipboard_clear()
        root.clipboard_append(text)
        notice.set(f"Copied feedback for {room_label(room)}. Paste it into this chat.")

    def act(action):
        if not refresh() or "disabled" in buttons[action].state():
            return
        leaving = action in ("retry", "next")
        last = latest["room"] == latest["total"]
        if leaving and not confirm_discard("retry the room" if action == "retry" else "continue"):
            return
        if action == "next" and last and not confirm(
            "Finish review", "Finish the review of the final room?"
        ):
            return
        try:
            status = session.action(action)
        except Exception as error:
            failed(error)
            return
        if leaving:
            clear()
            notice.set("")
        show(status)

    def close():
        if not confirm_discard("close the panel"):
            return
        if polling is not None:
            root.after_cancel(polling)
        try:
            session.close()
        finally:
            root.destroy()

    root.protocol("WM_DELETE_WINDOW", close)
    editor.bind("<<Modified>>", on_text)
    poll()
    root.mainloop()
