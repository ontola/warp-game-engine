//! Small Win32 front end. The native trackbars and buttons retain their normal
//! tab/arrow-key behavior; the oscilloscope reads samples from the audio callback.
use crate::bench::{Controller, Settings};
use std::{
    cell::RefCell,
    f32::consts::TAU,
    mem::size_of,
    ptr::{null, null_mut},
    time::Instant,
};
use windows_sys::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Controls::*, HiDpi::*, Input::KeyboardAndMouse::SetFocus, WindowsAndMessaging::*},
    },
};

const TBM_GETPOS: u32 = WM_USER; // commctrl.h gives this message offset zero.
const START: usize = 100;
const MODEL: usize = 101;
const SLOT_A: usize = 102;
const SLOT_B: usize = 103;
const COPY: usize = 104;
const SLIDER: usize = 110;
const ROWS: usize = 6;
const STYLE: u32 = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN;
const TITLE: PCWSTR = w!("Warp Engine Generator");
const CLASS: PCWSTR = w!("WarpEngineSoundLabWindow");

struct Row {
    label: &'static str,
    min: i32,
    max: i32,
    line: i32,
    page: i32,
}
const ROW: [Row; ROWS] = [
    Row {
        label: "&RPM",
        min: 650,
        max: 8000,
        line: 50,
        page: 500,
    },
    Row {
        label: "Engine &load",
        min: -100,
        max: 100,
        line: 1,
        page: 10,
    },
    Row {
        label: "Output &volume",
        min: 0,
        max: 100,
        line: 1,
        page: 10,
    },
    Row {
        label: "Chamber len&gth",
        min: 10,
        max: 150,
        line: 1,
        page: 10,
    },
    Row {
        label: "Chamber &diameter",
        min: 40,
        max: 350,
        line: 1,
        page: 10,
    },
    Row {
        label: "Wall &packing",
        min: 0,
        max: 100,
        line: 1,
        page: 10,
    },
];

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn last_error() -> String {
    std::io::Error::last_os_error().to_string()
}
fn px(n: i32, dpi: i32) -> i32 {
    (n * dpi + 48) / 96
}
fn color(r: u32, g: u32, b: u32) -> u32 {
    r | g << 8 | b << 16
}
fn rect(x: i32, y: i32, width: i32, height: i32) -> RECT {
    RECT {
        left: x,
        top: y,
        right: x + width,
        bottom: y + height,
    }
}
fn window_rect(width: i32, height: i32, dpi: i32) -> RECT {
    let mut r = rect(0, 0, px(width, dpi), px(height, dpi));
    unsafe {
        AdjustWindowRectExForDpi(&mut r, STYLE, 0, WS_EX_CONTROLPARENT, dpi as u32);
    }
    r
}

