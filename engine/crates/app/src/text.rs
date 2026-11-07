//! Text entry: the keyboard's text (typed characters, IME commits, editing
//! keys, paste) as [`TextEvent`]s, and [`TextField`], a single-line edit
//! model with a caret that draws into the HUD list. The shell sends text
//! events while [`crate::Game::text_input`] is true; a pad can produce the
//! same events with [`crate::ui::CharPicker`].
use render::hud::{FontFace, HudDrawList, TextAlign, TextRun};

/// One piece of text input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextEvent {
    /// Typed characters or an IME commit.
    Insert(String),
    /// Ctrl/Cmd+V: the clipboard's text.
    Paste(String),
    /// Ctrl/Cmd+C.
    Copy, Cut, SelectAll, SelectLeft, SelectRight, SelectHome, SelectEnd,
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Enter,
    Escape,
}

impl TextEvent {
    /// Script names: `Backspace`, `Delete`, `Left`, `Right`, `Home`, `End`,
    /// `Enter`, `Escape`, `Copy`, `Paste` (the clipboard's text).
    pub fn key(name: &str) -> Result<TextEvent, String> {
        Ok(match name.to_ascii_lowercase().as_str() {
            "backspace" => TextEvent::Backspace,
            "delete" | "del" => TextEvent::Delete,
            "left" => TextEvent::Left,
            "right" => TextEvent::Right,
            "home" => TextEvent::Home,
            "end" => TextEvent::End,
            "enter" | "return" => TextEvent::Enter,
            "escape" | "esc" => TextEvent::Escape,
            "selectall" => TextEvent::SelectAll,
            "cut" => TextEvent::Cut,
            "copy" => TextEvent::Copy,
            "paste" => TextEvent::Paste(crate::clipboard::get()),
            other => return Err(format!("unknown text key {other}")),
        })
    }
}

/// What an edit did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldEvent { None, Changed, Submit, Cancel, Copied }

/// A single-line text field: text, a caret and a length limit. Control
/// characters (newlines, tabs) arrive as spaces.
#[derive(Clone, Debug, Default)]
pub struct TextField {
    text: String,
    /// Byte offset of the caret, on a char boundary.
    caret: usize,
    anchor: usize,
    last_click: Option<(std::time::Instant,[f32;2])>,
    layout: std::cell::Cell<Option<(f32,FontFace,f32)>>,
    /// Most characters kept (0: no limit).
    pub max_chars: usize,
    /// Seconds since the last edit, for the caret blink.
    pub blink: f32,
}

impl TextField {
    pub fn new(max_chars: usize) -> Self { TextField { max_chars, ..Default::default() } }
    pub fn text(&self) -> &str { &self.text }
    /// Replace the text; the caret goes to the end.
    pub fn set(&mut self, text: &str) {
        self.text.clear();
        self.caret = 0;
        self.anchor = 0;
        self.last_click = None;
        self.insert(text);
        self.blink = 0.0;
    }
    pub fn caret(&self) -> usize { self.caret }
    pub fn is_empty(&self) -> bool { self.text.is_empty() }

    fn insert(&mut self, s: &str) -> bool {
        if s.is_empty() { return false; }
        let mut changed = self.delete_selection();
        let mut n = self.text.chars().count();
        for ch in s.chars() {
            if self.max_chars > 0 && n >= self.max_chars { break; }
            let ch = if ch.is_control() { ' ' } else { ch };
            self.text.insert(self.caret, ch);
            self.caret += ch.len_utf8();
            n += 1;
            changed = true;
        }
        self.anchor = self.caret;
        changed
    }

    fn prev(&self) -> Option<usize> { self.text[..self.caret].char_indices().next_back().map(|(i, _)| i) }
    fn next(&self) -> Option<usize> { self.text[self.caret..].chars().next().map(|c| self.caret + c.len_utf8()) }

