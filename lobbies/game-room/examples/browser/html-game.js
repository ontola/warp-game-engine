const channel = "splitscreen-club-webgame-v1";
let current;

function send(payload) {
    current?.frame.contentWindow?.postMessage({channel, payload}, location.origin);
}

function closeGame() {
    if (!current) return;
    send({type:"dispose", session:current.prepare.session});
    current.layer.remove();
    current = undefined;
}

export function mount(entry, title, description) {
    closeGame();
    const url = new URL(entry, document.baseURI);
    if (url.origin !== location.origin || !["http:", "https:"].includes(url.protocol))
        throw new Error("HTML games must be served from the lobby origin");
    const {party, prepare} = JSON.parse(description);
    const layer = document.createElement("div");
    layer.id = "html-game";
    layer.style.cssText = "position:fixed;inset:0;z-index:90;background:#12151c";
    const frame = document.createElement("iframe");
    frame.title = title;
    frame.allow = "autoplay; fullscreen; gamepad";
    frame.style.cssText = "width:100%;height:100%;border:0;display:block";
    const back = document.createElement("button");
    back.type = "button";
    back.textContent = "Back to room";
    back.style.cssText = "position:absolute;top:8px;right:8px;padding:6px 10px;background:#18212c;color:white;border:1px solid #89909a;border-radius:4px;cursor:pointer";
    back.onclick = () => window.warp.back_to_room();
    frame.addEventListener("load", () => {
        if (current?.frame !== frame) return;
        try {
            // Keyboard events inside an iframe do not reach Warp's device router.
            for (const type of ["keydown", "keyup"])
                frame.contentWindow.addEventListener(type, e => {
                    if (!current?.managed || current.layer.hidden) return;
                    const canvas = document.querySelector("#warp");
                    canvas?.dispatchEvent(new KeyboardEvent(type, {key:e.key,code:e.code,
                        repeat:e.repeat,ctrlKey:e.ctrlKey,altKey:e.altKey,shiftKey:e.shiftKey,
                        metaKey:e.metaKey,bubbles:true,cancelable:true}));
                }, true);
        } catch { /* A navigation away from the lobby origin ends input forwarding. */ }
    });
    current = {layer, frame, prepare, party, managed:false, ready:false};
    frame.src = url.href;
    layer.append(frame, back);
    document.body.append(layer);
    frame.focus();
}

if (typeof window !== "undefined") window.addEventListener("message", e => {
    if (!current || e.origin !== location.origin || e.source !== current.frame.contentWindow ||
        e.data?.channel !== channel) return;
    const msg = e.data.payload;
    if (!msg || typeof msg !== "object") return;
    if (msg.type === "hello" && msg.role === "game" && msg.game === current.prepare.game) {
        current.managed = true;
        current.ready = false;
        send({type:"welcome", protocol_version:1, party:current.party});
        send(current.prepare);
    } else if (current.managed && msg.session === current.prepare.session) {
        if (msg.type === "ready" && !current.ready) {
            current.ready = true;
            send({type:"start", session:msg.session});
            if (current.layer.hidden) send({type:"pause", session:msg.session});
        } else if (msg.type === "request_overlay" && !current.layer.hidden) {
            window.warp.back_to_room();
        }
    } else if (current.managed && msg.type === "request_overlay" && msg.session == null && !current.layer.hidden) {
        // The v1 SDK's overlay request may omit the session on its dedicated channel.
        window.warp.back_to_room();
    }
});

export function active(session) {
    return current?.prepare.session === session && current.ready && !current.layer.hidden;
}
export function input(session, frames) {
    if (active(session)) send({type:"controller_frame", session, controllers:JSON.parse(frames)});
}
export function pause() {
    if (!current || current.layer.hidden) return true;
    if (!current.managed) { closeGame(); return false; }
    send({type:"pause", session:current.prepare.session});
    current.layer.hidden = true;
    document.querySelector("#warp")?.focus();
    return true;
}
export function resume(session) {
    if (current?.prepare.session !== session) return false;
    current.layer.hidden = false;
    current.frame.focus();
    if (current.ready) send({type:"resume",session});
    return true;
}
export function dispose(session) {
    if (current?.prepare.session === session) closeGame();
}