struct Fonts {
    ui: HFONT,
    title: HFONT,
    label: HFONT,
}
impl Fonts {
    fn new(dpi: i32) -> Self {
        let font = |height, weight| unsafe {
            CreateFontW(
                -px(height, dpi),
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                CLEARTYPE_QUALITY as u32,
                0,
                w!("Segoe UI"),
            )
        };
        Self {
            ui: font(15, 400),
            title: font(28, 600),
            label: font(14, 600),
        }
    }
}
impl Drop for Fonts {
    fn drop(&mut self) {
        unsafe {
            for f in [self.ui, self.title, self.label] {
                DeleteObject(f);
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Layout {
    width: i32,
    scope_height: i32,
}
impl Layout {
    fn new(hwnd: HWND, dpi: i32) -> Self {
        let mut area = RECT::default();
        unsafe {
            GetClientRect(hwnd, &mut area);
        }
        let width = area.right * 96 / dpi;
        let height = area.bottom * 96 / dpi;
        Self {
            width,
            scope_height: (height - 434).clamp(65, 220),
        }
    }
    fn controls_top(self) -> i32 {
        126 + self.scope_height
    }
    fn row(self, index: usize) -> i32 {
        self.controls_top() + 29 + index as i32 * 32 + if index >= 3 { 32 } else { 0 }
    }
}

struct Ui {
    slots: [Settings; 2],
    slot: usize,
    enabled: bool,
    audio: Option<Controller>,
    notice: String,
    buttons: [HWND; 4],
    combo: HWND,
    captions: [HWND; ROWS],
    bars: [HWND; ROWS],
    dpi: i32,
    fonts: Fonts,
    samples: [f32; 1024],
    phase: f32,
    last_frame: Instant,
}

impl Ui {
    fn new(dpi: i32) -> Self {
        let a = Settings::default();
        let b = Settings {
            chamber_length: 0.55,
            ..a
        };
        Self {
            slots: [a, b],
            slot: 0,
            enabled: false,
            audio: None,
            notice: String::new(),
            buttons: [null_mut(); 4],
            combo: null_mut(),
            captions: [null_mut(); ROWS],
            bars: [null_mut(); ROWS],
            dpi,
            fonts: Fonts::new(dpi),
            samples: [0.; 1024],
            phase: 0.,
            last_frame: Instant::now(),
        }
    }
    fn settings(&self) -> Settings {
        Settings {
            enabled: self.enabled,
            ..self.slots[self.slot]
        }
    }
    fn publish(&mut self) {
        let s = self.settings();
        if let Some(c) = &mut self.audio {
            c.update(s);
        }
    }
    fn values(&self) -> [i32; ROWS] {
        let s = self.settings();
        [
            s.rpm.round() as i32,
            (100. * s.load).round() as i32,
            (100. * s.volume).round() as i32,
            (100. * s.chamber_length).round() as i32,
            (1000. * s.chamber_diameter).round() as i32,
            (100. * s.packing).round() as i32,
        ]
    }
    fn readout(&self, index: usize) -> String {
        let n = self.values()[index];
        match index {
            0 => format!("{n} rpm"),
            3 => format!("{n} cm"),
            4 => format!("{n} mm"),
            _ => format!("{n}%"),
        }
    }
    fn change(&mut self, index: usize, n: i32) {
        let s = &mut self.slots[self.slot];
        match index {
            0 => {
                for config in &mut self.slots {
                    config.rpm = n as f32;
                }
            }
            1 => {
                for config in &mut self.slots {
                    config.load = n as f32 / 100.;
                }
            }
            2 => {
                for config in &mut self.slots {
                    config.volume = n as f32 / 100.;
                }
            }
            3 => s.chamber_length = n as f32 / 100.,
            4 => s.chamber_diameter = n as f32 / 1000.,
            5 => s.packing = n as f32 / 100.,
            _ => return,
        }
        self.publish();
    }
    fn controls(&mut self, host: HWND) -> Result<(), String> {
        let child = |class: PCWSTR, text: &str, style: u32, id: usize| -> Result<HWND, String> {
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    class,
                    wide(text).as_ptr(),
                    WS_CHILD | WS_VISIBLE | style,
                    0,
                    0,
                    0,
                    0,
                    host,
                    id as HMENU,
                    GetModuleHandleW(null()),
                    null(),
                )
            };
            if hwnd.is_null() {
                Err(last_error())
            } else {
                Ok(hwnd)
            }
        };
        self.buttons[0] = child(
            WC_BUTTONW,
            "&Start engine",
            WS_TABSTOP | BS_PUSHBUTTON as u32,
            START,
        )?;
        self.combo = child(
            WC_COMBOBOXW,
            "Engine model",
            WS_TABSTOP | CBS_DROPDOWNLIST as u32 | WS_VSCROLL,
            MODEL,
        )?;
        for model in ["Inline four", "Inline five", "Boxer four"] {
            unsafe {
                SendMessageW(self.combo, CB_ADDSTRING, 0, wide(model).as_ptr() as isize);
            }
        }
        self.buttons[1] = child(
            WC_BUTTONW,
            "Setup &A",
            WS_GROUP | WS_TABSTOP | BS_RADIOBUTTON as u32,
            SLOT_A,
        )?;
        self.buttons[2] = child(
            WC_BUTTONW,
            "Setup &B",
            WS_TABSTOP | BS_RADIOBUTTON as u32,
            SLOT_B,
        )?;
        for (i, row) in ROW.iter().enumerate() {
            self.captions[i] = child(WC_STATICW, row.label, WS_GROUP, 140 + i)?;
            self.bars[i] = child(
                TRACKBAR_CLASSW,
                row.label,
                WS_TABSTOP | TBS_NOTICKS,
                SLIDER + i,
            )?;
            unsafe {
                SendMessageW(self.bars[i], TBM_SETRANGEMIN, 0, row.min as isize);
                SendMessageW(self.bars[i], TBM_SETRANGEMAX, 0, row.max as isize);
                SendMessageW(self.bars[i], TBM_SETLINESIZE, 0, row.line as isize);
                SendMessageW(self.bars[i], TBM_SETPAGESIZE, 0, row.page as isize);
            }
        }
        self.buttons[3] = child(
            WC_BUTTONW,
            "&Copy Luau",
            WS_GROUP | WS_TABSTOP | BS_PUSHBUTTON as u32,
            COPY,
        )?;
        self.refresh_controls();
        self.resize(host);
        Ok(())
    }
    fn refresh_controls(&self) {
        let s = self.settings();
        unsafe {
            SetWindowTextW(
                self.buttons[0],
                if self.enabled {
                    w!("&Stop engine")
                } else {
                    w!("&Start engine")
                },
            );
            SendMessageW(self.buttons[1], BM_SETCHECK, usize::from(self.slot == 0), 0);
            SendMessageW(self.buttons[2], BM_SETCHECK, usize::from(self.slot == 1), 0);
            SendMessageW(self.combo, CB_SETCURSEL, s.model, 0);
            for (bar, v) in self.bars.iter().zip(self.values()) {
                SendMessageW(*bar, TBM_SETPOS, 1, v as isize);
            }
        }
    }
    fn resize(&mut self, hwnd: HWND) {
        if self.combo.is_null() {
            return;
        }
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as i32;
        if dpi != self.dpi {
            self.dpi = dpi;
            let replacement = Fonts::new(dpi);
            for child in self
                .buttons
                .iter()
                .chain(&self.captions)
                .chain([&self.combo])
            {
                unsafe {
                    SendMessageW(*child, WM_SETFONT, replacement.ui as usize, 1);
                }
            }
            self.fonts = replacement;
        } else {
            for child in self
                .buttons
                .iter()
                .chain(&self.captions)
                .chain([&self.combo])
            {
                unsafe {
                    SendMessageW(*child, WM_SETFONT, self.fonts.ui as usize, 0);
                }
            }
        }
        let l = Layout::new(hwnd, dpi);
        let place = |h, x, y, w, hgt| unsafe {
            SetWindowPos(
                h,
                null_mut(),
                px(x, dpi),
                px(y, dpi),
                px(w, dpi),
                px(hgt, dpi),
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        };
        place(self.buttons[0], l.width - 148, 19, 126, 35);
        place(self.combo, 22, 71, (l.width - 310).clamp(144, 238), 160);
        place(self.buttons[1], l.width - 209, 68, 88, 27);
        place(self.buttons[2], l.width - 113, 68, 88, 27);
        for i in 0..ROWS {
            let y = l.row(i);
            place(self.captions[i], 24, y + 4, 137, 24);
            place(self.bars[i], 161, y + 1, l.width - 277, 28);
        }
        place(
            self.buttons[3],
            l.width - 148,
            l.row(ROWS - 1) + 38,
            126,
            31,
        );
        unsafe {
            InvalidateRect(hwnd, null(), 0);
        }
    }
    fn command(&mut self, hwnd: HWND, id: usize, notify: u32) {
        match (id, notify) {
            (START, BN_CLICKED) => {
                if self.audio.is_none() {
                    match Controller::open() {
                        Ok(c) => self.audio = Some(c),
                        Err(err) => {
                            self.notice = format!("Audio device: {err}");
                            unsafe {
                                InvalidateRect(hwnd, null(), 0);
                            }
                            return;
                        }
                    }
                }
                self.enabled = !self.enabled;
                self.notice.clear();
                self.last_frame = Instant::now();
                if self.enabled {
                    unsafe {
                        SetTimer(hwnd, 1, 33, None);
                    }
                } else {
                    unsafe {
                        KillTimer(hwnd, 1);
                    }
                    self.samples.fill(0.);
                }
                self.publish();
                self.refresh_controls();
            }
            (MODEL, CBN_SELCHANGE) => {
                let m = unsafe { SendMessageW(self.combo, CB_GETCURSEL, 0, 0) };
                if (0..=2).contains(&m) {
                    self.slots[self.slot].model = m as usize;
                    self.publish();
                }
                self.notice.clear();
            }
            (SLOT_A | SLOT_B, BN_CLICKED) => {
                self.slot = usize::from(id == SLOT_B);
                self.notice.clear();
                self.publish();
                self.refresh_controls();
            }
            (COPY, BN_CLICKED) => {
                self.notice = match crate::copy_luau(&self.settings().luau()) {
                    Ok(()) => "Luau patch copied.".into(),
                    Err(e) => format!("Clipboard: {e}"),
                };
            }
            _ => return,
        }
        unsafe {
            InvalidateRect(hwnd, null(), 0);
        }
    }
    fn frame(&mut self, hwnd: HWND) {
        if !self.enabled || unsafe { IsIconic(hwnd) } != 0 {
            return;
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.12);
        self.last_frame = now;
        // Slow the crank by 32x so cylinder movement remains readable at 30 fps.
        self.phase = (self.phase + dt * self.settings().rpm / 60. / 32.) % 2.;
        if let Some(c) = &mut self.audio {
            c.waveform(&mut self.samples);
        }
        let layout = Layout::new(hwnd, self.dpi);
        let scope = rect(
            0,
            px(110, self.dpi),
            px(layout.width, self.dpi),
            px(layout.scope_height, self.dpi),
        );
        unsafe {
            InvalidateRect(hwnd, &scope, 0);
        }
    }
}

struct Canvas {
    dc: HDC,
    dpi: i32,
}
impl Canvas {
    fn pos(&self, n: i32) -> i32 {
        px(n, self.dpi)
    }
    fn fill(&self, x: i32, y: i32, w: i32, h: i32, c: u32) {
        let r = rect(
            self.pos(x),
            self.pos(y),
            self.pos(x + w) - self.pos(x),
            self.pos(y + h) - self.pos(y),
        );
        unsafe {
            SetDCBrushColor(self.dc, c);
            FillRect(self.dc, &r, GetStockObject(DC_BRUSH));
        }
    }
    fn text(&self, x: i32, y: i32, w: i32, h: i32, s: &str, f: HFONT, c: u32, align: u32) {
        let mut r = rect(self.pos(x), self.pos(y), self.pos(w), self.pos(h));
        let val = wide(s);
        unsafe {
            SelectObject(self.dc, f);
            SetTextColor(self.dc, c);
            SetBkMode(self.dc, TRANSPARENT as i32);
            DrawTextW(
                self.dc,
                val.as_ptr(),
                -1,
                &mut r,
                align | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS,
            );
        }
    }
    fn path(&self, points: &[POINT], c: u32) {
        unsafe {
            SelectObject(self.dc, GetStockObject(DC_PEN));
            SetDCPenColor(self.dc, c);
            Polyline(self.dc, points.as_ptr(), points.len() as i32);
        }
    }
}

impl Ui {
    fn display(&self, g: &Canvas, l: Layout) {
        let ink = color(28, 41, 48);
        let soft = color(83, 98, 103);
        let divider = color(206, 216, 217);
        let bg = unsafe { GetSysColor(COLOR_BTNFACE) };
        g.text(
            22,
            12,
            l.width - 193,
            43,
            "Warp Engine Generator",
            self.fonts.title,
            ink,
            DT_LEFT,
        );
        g.fill(22, 58, l.width - 44, 1, divider);
        let top = l.controls_top();
        g.text(
            23,
            top - 2,
            147,
            24,
            "Engine controls",
            self.fonts.label,
            ink,
            DT_LEFT,
        );
        g.fill(161, top + 11, l.width - 184, 1, divider);
        let mid = l.row(2) + 33;
        g.text(
            23,
            mid,
            147,
            24,
            "Expansion chamber",
            self.fonts.label,
            ink,
            DT_LEFT,
        );
        g.fill(161, mid + 13, l.width - 184, 1, divider);
        for i in 0..ROWS {
            g.text(
                l.width - 113,
                l.row(i),
                l.width.min(90),
                26,
                &self.readout(i),
                self.fonts.ui,
                ink,
                DT_RIGHT,
            );
        }
        let y = l.row(ROWS - 1) + 38;
        g.fill(22, y - 6, l.width - 44, 1, divider);
        g.fill(
            22,
            y + 9,
            6,
            12,
            if self.enabled {
                color(26, 136, 107)
            } else {
                soft
            },
        );
        let playback;
        let status = if !self.notice.is_empty() {
            self.notice.as_str()
        } else if let Some(audio) = self.audio.as_ref().filter(|_| self.enabled) {
            playback = format!("Playing on {}", audio.output_name());
            playback.as_str()
        } else {
            "Engine stopped"
        };
        g.text(
            39,
            y,
            l.width - 203,
            30,
            status,
            self.fonts.ui,
            soft,
            DT_LEFT,
        );
        // Fill gaps around the monitor with the same color as the native controls.
        g.fill(0, 108, l.width, 2, bg);
    }

    fn pistons(&self, g: &Canvas, x: i32, y: i32, width: i32, height: i32) {
        let grid = color(67, 85, 88);
        let metal = color(156, 177, 177);
        let active = color(242, 179, 91);
        let s = self.settings();
        let cyl = if s.model == 1 { 5 } else { 4 };
        // Invert the engine firing order to index crank phase by cylinder number.
        let stages: &[usize] = if cyl == 5 {
            &[0, 1, 4, 2, 3]
        } else if s.model == 2 {
            &[0, 2, 1, 3]
        } else {
            &[0, 3, 1, 2]
        };
        let base_y = y + height - 9;
        if s.model == 2 {
            let cx = x + width / 2;
            let stride = (height / 2).min(23);
            for (i, stage) in stages.iter().enumerate() {
                let t = (self.phase - 2. * (*stage as f32) / cyl as f32).rem_euclid(2.);
                let sign = if i % 2 == 0 { -1 } else { 1 };
                let row = y + height / 2 + (i / 2) as i32 * stride - stride / 2;
                let outer = (width / 2 - 8).clamp(36, 68);
                let dist = outer - 12 - ((0.5 - 0.5 * (TAU * t).cos()) * 20.) as i32;
                let far = cx + sign * outer;
                let pos = cx + sign * dist;
                let left = if sign < 0 { far } else { cx + 5 };
                g.fill(left, row - 8, outer - 5, 1, grid);
                g.fill(left, row + 8, outer - 5, 1, grid);
                g.fill(far, row - 8, 2, 17, grid);
                g.fill(pos.min(cx), row, (pos - cx).abs(), 2, metal);
                g.fill(
                    pos - 2,
                    row - 5,
                    5,
                    11,
                    if self.enabled && t < 0.18 {
                        active
                    } else {
                        metal
                    },
                );
            }
            g.fill(
                cx - 1,
                y + height / 2 - stride / 2 - 6,
                3,
                stride + 12,
                grid,
            );
        } else {
            let stride = (width / cyl as i32).clamp(14, 35);
            let origin = x + (width - stride * cyl as i32) / 2;
            let bore = (stride - 6).min(26);
            let chamber_h = (height - 14).clamp(19, 48);
            for (i, stage) in stages.iter().enumerate() {
                let t = (self.phase - 2. * (*stage as f32) / cyl as f32).rem_euclid(2.);
                let left = origin + i as i32 * stride;
                let head = base_y - chamber_h;
                let piston =
                    head + 4 + ((0.5 - 0.5 * (TAU * t).cos()) * (chamber_h - 13) as f32) as i32;
                g.fill(left, head, bore, 1, grid);
                g.fill(left, head, 1, chamber_h, grid);
                g.fill(left + bore - 1, head, 1, chamber_h, grid);
                g.fill(
                    left + 2,
                    piston,
                    bore - 4,
                    5,
                    if self.enabled && t < 0.18 {
                        active
                    } else {
                        metal
                    },
                );
                g.fill(left + bore / 2 - 1, piston + 5, 2, base_y - piston, metal);
            }
            g.fill(origin, base_y + 4, stride * cyl as i32 - 6, 2, grid);
        }
    }
    fn monitor(&self, g: &Canvas, l: Layout) {
        let x = 22;
        let y = 110;
        let width = l.width - 44;
        let height = l.scope_height;
        let split = (width * 32 / 100).clamp(151, 218);
        let paper = color(22, 37, 40);
        let grid = color(44, 64, 67);
        let label = color(166, 189, 187);
        g.fill(x, y, width, height, paper);
        g.fill(x + split, y + 12, 1, height - 24, grid);
        g.text(
            x + 12,
            y + 8,
            split - 22,
            19,
            "Crank · 1/32 speed",
            self.fonts.label,
            label,
            DT_LEFT,
        );
        self.pistons(g, x + 8, y + 28, split - 16, height - 35);
        let sx = x + split + 14;
        let sy = y + 33;
        let sw = width - split - 28;
        let sh = height - 47;
        g.text(
            sx,
            y + 8,
            sw,
            19,
            "Output waveform",
            self.fonts.label,
            label,
            DT_LEFT,
        );
        for i in 0..=8 {
            g.fill(sx + i * sw / 8, sy, 1, sh, grid);
        }
        for i in 0..=4 {
            g.fill(sx, sy + i * sh / 4, sw, 1, grid);
        }
        let mid = sy + sh / 2;
        let count = self.samples.len();
        let offset = self.samples[..count / 2]
            .windows(2)
            .position(|w| w[0] <= 0. && w[1] > 0.)
            .unwrap_or(0);
        let trace = (0..sw)
            .map(|j| {
                let k = offset + j as usize * (count / 2 - 1) / (sw - 1).max(1) as usize;
                let value = self.samples[k].clamp(-1., 1.);
                POINT {
                    x: g.pos(sx + j),
                    y: g.pos(mid - (value * (sh / 2 - 1) as f32).round() as i32),
                }
            })
            .collect::<Vec<_>>();
        g.path(
            &trace,
            if self.enabled {
                color(115, 232, 183)
            } else {
                color(80, 130, 111)
            },
        );
    }

    fn paint(&self, hwnd: HWND, dc: HDC, dirty: RECT) {
        let l = Layout::new(hwnd, self.dpi);
        let w = dirty.right - dirty.left;
        let h = dirty.bottom - dirty.top;
        if w <= 0 || h <= 0 {
            return;
        }
        // A paint DC can be clipped to the monitor alone. Keep the offscreen
        // bitmap just as small and draw everything in client coordinates.
        unsafe {
            let mem = CreateCompatibleDC(dc);
            if mem.is_null() {
                return;
            }
            let bitmap = CreateCompatibleBitmap(dc, w, h);
            if bitmap.is_null() {
                DeleteDC(mem);
                return;
            }
            let old = SelectObject(mem, bitmap);
            SetViewportOrgEx(mem, -dirty.left, -dirty.top, null_mut());
            FillRect(mem, &dirty, GetSysColorBrush(COLOR_BTNFACE));
            let canvas = Canvas {
                dc: mem,
                dpi: self.dpi,
            };
            let monitor_only = dirty.top >= px(110, self.dpi)
                && dirty.bottom <= px(110 + l.scope_height, self.dpi);
            if !monitor_only {
                self.display(&canvas, l);
            }
            self.monitor(&canvas, l);
            BitBlt(
                dc, dirty.left, dirty.top, w, h, mem, dirty.left, dirty.top, SRCCOPY,
            );
            SelectObject(mem, old);
            DeleteObject(bitmap);
            DeleteDC(mem);
        }
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = unsafe { &*(lp as *const CREATESTRUCTW) };
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
    }
    // Creating, resizing and synchronizing a native child can deliver
    // synchronous nested messages. They must never alias a mutable Ui.
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const RefCell<Ui>;
    if ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
    }
    match msg {
        WM_DESTROY => {
            unsafe {
                KillTimer(hwnd, 1);
                PostQuitMessage(0);
            }
            return 0;
        }
        WM_NCDESTROY => {
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
        }
        WM_ERASEBKGND => return 1,
        WM_GETMINMAXINFO => {
            let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as i32;
            let r = window_rect(548, 510, dpi);
            let limits = unsafe { &mut *(lp as *mut MINMAXINFO) };
            limits.ptMinTrackSize = POINT {
                x: r.right - r.left,
                y: r.bottom - r.top,
            };
            return 0;
        }
        WM_DPICHANGED => {
            let r = unsafe { &*(lp as *const RECT) };
            unsafe {
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let dc = unsafe { BeginPaint(hwnd, &mut ps) };
            if let Ok(ui) = unsafe { &*ptr }.try_borrow() {
                ui.paint(hwnd, dc, ps.rcPaint);
            }
            unsafe {
                EndPaint(hwnd, &ps);
            }
            return 0;
        }
        WM_PRINTCLIENT => {
            if let Ok(ui) = unsafe { &*ptr }.try_borrow() {
                let mut r = RECT::default();
                unsafe {
                    GetClientRect(hwnd, &mut r);
                }
                ui.paint(hwnd, wp as HDC, r);
            }
            return 0;
        }
        _ => {}
    }
    if let Ok(mut ui) = unsafe { &*ptr }.try_borrow_mut() {
        match msg {
            WM_SIZE | WM_DPICHANGED => {
                ui.resize(hwnd);
                return 0;
            }
            WM_TIMER if wp == 1 => {
                ui.frame(hwnd);
                return 0;
            }
            WM_COMMAND => {
                ui.command(hwnd, wp & 0xffff, ((wp >> 16) & 0xffff) as u32);
                return 0;
            }
            WM_HSCROLL => {
                if let Some(i) = ui.bars.iter().position(|h| *h == lp as HWND) {
                    let value = unsafe { SendMessageW(ui.bars[i], TBM_GETPOS, 0, 0) };
                    ui.change(i, value as i32);
                    ui.notice.clear();
                    unsafe {
                        InvalidateRect(hwnd, null(), 0);
                    }
                }
                return 0;
            }
            _ => {}
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

pub fn run() -> Result<(), String> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let common = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_BAR_CLASSES | ICC_STANDARD_CLASSES,
        };
        if InitCommonControlsEx(&common) == 0 {
            return Err(last_error());
        }
        let instance = GetModuleHandleW(null());
        let class = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hIcon: LoadIconW(null_mut(), IDI_APPLICATION),
            lpszClassName: CLASS,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(last_error());
        }
        let dpi = GetDpiForSystem().max(96) as i32;
        let ui = RefCell::new(Ui::new(dpi));
        let size = window_rect(688, 570, dpi);
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            CLASS,
            TITLE,
            STYLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            size.right - size.left,
            size.bottom - size.top,
            null_mut(),
            null_mut(),
            instance,
            (&ui as *const RefCell<Ui>).cast(),
        );
        if hwnd.is_null() {
            return Err(last_error());
        }
        if let Err(error) = ui.borrow_mut().controls(hwnd) {
            DestroyWindow(hwnd);
            return Err(error);
        }
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        SetFocus(ui.borrow().buttons[0]);
        let mut msg = MSG::default();
        loop {
            match GetMessageW(&mut msg, null_mut(), 0, 0) {
                -1 => {
                    let err = last_error();
                    DestroyWindow(hwnd);
                    return Err(err);
                }
                0 => break,
                _ => {
                    if IsDialogMessageW(hwnd, &msg) == 0 {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            }
        }
        Ok(())
    }
}