    fn selection(&self) -> std::ops::Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }
    fn delete_selection(&mut self) -> bool {
        let range=self.selection();
        if range.is_empty() { return false; }
        self.caret=range.start;
        self.anchor=range.start;
        self.text.replace_range(range,"");
        true
    }
    /// Place or extend the selection using the last drawn font and scroll offset.
    pub fn pointer_select(&mut self, xy:[f32;2], pressed:bool) {
        let Some((left,font,size))=self.layout.get() else {return;};
        self.blink=0.;
        if pressed {
            let now=std::time::Instant::now();
            if self.last_click.is_some_and(|(at,p)| now.duration_since(at).as_millis()<=500
                && (p[0]-xy[0]).abs()<=5. && (p[1]-xy[1]).abs()<=5.) {
                self.last_click=None;
                self.apply(&TextEvent::SelectAll);
                return;
            }
            self.last_click=Some((now,xy));
        }
        let x=xy[0]-left;
        let indices:Vec<_>=self.text.char_indices().map(|(i,_)|i).chain(std::iter::once(self.text.len())).collect();
        let i=indices.partition_point(|&end|width_with_spaces(&self.text[..end],font,size)<x);
        let end=indices[i.min(indices.len()-1)];
        let start=indices[i.saturating_sub(1)];
        let middle=0.5*(width_with_spaces(&self.text[..start],font,size)+width_with_spaces(&self.text[..end],font,size));
        self.caret=if x<middle {start} else {end};
        if pressed { self.anchor=self.caret; }
    }

    /// Apply one event.
    pub fn apply(&mut self, ev: &TextEvent) -> FieldEvent {
        self.blink = 0.0;
        let changed = match ev {
            TextEvent::Insert(s) | TextEvent::Paste(s) => self.insert(s),
            TextEvent::Delete | TextEvent::Backspace => {
                if self.anchor==self.caret {
                    self.anchor=if matches!(ev,TextEvent::Backspace) {self.prev()} else {self.next()}.unwrap_or(self.caret);
                }
                self.delete_selection()
            }
            TextEvent::SelectAll => {self.anchor=0;self.caret=self.text.len();false}
            TextEvent::Left | TextEvent::Right | TextEvent::Home | TextEvent::End |
            TextEvent::SelectLeft | TextEvent::SelectRight | TextEvent::SelectHome | TextEvent::SelectEnd => {
                self.caret=match ev {
                    TextEvent::Left if self.anchor!=self.caret => self.selection().start,
                    TextEvent::Right if self.anchor!=self.caret => self.selection().end,
                    TextEvent::Left|TextEvent::SelectLeft => self.prev().unwrap_or(0),
                    TextEvent::Right|TextEvent::SelectRight => self.next().unwrap_or(self.text.len()),
                    TextEvent::Home|TextEvent::SelectHome => 0,
                    _=>self.text.len()
                };
                if matches!(ev,TextEvent::Left|TextEvent::Right|TextEvent::Home|TextEvent::End){self.anchor=self.caret;}
                false
            }
            TextEvent::Cut => {
                if self.anchor!=self.caret { crate::clipboard::set(&self.text[self.selection()]); }
                self.delete_selection()
            }
            TextEvent::Enter => return FieldEvent::Submit,
            TextEvent::Escape => return FieldEvent::Cancel,
            TextEvent::Copy => { let range=self.selection();
                crate::clipboard::set(if range.is_empty(){&self.text}else{&self.text[range]}); return FieldEvent::Copied; }
        };
        if changed { FieldEvent::Changed } else { FieldEvent::None }
    }

    pub fn tick(&mut self, dt: f32) { self.blink += dt; }

    /// The caret's x from the text's left edge at this face and size.
    pub fn caret_x(&self, font: FontFace, size: f32) -> f32 { width_with_spaces(&self.text[..self.caret], font, size) }

    /// Draw the text (or the placeholder) left-aligned and vertically
    /// centred in `rect`, clipped to it, scrolled so the caret shows; with
    /// `caret` (a colour), a blinking caret. Colours are sRGB bytes.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&self, d: &mut HudDrawList, id: u64, rect: [f32; 4], font: FontFace, size: f32, color: [u8; 4], placeholder: (&str, [u8; 4]), caret: Option<[f32; 4]>) {
        let top = rect[1] + (rect[3] - size * 1.2) * 0.5 - size * 0.04;
        let cx = self.caret_x(font, size);
        let scroll = (cx - rect[2] + size * 0.3).max(0.0);
        let left=rect[0]-scroll;
        self.layout.set(Some((left,font,size)));
        let sel=self.selection();
        if caret.is_some() && !sel.is_empty() {
            let x=width_with_spaces(&self.text[..sel.start],font,size);
            let end=width_with_spaces(&self.text[..sel.end],font,size);
            d.rect([left+x,rect[1]+rect[3]*0.12,end-x,rect[3]*0.76],[0.10,0.38,0.57,0.9],rect);
        }
        let (text, c) = if self.text.is_empty() { (placeholder.0, placeholder.1) } else { (self.text.as_str(), color) };
        if !text.is_empty() {
            d.text.push(TextRun { id, text: text.into(), position: [rect[0] - scroll, top], bounds: rect, font_size: size, color: c, font, align: TextAlign::Left,
                width: width_with_spaces(text, font, size) + size });
        }
        if let Some(cc) = caret && (self.blink % 1.0) < 0.6 {
            let x = rect[0] + cx - scroll;
            d.rect([x, rect[1] + rect[3] * 0.2, (size * 0.08).max(1.0), rect[3] * 0.6], cc, rect);
        }
    }
}

