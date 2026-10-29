//! The system clipboard (text, and images for screenshots) through `arboard`, with an in-memory
//! fallback: where there is no clipboard (headless, no display server) or
//! after [`memory_only`] (scripted playtests, tests), copy and paste stay
//! inside the process, so a script never touches the user's clipboard.
use std::sync::Mutex;

struct State { memory_only: bool, memory: String, system: Option<arboard::Clipboard>, tried: bool }

static STATE: Mutex<State> = Mutex::new(State { memory_only: false, memory: String::new(), system: None, tried: false });

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut s)
}

fn system(s: &mut State) -> Option<&mut arboard::Clipboard> {
    if s.memory_only { return None; }
    if !s.tried {
        s.tried = true;
        s.system = arboard::Clipboard::new().map_err(|e| eprintln!("clipboard: {e}; copy and paste stay in this program")).ok();
    }
    s.system.as_mut()
}

/// Keep the clipboard inside this process from now on.
pub fn memory_only() { with(|s| { s.memory_only = true; s.system = None; }); }

/// Put text on the clipboard. Returns false when only the in-process
/// clipboard has it.
pub fn set(text: &str) -> bool {
    with(|s| {
        s.memory = text.to_string();
        system(s).is_some_and(|c| c.set_text(text.to_string()).map_err(|e| eprintln!("clipboard: {e}")).is_ok())
    })
}

/// Put an RGBA8 image (top row first) on the system clipboard. False when
/// there is none or the clipboard is in-process only.
pub fn set_image(width: u32, height: u32, rgba: &[u8]) -> bool {
    let image = arboard::ImageData { width: width as usize, height: height as usize, bytes: rgba.into() };
    with(|s| system(s).is_some_and(|c| c.set_image(image).map_err(|e| eprintln!("clipboard: {e}")).is_ok()))
}

/// The clipboard's text (the system's when it has text, else the last
/// [`set`] in this process).
pub fn get() -> String {
    with(|s| {
        let memory = s.memory.clone();
        system(s).and_then(|c| c.get_text().ok()).unwrap_or(memory)
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn memory_clipboard_round_trips() {
        super::memory_only();
        assert!(!super::set("ticket-abc"));
        assert_eq!(super::get(), "ticket-abc");
    }
}