/// Width including trailing spaces (layout drops them from a line's width).
fn width_with_spaces(s: &str, font: FontFace, size: f32) -> f32 {
    if s.is_empty() { return 0.0; }
    let bar = render::hud::measure("|", font, size)[0];
    (render::hud::measure(&format!("{s}|"), font, size)[0] - bar).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ins(s: &str) -> TextEvent { TextEvent::Insert(s.into()) }

    #[test]
    fn editing_moves_the_caret_over_whole_characters() {
        let mut f = TextField::new(0);
        assert_eq!(f.apply(&ins("hé")), FieldEvent::Changed);
        assert_eq!(f.apply(&ins("llo")), FieldEvent::Changed);
        assert_eq!(f.text(), "héllo");
        f.apply(&TextEvent::Home);
        f.apply(&TextEvent::Right);
        f.apply(&TextEvent::Right);
        assert_eq!(f.caret(), 3, "past the two-byte é");
        assert_eq!(f.apply(&TextEvent::Backspace), FieldEvent::Changed);
        assert_eq!(f.text(), "hllo");
        assert_eq!(f.apply(&TextEvent::Delete), FieldEvent::Changed);
        assert_eq!(f.text(), "hlo");
        f.apply(&TextEvent::End);
        assert_eq!(f.apply(&TextEvent::Delete), FieldEvent::None, "nothing after the caret");
        f.apply(&TextEvent::Home);
        assert_eq!(f.apply(&TextEvent::Backspace), FieldEvent::None);
        assert_eq!(f.apply(&TextEvent::Left), FieldEvent::None);
        assert_eq!(f.apply(&TextEvent::Enter), FieldEvent::Submit);
        assert_eq!(f.apply(&TextEvent::Escape), FieldEvent::Cancel);
    }

    #[test]
    fn double_click_and_keyboard_selections_replace_whole_characters() {
        let mut f=TextField::new(3);
        f.set("a日é");
        f.apply(&TextEvent::SelectLeft);
        assert_eq!(&f.text[f.selection()],"é");
        f.apply(&TextEvent::Paste("x".into()));
        assert_eq!(f.text(),"a日x");
        f.layout.set(Some((0., FontFace::Body, 16.)));
        f.pointer_select([8.,4.],true);
        f.pointer_select([8.,4.],true);
        assert_eq!(&f.text[f.selection()],"a日x");
        f.apply(&ins("b"));
        f.apply(&TextEvent::SelectAll);
        f.apply(&TextEvent::Delete);
        assert!(f.is_empty());
    }

    #[test]
    fn limits_and_control_characters() {
        let mut f = TextField::new(5);
        f.apply(&TextEvent::Paste("ab\ncdefg".into()));
        assert_eq!(f.text(), "ab cd");
        assert_eq!(f.apply(&ins("x")), FieldEvent::None, "full");
        f.set("日本語");
        assert_eq!((f.text(), f.caret()), ("日本語", 9));
        crate::clipboard::memory_only();
        assert_eq!(f.apply(&TextEvent::Copy), FieldEvent::Copied);
        assert_eq!(crate::clipboard::get(), "日本語");
        assert_eq!(TextEvent::key("paste"), Ok(TextEvent::Paste("日本語".into())));
        assert!(TextEvent::key("tab").is_err());
    }

    #[test]
    fn drawing_clips_scrolls_and_blinks() {
        let mut f = TextField::new(0);
        let mut d = HudDrawList::new([400, 100]);
        f.draw(&mut d, 1, [10.0, 10.0, 100.0, 30.0], FontFace::Body, 16.0, [255; 4], ("type here", [128; 4]), Some([1.0; 4]));
        assert_eq!(d.text.len(), 1);
        assert_eq!(d.text[0].text, "type here");
        assert!(!d.vertices.is_empty(), "caret shown");
        f.set(&"W".repeat(40));
        let mut d = HudDrawList::new([400, 100]);
        f.blink = 0.7;
        f.draw(&mut d, 1, [10.0, 10.0, 100.0, 30.0], FontFace::Body, 16.0, [255; 4], ("", [0; 4]), Some([1.0; 4]));
        assert!(d.text[0].position[0] < 10.0, "scrolled to keep the caret in view");
        assert_eq!(d.text[0].bounds, [10.0, 10.0, 100.0, 30.0]);
        assert!(d.vertices.is_empty(), "caret off in the blink");
        assert!(f.caret_x(FontFace::Body, 16.0) > 100.0);
        assert!(width_with_spaces("a ", FontFace::Body, 16.0) > width_with_spaces("a", FontFace::Body, 16.0));
    }
}
