#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hotkey;
mod image;
mod platform;
mod winhook;

use hotkey::{Hotkey, Hotkeys, DEFAULTS};
use image::{Area, Image, Mark, Point, Result, Tool};
use platform::*;
use std::mem::{size_of, zeroed};
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::{Dwm::*, Gdi::*};
use windows_sys::Win32::System::{LibraryLoader::*, Threading::*};
use windows_sys::Win32::UI::{
    Controls::*, HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*,
};

const MAIN_CLASS: &str = "FeatherShot.Main.1";
const OVERLAY_CLASS: &str = "FeatherShot.Overlay.1";
const EDITOR_CLASS: &str = "FeatherShot.Editor.1";
const HOTKEY_CLASS: &str = "FeatherShot.Hotkeys.1";
const TRAY_MESSAGE: u32 = WM_APP + 1;
const SHOW_MESSAGE: u32 = WM_APP + 2;
const CAPTURE_TIMER: usize = 1;
const REGION: u16 = 101;
const WINDOW: u16 = 102;
const SCREEN: u16 = 103;
const DELAY: u16 = 104;
const COPY_OPTION: u16 = 105;
const CURSOR_OPTION: u16 = 106;
const EDIT: u16 = 107;
const OPEN_IMAGE: u16 = 108;
const FOLDER: u16 = 109;
const COPY_LAST: u16 = 110;
const HIDE: u16 = 111;
const EXIT: u16 = 112;
const SHOW: u16 = 113;
const HOTKEYS: u16 = 114;
const STARTUP_OPTION: u16 = 115;
const APPLY_KEYS: u16 = 404;
const RESET_KEYS: u16 = 405;
const CANCEL_KEYS: u16 = 406;
const WIN_SNIP: u16 = 407;
const PRINT_SNIP: u16 = 408;
const BLOCK_SNIPPING: u16 = 409;
const ARROW: u16 = 201;
const RECTANGLE: u16 = 202;
const PEN: u16 = 203;
const REDACT: u16 = 204;
const UNDO: u16 = 205;
const REDO: u16 = 206;
const SAVE: u16 = 207;
const EDITOR_COPY: u16 = 208;
const COLOR: u16 = 209;
const WIDTH: u16 = 210;

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | ((g as u32) << 8) | ((b as u32) << 16)
}
fn bg() -> u32 {
    rgb(15, 19, 25)
}
fn panel() -> u32 {
    rgb(24, 30, 39)
}
fn border() -> u32 {
    rgb(46, 57, 69)
}
fn muted() -> u32 {
    rgb(153, 169, 184)
}
fn white() -> u32 {
    rgb(237, 243, 247)
}
fn accent() -> u32 {
    rgb(122, 232, 185)
}

#[derive(Clone)]
struct Settings {
    block_snipping: bool,
    delay: u32,
    no_copy: bool,
    cursor: bool,
    start_in_tray: bool,
    hotkeys: [Hotkey; 3],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            block_snipping: false,
            delay: 0,
            no_copy: false,
            cursor: false,
            start_in_tray: false,
            hotkeys: DEFAULTS,
        }
    }
}

impl Settings {
    fn load(path: &std::path::Path) -> Self {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let mut s = Self::default();
        for line in text.lines() {
            match line.split_once('=') {
                Some(("delay", n)) => s.delay = n.parse::<u32>().unwrap_or(0).min(5),
                Some(("copy", n)) => s.no_copy = n == "false",
                Some(("cursor", n)) => s.cursor = n == "true",
                Some(("start_in_tray", n)) => s.start_in_tray = n == "true",
                Some(("block_snipping", n)) => s.block_snipping = n == "true",
                Some(("hotkey_region", n)) => {
                    if let Some(key) = Hotkey::parse(n) {
                        s.hotkeys[0] = key;
                    }
                }
                Some(("hotkey_window", n)) => {
                    if let Some(key) = Hotkey::parse(n) {
                        s.hotkeys[1] = key;
                    }
                }
                Some(("hotkey_screen", n)) => {
                    if let Some(key) = Hotkey::parse(n) {
                        s.hotkeys[2] = key;
                    }
                }
                _ => (),
            }
        }
        s
    }
    fn save(&self, path: &std::path::Path) -> Result<()> {
        let temporary = path.with_extension("ini.tmp");
        std::fs::write(
            &temporary,
            format!(
                "delay={}\ncopy={}\ncursor={}\nstart_in_tray={}\nblock_snipping={}\nhotkey_region={}\nhotkey_window={}\nhotkey_screen={}\n",
                self.delay, !self.no_copy, self.cursor, self.start_in_tray, self.block_snipping, self.hotkeys[0].encode(), self.hotkeys[1].encode(), self.hotkeys[2].encode()
            ),
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, path).map_err(|e| {
            let _ = std::fs::remove_file(&temporary);
            e.to_string()
        })
    }
}

struct Fonts {
    title: HFONT,
    heading: HFONT,
    normal: HFONT,
    small: HFONT,
}
impl Fonts {
    unsafe fn new(dpi: u32) -> Self {
        unsafe fn font(size: i32, weight: u32, dpi: u32) -> HFONT {
            CreateFontW(
                -(size * dpi as i32 / 96),
                0,
                0,
                0,
                weight as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                DEFAULT_PITCH as u32,
                wide("Segoe UI").as_ptr(),
            )
        }
        Self {
            title: font(32, FW_SEMIBOLD, dpi),
            heading: font(17, FW_SEMIBOLD, dpi),
            normal: font(14, FW_NORMAL, dpi),
            small: font(12, FW_NORMAL, dpi),
        }
    }
}
impl Drop for Fonts {
    fn drop(&mut self) {
        unsafe {
            for font in [self.title, self.heading, self.normal, self.small] {
                DeleteObject(font);
            }
        }
    }
}

struct Canvas {
    dc: HDC,
    scale: f64,
}
impl Canvas {
    fn s(&self, value: i32) -> i32 {
        (value as f64 * self.scale).round() as i32
    }
    fn rect(&self, a: Area) -> RECT {
        RECT {
            left: self.s(a.x),
            top: self.s(a.y),
            right: self.s(a.x + a.w),
            bottom: self.s(a.y + a.h),
        }
    }
    unsafe fn fill(&self, a: Area, color: u32) {
        let brush = CreateSolidBrush(color);
        FillRect(self.dc, &self.rect(a), brush);
        DeleteObject(brush);
    }
    unsafe fn rounded(&self, a: Area, color: u32, line: u32, radius: i32) {
        let brush = CreateSolidBrush(color);
        let pen = CreatePen(PS_SOLID, 1, line);
        let old_b = SelectObject(self.dc, brush);
        let old_p = SelectObject(self.dc, pen);
        let r = self.rect(a);
        RoundRect(
            self.dc,
            r.left,
            r.top,
            r.right,
            r.bottom,
            self.s(radius),
            self.s(radius),
        );
        SelectObject(self.dc, old_p);
        SelectObject(self.dc, old_b);
        DeleteObject(pen);
        DeleteObject(brush);
    }
    unsafe fn text(&self, value: &str, a: Area, font: HFONT, color: u32, flags: u32) {
        let old = SelectObject(self.dc, font);
        SetBkMode(self.dc, TRANSPARENT as i32);
        SetTextColor(self.dc, color);
        let mut rect = self.rect(a);
        let text = wide(value);
        DrawTextW(
            self.dc,
            text.as_ptr(),
            (text.len() - 1) as i32,
            &mut rect,
            flags | DT_NOPREFIX,
        );
        SelectObject(self.dc, old);
    }
    unsafe fn image(&self, image: &Image, a: Area) {
        let r = self.rect(a);
        let info = bitmap_info(image.width, image.height);
        SetStretchBltMode(self.dc, HALFTONE as i32);
        SetBrushOrgEx(self.dc, 0, 0, null_mut());
        StretchDIBits(
            self.dc,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            0,
            0,
            image.width as i32,
            image.height as i32,
            image.pixels.as_ptr().cast(),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
    }
}

struct Overlay {
    image: Image,
    dimmed: Image,
    windows: Vec<Area>,
    selection: Area,
    anchor: Option<Point>,
    window_mode: bool,
}

struct Editor {
    fonts: Fonts,
    original: Image,
    rendered: Image,
    marks: Vec<Mark>,
    undone: Vec<Mark>,
    draft: Option<Mark>,
    tool: Tool,
    color: usize,
    radius: i32,
    view: Area,
    dirty: bool,
    status: String,
}

const COLORS: [[u8; 4]; 4] = [
    [75, 91, 255, 255],
    [185, 232, 122, 255],
    [90, 218, 255, 255],
    [255, 255, 255, 255],
];
impl Editor {
    fn redraw(&mut self) {
        self.rendered = self.original.clone();
        for mark in &self.marks {
            self.rendered.annotate(mark);
        }
        if let Some(mark) = &self.draft {
            self.rendered.annotate(mark);
        }
    }
    fn point(&self, p: Point) -> Point {
        Point {
            x: (((p.x - self.view.x) as f64 * self.original.width as f64
                / self.view.w.max(1) as f64) as i32)
                .clamp(0, self.original.width as i32 - 1),
            y: (((p.y - self.view.y) as f64 * self.original.height as f64
                / self.view.h.max(1) as f64) as i32)
                .clamp(0, self.original.height as i32 - 1),
        }
    }
}

struct App {
    main: HWND,
    overlay_hwnd: HWND,
    editor_hwnd: HWND,
    hotkeys_hwnd: HWND,
    hotkeys_fonts: Option<Fonts>,
    hotkeys_status: String,
    hotkeys_win: [bool; 3],
    hotkeys_block_snipping: bool,
    registered: Hotkeys,
    fonts: Fonts,
    dpi: u32,
    icon: HICON,
    data: PathBuf,
    screenshots: PathBuf,
    settings: Settings,
    overlay: Option<Overlay>,
    editor: Option<Editor>,
    last: Option<PathBuf>,
    preview: Option<Image>,
    dimensions: (u32, u32),
    status: String,
    busy: bool,
    pending: u16,
    return_main: bool,
    count: usize,
    hotkey_warning: String,
    tray_added: bool,
    taskbar_message: u32,
}

impl App {
    unsafe fn open_hotkeys(&mut self) {
        if !self.hotkeys_hwnd.is_null() {
            SetForegroundWindow(self.hotkeys_hwnd);
            return;
        }
        if !self.editor_hwnd.is_null() {
            SetForegroundWindow(self.editor_hwnd);
            return;
        }
        self.hotkeys_fonts = Some(Fonts::new(self.dpi));
        self.hotkeys_block_snipping = self.settings.block_snipping;
        self.hotkeys_win = self
            .settings
            .hotkeys
            .map(|key| key.modifiers & MOD_WIN != 0);
        self.hotkeys_status =
            "Tick Win to add the Windows key. Win shortcuts override the selected chord while FeatherShot is running."
                .into();
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: (610 * self.dpi / 96) as i32,
            bottom: (494 * self.dpi / 96) as i32,
        };
        AdjustWindowRectExForDpi(&mut rect, style, 0, 0, self.dpi);
        let mut parent: RECT = zeroed();
        GetWindowRect(self.main, &mut parent);
        let hwnd = CreateWindowExW(
            0,
            wide(HOTKEY_CLASS).as_ptr(),
            wide("FeatherShot - Keyboard shortcuts").as_ptr(),
            style,
            parent.left + (parent.right - parent.left - rect.right + rect.left) / 2,
            parent.top + (parent.bottom - parent.top - rect.bottom + rect.top) / 2,
            rect.right - rect.left,
            rect.bottom - rect.top,
            self.main,
            null_mut(),
            GetModuleHandleW(null()),
            self as *mut Self as _,
        );
        if hwnd.is_null() {
            self.hotkeys_fonts = None;
            self.status(os_error("Create shortcut settings"));
            return;
        }
        self.hotkeys_hwnd = hwnd;
        EnableWindow(self.main, 0);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetFocus(GetDlgItem(hwnd, 401));
    }

    unsafe fn hotkeys_command(&mut self, id: u16) {
        match id {
            APPLY_KEYS => {
                let keys = std::array::from_fn(|i| {
                    let mut key = Hotkey::from_control(SendMessageW(
                        GetDlgItem(self.hotkeys_hwnd, 401 + i as i32),
                        HKM_GETHOTKEY,
                        0,
                        0,
                    ) as usize);
                    if self.hotkeys_win[i] {
                        key.modifiers |= MOD_WIN;
                    }
                    key
                });
                match self.registered.replace_with_policy(
                    self.main,
                    &keys,
                    self.hotkeys_block_snipping,
                ) {
                    Ok(()) => {
                        self.settings.hotkeys = keys;
                        self.settings.block_snipping = self.hotkeys_block_snipping;
                        self.hotkey_warning.clear();
                        update_option_labels(self);
                        let mut tray: NOTIFYICONDATAW = zeroed();
                        tray.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
                        tray.hWnd = self.main;
                        tray.uID = 1;
                        tray.uFlags = NIF_TIP;
                        copy_wide(
                            &mut tray.szTip,
                            &format!("FeatherShot | {}", keys[0].display()),
                        );
                        if self.tray_added {
                            Shell_NotifyIconW(NIM_MODIFY, &tray);
                        }
                        match self.settings.save(&self.data.join("settings.ini")) {
                            Ok(()) => {
                                self.status("Keyboard shortcuts saved and applied.");
                                DestroyWindow(self.hotkeys_hwnd);
                                return;
                            }
                            Err(e) => {
                                self.hotkeys_status = format!(
                                    "Applied for this session, but could not save settings: {e}"
                                )
                            }
                        }
                    }
                    Err(e) => {
                        self.hotkeys_status = format!("{e} Existing shortcuts are unchanged.")
                    }
                }
            }
            RESET_KEYS => {
                self.hotkeys_block_snipping = false;
                SetWindowTextW(
                    GetDlgItem(self.hotkeys_hwnd, BLOCK_SNIPPING as i32),
                    wide("[ ] Block Snipping Tool shortcuts while running").as_ptr(),
                );
                self.hotkeys_win = [false; 3];
                for (i, key) in DEFAULTS.iter().enumerate() {
                    SendMessageW(
                        GetDlgItem(self.hotkeys_hwnd, 401 + i as i32),
                        HKM_SETHOTKEY,
                        key.control_value(),
                        0,
                    );
                }
                self.hotkeys_status = "Defaults loaded. Click Save shortcuts to apply them.".into();
                update_win_labels(self);
            }
            BLOCK_SNIPPING => {
                self.hotkeys_block_snipping = !self.hotkeys_block_snipping;
                SetWindowTextW(
                    GetDlgItem(self.hotkeys_hwnd, BLOCK_SNIPPING as i32),
                    wide(if self.hotkeys_block_snipping {
                        "[x] Block Snipping Tool shortcuts while running"
                    } else {
                        "[ ] Block Snipping Tool shortcuts while running"
                    })
                    .as_ptr(),
                );
                self.hotkeys_status = "Blocks PrtSc and Win+Shift+S while running. Assigned FeatherShot shortcuts still capture. Quit restores Windows behavior. Save to apply.".into();
            }
            411..=413 => {
                let index = (id - 411) as usize;
                self.hotkeys_win[index] = !self.hotkeys_win[index];
                update_win_labels(self);
            }
            WIN_SNIP => {
                self.hotkeys_win[0] = true;
                SendMessageW(
                    GetDlgItem(self.hotkeys_hwnd, 401),
                    HKM_SETHOTKEY,
                    Hotkey {
                        modifiers: MOD_SHIFT,
                        key: 0x53,
                    }
                    .control_value(),
                    0,
                );
                update_win_labels(self);
                self.hotkeys_status =
                    "Win+Shift+S selected for region capture. Click Save shortcuts to apply."
                        .into();
            }
            PRINT_SNIP => {
                self.hotkeys_win[2] = false;
                for index in 0..2 {
                    let input = GetDlgItem(self.hotkeys_hwnd, 401 + index as i32);
                    let key =
                        Hotkey::from_control(SendMessageW(input, HKM_GETHOTKEY, 0, 0) as usize);
                    if key.key == VK_SNAPSHOT as u32
                        && key.modifiers == 0
                        && !self.hotkeys_win[index]
                    {
                        SendMessageW(input, HKM_SETHOTKEY, 0, 0);
                    }
                }
                SendMessageW(
                    GetDlgItem(self.hotkeys_hwnd, 403),
                    HKM_SETHOTKEY,
                    Hotkey {
                        modifiers: 0,
                        key: VK_SNAPSHOT as u32,
                    }
                    .control_value(),
                    0,
                );
                update_win_labels(self);
                self.hotkeys_status =
                    "PrtSc selected for all screens. Click Save shortcuts to apply.".into();
            }
            CANCEL_KEYS => {
                DestroyWindow(self.hotkeys_hwnd);
                return;
            }
            _ => (),
        }
        InvalidateRect(self.hotkeys_hwnd, null(), 0);
    }

    unsafe fn notify(&self, message: &str) {
        if !self.tray_added {
            return;
        }
        let mut tray: NOTIFYICONDATAW = zeroed();
        tray.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        tray.hWnd = self.main;
        tray.uID = 1;
        tray.uFlags = NIF_INFO;
        copy_wide(&mut tray.szInfoTitle, "FeatherShot");
        copy_wide(&mut tray.szInfo, message);
        tray.dwInfoFlags = NIIF_INFO | NIIF_NOSOUND;
        Shell_NotifyIconW(NIM_MODIFY, &tray);
    }
    unsafe fn add_tray(&mut self) {
        let mut tray: NOTIFYICONDATAW = zeroed();
        tray.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        tray.hWnd = self.main;
        tray.uID = 1;
        tray.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        tray.uCallbackMessage = TRAY_MESSAGE;
        tray.hIcon = self.icon;
        copy_wide(
            &mut tray.szTip,
            &format!("FeatherShot | {}", self.settings.hotkeys[0].display()),
        );
        self.tray_added = Shell_NotifyIconW(NIM_ADD, &tray) != 0;
    }
    unsafe fn status(&mut self, value: impl Into<String>) {
        self.status = value.into();
        InvalidateRect(self.main, null(), 0);
    }
    unsafe fn show(&self) {
        ShowWindow(self.main, SW_RESTORE);
        SetForegroundWindow(if !self.hotkeys_hwnd.is_null() {
            self.hotkeys_hwnd
        } else if !self.editor_hwnd.is_null() {
            self.editor_hwnd
        } else {
            self.main
        });
    }
    unsafe fn finish(&mut self) {
        self.busy = false;
        if self.return_main {
            self.show();
        }
    }
    unsafe fn fail(&mut self, error: &str) {
        self.finish();
        self.status(format!("Error: {error}"));
        self.notify(error);
    }
    unsafe fn save_image(&mut self, image: &Image, edited: bool) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.screenshots)
            .map_err(|e| format!("Create Screenshots folder: {e}"))?;
        let path = new_capture_path(&self.screenshots, edited);
        image.save_png(&path)?;
        self.dimensions = (image.width, image.height);
        self.preview = Some(image.thumbnail(480, 290));
        self.last = Some(path.clone());
        self.count += 1;
        let mut status = format!("Saved {} x {} PNG", image.width, image.height);
        if !self.settings.no_copy {
            match copy_image(self.main, image) {
                Ok(()) => status.push_str("  |  Copied to clipboard"),
                Err(e) => {
                    status.push_str("  |  Clipboard busy: use Copy again");
                    self.notify(&e);
                }
            }
        }
        self.status(status);
        if !self.return_main && !edited {
            self.notify("Screenshot saved in Screenshots.");
        }
        for id in [EDIT, COPY_LAST, OPEN_IMAGE] {
            EnableWindow(GetDlgItem(self.main, id as i32), 1);
        }
        Ok(path)
    }
    unsafe fn start_capture(&mut self, mode: u16) {
        if !self.hotkeys_hwnd.is_null() {
            SetForegroundWindow(self.hotkeys_hwnd);
            return;
        }
        if self.busy {
            return;
        }
        if !self.editor_hwnd.is_null() {
            SetForegroundWindow(self.editor_hwnd);
            return;
        }
        self.busy = true;
        self.pending = mode;
        self.return_main = IsWindowVisible(self.main) != 0 && IsIconic(self.main) == 0;
        ShowWindow(self.main, SW_HIDE);
        // One-shot timer lets the compositor remove our window before capture.
        if SetTimer(
            self.main,
            CAPTURE_TIMER,
            180 + self.settings.delay * 1000,
            None,
        ) == 0
        {
            self.fail("Unable to schedule capture");
        }
    }
    unsafe fn take_capture(&mut self) {
        KillTimer(self.main, CAPTURE_TIMER);
        let bounds = virtual_screen();
        let image = match capture(bounds, self.settings.cursor) {
            Ok(image) => image,
            Err(e) => {
                self.fail(&e);
                return;
            }
        };
        if self.pending == SCREEN {
            if let Err(e) = self.save_image(&image, false) {
                self.fail(&e);
                return;
            }
            self.finish();
            return;
        }
        let windows = if self.pending == WINDOW {
            window_areas(self.main, bounds)
        } else {
            Vec::new()
        };
        let mut dimmed = image.clone();
        for pixel in dimmed.pixels.chunks_exact_mut(4) {
            for c in &mut pixel[..3] {
                *c = (*c as u16 * 42 / 100) as u8;
            }
        }
        self.overlay = Some(Overlay {
            image,
            dimmed,
            windows,
            selection: Area::default(),
            anchor: None,
            window_mode: self.pending == WINDOW,
        });
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            wide(OVERLAY_CLASS).as_ptr(),
            wide("FeatherShot - Select capture").as_ptr(),
            WS_POPUP,
            bounds.x,
            bounds.y,
            bounds.w,
            bounds.h,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            self as *mut Self as _,
        );
        if hwnd.is_null() {
            self.overlay = None;
            self.fail(&os_error("Create capture overlay"));
            return;
        }
        self.overlay_hwnd = hwnd;
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetFocus(hwnd);
    }
    unsafe fn end_overlay(&mut self, accept: bool) {
        let overlay = self.overlay.take();
        let hwnd = self.overlay_hwnd;
        self.overlay_hwnd = null_mut();
        ReleaseCapture();
        if !hwnd.is_null() {
            DestroyWindow(hwnd);
        }
        if let Some(overlay) = overlay {
            if accept && overlay.selection.w > 1 && overlay.selection.h > 1 {
                match overlay
                    .image
                    .crop(overlay.selection)
                    .and_then(|image| self.save_image(&image, false))
                {
                    Ok(_) => (),
                    Err(e) => {
                        self.fail(&e);
                        return;
                    }
                }
            } else {
                self.status("Capture cancelled. Ready when you are.");
            }
        }
        self.finish();
    }
    unsafe fn open_editor(&mut self) {
        if !self.editor_hwnd.is_null() {
            SetForegroundWindow(self.editor_hwnd);
            return;
        }
        let Some(path) = &self.last else {
            return;
        };
        let image = match Image::load_png(path) {
            Ok(i) => i,
            Err(e) => {
                self.status(e);
                return;
            }
        };
        self.editor = Some(Editor {
            fonts: Fonts::new(self.dpi),
            rendered: image.clone(),
            original: image,
            marks: Vec::new(),
            undone: Vec::new(),
            draft: None,
            tool: Tool::Arrow,
            color: 0,
            radius: 2,
            view: Area::default(),
            dirty: false,
            status: "Draw on the image. Ctrl+S saves a new PNG. Ctrl+C copies. Ctrl+Z undoes."
                .into(),
        });
        let scale = self.dpi as f64 / 96.0;
        let mut work: RECT = zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut work as *mut RECT).cast(), 0);
        let w = ((1040.0 * scale) as i32).min(work.right - work.left);
        let h = ((760.0 * scale) as i32).min(work.bottom - work.top);
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            wide(EDITOR_CLASS).as_ptr(),
            wide("FeatherShot - Annotate").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            work.left + (work.right - work.left - w) / 2,
            work.top + (work.bottom - work.top - h) / 2,
            w,
            h,
            self.main,
            null_mut(),
            GetModuleHandleW(null()),
            self as *mut Self as _,
        );
        if hwnd.is_null() {
            self.editor = None;
            self.status(os_error("Create editor"));
            return;
        }
        self.editor_hwnd = hwnd;
        EnableWindow(self.main, 0);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
    }
    unsafe fn editor_command(&mut self, id: u16) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        match id {
            ARROW => editor.tool = Tool::Arrow,
            RECTANGLE => editor.tool = Tool::Rectangle,
            PEN => editor.tool = Tool::Pen,
            REDACT => {
                editor.tool = Tool::Redact;
                editor.status = "Solid redaction. Save creates a new PNG; the original capture remains in Screenshots.".into();
            }
            COLOR => editor.color = (editor.color + 1) % COLORS.len(),
            WIDTH => {
                editor.radius = match editor.radius {
                    2 => 4,
                    4 => 7,
                    _ => 2,
                }
            }
            UNDO => {
                if let Some(mark) = editor.marks.pop() {
                    editor.undone.push(mark);
                    editor.dirty = true;
                    editor.redraw();
                }
            }
            REDO => {
                if let Some(mark) = editor.undone.pop() {
                    editor.marks.push(mark);
                    editor.dirty = true;
                    editor.redraw();
                }
            }
            SAVE => {
                let image = editor.rendered.clone();
                match self.save_image(&image, true) {
                    Ok(path) => {
                        if let Some(editor) = &mut self.editor {
                            editor.dirty = false;
                            editor.status = format!(
                                "Saved new file: {}",
                                path.file_name().unwrap_or_default().to_string_lossy()
                            );
                        }
                    }
                    Err(e) => {
                        if let Some(editor) = &mut self.editor {
                            editor.status = format!("Save failed: {e}");
                        }
                    }
                }
            }
            EDITOR_COPY => {
                editor.status = match copy_image(self.main, &editor.rendered) {
                    Ok(()) => "Image copied. Paste with Ctrl+V.".into(),
                    Err(e) => e,
                };
            }
            _ => (),
        }
        InvalidateRect(self.editor_hwnd, null(), 0);
        for id in ARROW..=WIDTH {
            InvalidateRect(GetDlgItem(self.editor_hwnd, id as i32), null(), 0);
        }
    }
    unsafe fn command(&mut self, id: u16) {
        match id {
            REGION | WINDOW | SCREEN => self.start_capture(id),
            DELAY | COPY_OPTION | CURSOR_OPTION => {
                match id {
                    DELAY => {
                        self.settings.delay = match self.settings.delay {
                            0 => 3,
                            3 => 5,
                            _ => 0,
                        }
                    }
                    COPY_OPTION => self.settings.no_copy = !self.settings.no_copy,
                    _ => self.settings.cursor = !self.settings.cursor,
                }
                if let Err(e) = self.settings.save(&self.data.join("settings.ini")) {
                    self.status(format!("Could not save settings: {e}"));
                }
                update_option_labels(self);
            }
            EDIT => self.open_editor(),
            HOTKEYS => self.open_hotkeys(),
            STARTUP_OPTION => {
                let mut settings = self.settings.clone();
                settings.start_in_tray = !settings.start_in_tray;
                match settings.save(&self.data.join("settings.ini")) {
                    Ok(()) => {
                        self.settings = settings;
                        self.status(if self.settings.start_in_tray {
                            "Saved. Next launch: system tray. Click the tray icon to show the window."
                        } else {
                            "Saved. Next launch: show the main window."
                        });
                        update_option_labels(self);
                    }
                    Err(e) => self.status(format!("Launch preference was not changed: {e}")),
                }
            }
            COPY_LAST => {
                if let Some(path) = &self.last {
                    match Image::load_png(path).and_then(|i| copy_image(self.main, &i)) {
                        Ok(()) => self.status("Image copied. Paste with Ctrl+V."),
                        Err(e) => self.status(e),
                    }
                }
            }
            OPEN_IMAGE => {
                if let Some(path) = &self.last {
                    if let Err(e) = open_path(path) {
                        self.status(e);
                    }
                }
            }
            FOLDER => {
                match std::fs::create_dir_all(&self.screenshots)
                    .map_err(|e| e.to_string())
                    .and_then(|_| open_path(&self.screenshots))
                {
                    Ok(()) => (),
                    Err(e) => self.status(e),
                }
            }
            HIDE => {
                if self.tray_added {
                    ShowWindow(self.main, SW_HIDE);
                    self.notify(&format!("Running in the tray. {} selects a region. Right-click the tray icon to quit.", self.settings.hotkeys[0].display()));
                } else {
                    ShowWindow(self.main, SW_MINIMIZE);
                }
            }
            SHOW => self.show(),
            EXIT => {
                if !self.hotkeys_hwnd.is_null() {
                    DestroyWindow(self.hotkeys_hwnd);
                }
                if !self.editor_hwnd.is_null() {
                    SendMessageW(self.editor_hwnd, WM_CLOSE, 0, 0);
                }
                if self.editor_hwnd.is_null() {
                    DestroyWindow(self.main);
                }
            }
            _ => (),
        }
    }
}

fn copy_wide<const N: usize>(dest: &mut [u16; N], value: &str) {
    for (out, c) in dest[..N - 1].iter_mut().zip(value.encode_utf16()) {
        *out = c;
    }
}

unsafe fn app_ptr(hwnd: HWND, message: u32, lparam: LPARAM) -> *mut App {
    if message == WM_NCCREATE {
        let create = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App
}

unsafe fn button(parent: HWND, id: u16, label: &str, a: Area, scale: f64) {
    CreateWindowExW(
        0,
        wide("BUTTON").as_ptr(),
        wide(label).as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        (a.x as f64 * scale) as i32,
        (a.y as f64 * scale) as i32,
        (a.w as f64 * scale) as i32,
        (a.h as f64 * scale) as i32,
        parent,
        id as usize as HMENU,
        GetModuleHandleW(null()),
        null(),
    );
}

unsafe fn update_option_labels(app: &App) {
    for (id, label) in [
        (DELAY, format!("Delay: {} s", app.settings.delay)),
        (
            STARTUP_OPTION,
            format!(
                "On launch: {}",
                if app.settings.start_in_tray {
                    "System tray"
                } else {
                    "Show window"
                }
            ),
        ),
        (
            COPY_OPTION,
            format!(
                "{} Auto-copy",
                if app.settings.no_copy { "[ ]" } else { "[x]" }
            ),
        ),
        (
            CURSOR_OPTION,
            format!(
                "{} Include cursor",
                if app.settings.cursor { "[x]" } else { "[ ]" }
            ),
        ),
    ] {
        SetWindowTextW(GetDlgItem(app.main, id as i32), wide(&label).as_ptr());
    }
    for (index, (id, title)) in [
        (REGION, "Select region"),
        (WINDOW, "Pick a window"),
        (SCREEN, "All screens"),
    ]
    .into_iter()
    .enumerate()
    {
        SetWindowTextW(
            GetDlgItem(app.main, id as i32),
            wide(&format!(
                "{}\n{}",
                title,
                app.settings.hotkeys[index].display()
            ))
            .as_ptr(),
        );
    }
}

unsafe fn main_controls(app: &App) {
    let s = app.dpi as f64 / 96.0;
    for (id, label, a) in [
        (
            REGION,
            "Select region\nCtrl + Shift + S",
            Area {
                x: 32,
                y: 126,
                w: 244,
                h: 82,
            },
        ),
        (
            WINDOW,
            "Pick a window\nCtrl + Shift + W",
            Area {
                x: 288,
                y: 126,
                w: 244,
                h: 82,
            },
        ),
        (
            SCREEN,
            "All screens\nCtrl + Shift + F",
            Area {
                x: 544,
                y: 126,
                w: 244,
                h: 82,
            },
        ),
        (
            DELAY,
            "Delay: 0 s",
            Area {
                x: 32,
                y: 224,
                w: 136,
                h: 38,
            },
        ),
        (
            COPY_OPTION,
            "[x] Auto-copy",
            Area {
                x: 180,
                y: 224,
                w: 180,
                h: 38,
            },
        ),
        (
            CURSOR_OPTION,
            "[ ] Include cursor",
            Area {
                x: 372,
                y: 224,
                w: 180,
                h: 38,
            },
        ),
        (
            EDIT,
            "Annotate",
            Area {
                x: 340,
                y: 430,
                w: 134,
                h: 38,
            },
        ),
        (
            COPY_LAST,
            "Copy",
            Area {
                x: 486,
                y: 430,
                w: 118,
                h: 38,
            },
        ),
        (
            OPEN_IMAGE,
            "Open image",
            Area {
                x: 616,
                y: 430,
                w: 142,
                h: 38,
            },
        ),
        (
            FOLDER,
            "Open screenshots folder",
            Area {
                x: 340,
                y: 480,
                w: 270,
                h: 34,
            },
        ),
        (
            HIDE,
            "Hide to tray",
            Area {
                x: 32,
                y: 612,
                w: 144,
                h: 34,
            },
        ),
        (
            EXIT,
            "Quit",
            Area {
                x: 694,
                y: 612,
                w: 94,
                h: 34,
            },
        ),
    ] {
        button(app.main, id, label, a, s);
    }
    button(
        app.main,
        HOTKEYS,
        "Customize hotkeys",
        Area {
            x: 564,
            y: 224,
            w: 224,
            h: 38,
        },
        s,
    );
    button(
        app.main,
        STARTUP_OPTION,
        "On launch: Show window",
        Area {
            x: 190,
            y: 612,
            w: 488,
            h: 34,
        },
        s,
    );
    for id in [EDIT, COPY_LAST, OPEN_IMAGE] {
        EnableWindow(GetDlgItem(app.main, id as i32), app.last.is_some() as i32);
    }
    update_option_labels(app);
}

unsafe fn draw_button(app: &App, item: &DRAWITEMSTRUCT) {
    let dc = item.hDC;
    let mut label = [0u16; 180];
    let n = GetWindowTextW(item.hwndItem, label.as_mut_ptr(), label.len() as i32);
    let label = String::from_utf16_lossy(&label[..n as usize]);
    let disabled = item.itemState & ODS_DISABLED != 0;
    let pressed = item.itemState & ODS_SELECTED != 0;
    let id = item.CtlID as u16;
    let fonts = if id >= 400 {
        app.hotkeys_fonts.as_ref().unwrap_or(&app.fonts)
    } else if id >= ARROW {
        app.editor.as_ref().map(|e| &e.fonts).unwrap_or(&app.fonts)
    } else {
        &app.fonts
    };
    let tool_selected = app.editor.as_ref().is_some_and(|e| {
        matches!(
            (id, e.tool),
            (ARROW, Tool::Arrow)
                | (RECTANGLE, Tool::Rectangle)
                | (PEN, Tool::Pen)
                | (REDACT, Tool::Redact)
        )
    });
    let primary = id == REGION || id == SAVE || tool_selected;
    let fill = if disabled {
        rgb(28, 33, 42)
    } else if pressed {
        rgb(54, 98, 80)
    } else if primary {
        rgb(32, 65, 55)
    } else {
        rgb(32, 40, 51)
    };
    let line = if primary { accent() } else { border() };
    let s = GetDpiForWindow(item.hwndItem).max(96) as f64 / 96.0;
    let c = Canvas { dc, scale: 1.0 };
    let rect = item.rcItem;
    c.fill(
        Area {
            x: rect.left,
            y: rect.top,
            w: rect.right - rect.left,
            h: rect.bottom - rect.top,
        },
        bg(),
    );
    c.rounded(
        Area {
            x: rect.left,
            y: rect.top,
            w: rect.right - rect.left,
            h: rect.bottom - rect.top,
        },
        fill,
        line,
        (12.0 * s) as i32,
    );
    let lines: Vec<_> = label.split('\n').collect();
    let text_color = if disabled {
        rgb(85, 99, 112)
    } else if primary {
        accent()
    } else {
        white()
    };
    if lines.len() > 1 {
        c.text(
            lines[0],
            Area {
                x: rect.left + 8,
                y: rect.top + (17.0 * s) as i32,
                w: rect.right - rect.left - 16,
                h: (26.0 * s) as i32,
            },
            fonts.heading,
            text_color,
            DT_CENTER | DT_SINGLELINE,
        );
        c.text(
            lines[1],
            Area {
                x: rect.left + 8,
                y: rect.top + (46.0 * s) as i32,
                w: rect.right - rect.left - 16,
                h: (24.0 * s) as i32,
            },
            fonts.small,
            muted(),
            DT_CENTER | DT_SINGLELINE,
        );
    } else {
        c.text(
            &label,
            Area {
                x: rect.left + 6,
                y: rect.top,
                w: rect.right - rect.left - 12,
                h: rect.bottom - rect.top,
            },
            fonts.normal,
            text_color,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
    }
    if item.itemState & ODS_FOCUS != 0 {
        let inset = (4.0 * s) as i32;
        let r = RECT {
            left: rect.left + inset,
            top: rect.top + inset,
            right: rect.right - inset,
            bottom: rect.bottom - inset,
        };
        DrawFocusRect(dc, &r);
    }
}

unsafe fn paint_main(app: &App, dc: HDC, client: RECT) {
    let c = Canvas {
        dc,
        scale: app.dpi as f64 / 96.0,
    };
    FillRect(dc, &client, GetStockObject(BLACK_BRUSH) as HBRUSH);
    c.fill(
        Area {
            x: 0,
            y: 0,
            w: 820,
            h: 674,
        },
        bg(),
    );
    c.rounded(
        Area {
            x: 32,
            y: 30,
            w: 46,
            h: 46,
        },
        rgb(32, 65, 55),
        accent(),
        14,
    );
    c.rounded(
        Area {
            x: 42,
            y: 42,
            w: 26,
            h: 20,
        },
        rgb(32, 65, 55),
        accent(),
        6,
    );
    c.text(
        "+",
        Area {
            x: 46,
            y: 39,
            w: 20,
            h: 24,
        },
        app.fonts.heading,
        accent(),
        DT_CENTER,
    );
    c.text(
        "FeatherShot",
        Area {
            x: 94,
            y: 22,
            w: 340,
            h: 44,
        },
        app.fonts.title,
        white(),
        DT_SINGLELINE,
    );
    c.text(
        "Capture fast. Stay light.",
        Area {
            x: 96,
            y: 69,
            w: 340,
            h: 24,
        },
        app.fonts.normal,
        muted(),
        DT_SINGLELINE,
    );
    c.rounded(
        Area {
            x: 664,
            y: 36,
            w: 124,
            h: 28,
        },
        panel(),
        border(),
        12,
    );
    c.text(
        "NATIVE RUST",
        Area {
            x: 664,
            y: 41,
            w: 124,
            h: 20,
        },
        app.fonts.small,
        accent(),
        DT_CENTER | DT_SINGLELINE,
    );
    c.text(
        "CAPTURE",
        Area {
            x: 34,
            y: 102,
            w: 300,
            h: 20,
        },
        app.fonts.small,
        muted(),
        DT_SINGLELINE,
    );
    c.rounded(
        Area {
            x: 32,
            y: 286,
            w: 756,
            h: 248,
        },
        panel(),
        border(),
        18,
    );
    c.text(
        "LATEST CAPTURE",
        Area {
            x: 50,
            y: 304,
            w: 270,
            h: 20,
        },
        app.fonts.small,
        muted(),
        DT_SINGLELINE,
    );
    c.rounded(
        Area {
            x: 50,
            y: 338,
            w: 264,
            h: 176,
        },
        bg(),
        border(),
        10,
    );
    if let Some(image) = &app.preview {
        let ratio = (250.0 / image.width as f64).min(162.0 / image.height as f64);
        let w = (image.width as f64 * ratio) as i32;
        let h = (image.height as f64 * ratio) as i32;
        c.image(
            image,
            Area {
                x: 57 + (250 - w) / 2,
                y: 345 + (162 - h) / 2,
                w,
                h,
            },
        );
        c.text(
            &format!("{} x {} pixels", app.dimensions.0, app.dimensions.1),
            Area {
                x: 340,
                y: 340,
                w: 420,
                h: 30,
            },
            app.fonts.heading,
            white(),
            DT_SINGLELINE,
        );
        let name = app
            .last
            .as_ref()
            .and_then(|p| p.file_name())
            .unwrap_or_default()
            .to_string_lossy();
        c.text(
            &name,
            Area {
                x: 340,
                y: 379,
                w: 420,
                h: 28,
            },
            app.fonts.small,
            muted(),
            DT_SINGLELINE | DT_END_ELLIPSIS,
        );
    } else {
        c.text(
            "Your next great capture",
            Area {
                x: 62,
                y: 400,
                w: 240,
                h: 26,
            },
            app.fonts.normal,
            muted(),
            DT_CENTER | DT_SINGLELINE,
        );
        c.text(
            "Ready for your first shot",
            Area {
                x: 340,
                y: 342,
                w: 420,
                h: 30,
            },
            app.fonts.heading,
            white(),
            DT_SINGLELINE,
        );
        c.text(
            "Select a region, choose a window, or capture every screen.",
            Area {
                x: 340,
                y: 380,
                w: 414,
                h: 42,
            },
            app.fonts.normal,
            muted(),
            DT_WORDBREAK,
        );
    }
    c.rounded(
        Area {
            x: 32,
            y: 550,
            w: 756,
            h: 46,
        },
        rgb(21, 38, 34),
        rgb(37, 70, 56),
        12,
    );
    c.text(
        &app.status,
        Area {
            x: 46,
            y: 550,
            w: 728,
            h: 46,
        },
        app.fonts.small,
        accent(),
        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
    );
    let footer = if app.hotkey_warning.is_empty() {
        "Portable  /  Local PNG files  /  No background polling"
    } else {
        &app.hotkey_warning
    };
    c.text(
        footer,
        Area {
            x: 190,
            y: 650,
            w: 488,
            h: 22,
        },
        app.fonts.small,
        muted(),
        DT_CENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );
}

unsafe fn paint_buffered(hwnd: HWND, paint: impl FnOnce(HDC, RECT)) {
    let mut ps: PAINTSTRUCT = zeroed();
    let target = BeginPaint(hwnd, &mut ps);
    let mut client: RECT = zeroed();
    GetClientRect(hwnd, &mut client);
    let memory = CreateCompatibleDC(target);
    let bitmap = CreateCompatibleBitmap(target, client.right.max(1), client.bottom.max(1));
    if memory.is_null() || bitmap.is_null() {
        paint(target, client);
    } else {
        let old = SelectObject(memory, bitmap);
        paint(memory, client);
        BitBlt(
            target,
            0,
            0,
            client.right,
            client.bottom,
            memory,
            0,
            0,
            SRCCOPY,
        );
        SelectObject(memory, old);
    }
    if !bitmap.is_null() {
        DeleteObject(bitmap);
    }
    if !memory.is_null() {
        DeleteDC(memory);
    }
    EndPaint(hwnd, &ps);
}

unsafe extern "system" fn main_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let ptr = app_ptr(hwnd, message, l);
    if ptr.is_null() {
        return DefWindowProcW(hwnd, message, w, l);
    }
    let app = &mut *ptr;
    if message == app.taskbar_message && message != 0 {
        app.add_tray();
        return 0;
    }
    match message {
        WM_CREATE => {
            app.main = hwnd;
            main_controls(app);
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint_buffered(hwnd, |dc, rect| paint_main(app, dc, rect));
            0
        }
        WM_DRAWITEM => {
            draw_button(app, &*(l as *const DRAWITEMSTRUCT));
            1
        }
        WM_COMMAND => {
            app.command((w & 0xffff) as u16);
            0
        }
        WM_HOTKEY => {
            if let Some(index) = app.registered.action(w) {
                if !app.hotkeys_hwnd.is_null() {
                    // Existing global shortcuts are intercepted by Windows. Feed the
                    // selected combination to the focused recorder so modes can swap keys.
                    let focus = GetFocus();
                    let id = GetDlgCtrlID(focus);
                    if (401..=403).contains(&id) && GetParent(focus) == app.hotkeys_hwnd {
                        SendMessageW(
                            focus,
                            HKM_SETHOTKEY,
                            app.settings.hotkeys[index].control_value(),
                            0,
                        );
                        app.hotkeys_win[(id - 401) as usize] =
                            app.settings.hotkeys[index].modifiers & MOD_WIN != 0;
                        update_win_labels(app);
                    }
                } else {
                    app.command([REGION, WINDOW, SCREEN][index]);
                }
            }
            0
        }
        WM_TIMER if w == CAPTURE_TIMER => {
            app.take_capture();
            0
        }
        WM_CLOSE => {
            app.command(HIDE);
            0
        }
        SHOW_MESSAGE => {
            app.show();
            0
        }
        WM_SIZE if w == SIZE_MINIMIZED as usize && app.tray_added => {
            ShowWindow(hwnd, SW_HIDE);
            0
        }
        WM_DPICHANGED => {
            app.dpi = (w & 0xffff) as u32;
            app.fonts = Fonts::new(app.dpi);
            let rect = &*(l as *const RECT);
            SetWindowPos(
                hwnd,
                null_mut(),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            for id in REGION..=STARTUP_OPTION {
                let child = GetDlgItem(hwnd, id as i32);
                if !child.is_null() {
                    DestroyWindow(child);
                }
            }
            main_controls(app);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        TRAY_MESSAGE => {
            match l as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => app.show(),
                WM_RBUTTONUP | WM_CONTEXTMENU => {
                    let menu = CreatePopupMenu();
                    for (id, text) in [
                        (
                            REGION,
                            format!("Select region\t{}", app.settings.hotkeys[0].display()),
                        ),
                        (
                            WINDOW,
                            format!("Pick a window\t{}", app.settings.hotkeys[1].display()),
                        ),
                        (
                            SCREEN,
                            format!("All screens\t{}", app.settings.hotkeys[2].display()),
                        ),
                        (HOTKEYS, "Customize hotkeys".into()),
                        (FOLDER, "Open screenshots folder".into()),
                        (SHOW, "Show FeatherShot".into()),
                        (EXIT, "Quit".into()),
                    ] {
                        AppendMenuW(menu, MF_STRING, id as usize, wide(&text).as_ptr());
                    }
                    let mut p: POINT = zeroed();
                    GetCursorPos(&mut p);
                    SetForegroundWindow(hwnd);
                    let id = TrackPopupMenu(
                        menu,
                        TPM_RETURNCMD | TPM_RIGHTBUTTON,
                        p.x,
                        p.y,
                        0,
                        hwnd,
                        null(),
                    );
                    DestroyMenu(menu);
                    PostMessageW(hwnd, WM_NULL, 0, 0);
                    if id != 0 {
                        app.command(id as u16);
                    }
                }
                _ => (),
            }
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, CAPTURE_TIMER);
            app.registered.clear(hwnd);
            let mut tray: NOTIFYICONDATAW = zeroed();
            tray.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            tray.hWnd = hwnd;
            tray.uID = 1;
            Shell_NotifyIconW(NIM_DELETE, &tray);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, message, w, l),
    }
}

fn mouse_point(l: LPARAM) -> Point {
    Point {
        x: (l as u16 as i16) as i32,
        y: ((l >> 16) as u16 as i16) as i32,
    }
}

unsafe fn paint_overlay(app: &App, dc: HDC, _rect: RECT) {
    let Some(overlay) = &app.overlay else {
        return;
    };
    let c = Canvas { dc, scale: 1.0 };
    c.image(&overlay.dimmed, overlay.image.bounds());
    let a = overlay.selection;
    if a.w > 0 && a.h > 0 {
        let saved = SaveDC(dc);
        IntersectClipRect(dc, a.x, a.y, a.x + a.w, a.y + a.h);
        c.image(&overlay.image, overlay.image.bounds());
        RestoreDC(dc, saved);
        let pen = CreatePen(PS_SOLID, 2, accent());
        let old_pen = SelectObject(dc, pen);
        let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
        Rectangle(dc, a.x, a.y, a.x + a.w, a.y + a.h);
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        DeleteObject(pen);
    }
    let mut cursor: POINT = zeroed();
    GetCursorPos(&mut cursor);
    ScreenToClient(app.overlay_hwnd, &mut cursor);
    let text = if overlay.window_mode {
        "Click a window to capture  |  Esc / Right-click to cancel"
    } else {
        "Drag to select a region  |  Esc / Right-click to cancel"
    };
    let s = app.dpi as f64 / 96.0;
    let width = (450.0 * s) as i32;
    let height = (62.0 * s) as i32;
    let x = (cursor.x + 22)
        .min(overlay.image.width as i32 - width - 8)
        .max(8);
    let y = (cursor.y + 28)
        .min(overlay.image.height as i32 - height - 8)
        .max(8);
    c.rounded(
        Area {
            x,
            y,
            w: width,
            h: height,
        },
        bg(),
        border(),
        12,
    );
    c.text(
        text,
        Area {
            x: x + 12,
            y: y + 9,
            w: width - 24,
            h: height / 2,
        },
        app.fonts.small,
        white(),
        DT_SINGLELINE,
    );
    c.text(
        &format!("{} x {} px", a.w, a.h),
        Area {
            x: x + 12,
            y: y + height / 2,
            w: width - 24,
            h: height / 2,
        },
        app.fonts.small,
        accent(),
        DT_SINGLELINE,
    );
}

unsafe extern "system" fn overlay_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let ptr = app_ptr(hwnd, message, l);
    if ptr.is_null() {
        return DefWindowProcW(hwnd, message, w, l);
    }
    let app = &mut *ptr;
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint_buffered(hwnd, |dc, rect| paint_overlay(app, dc, rect));
            0
        }
        WM_SETCURSOR => {
            SetCursor(LoadCursorW(null_mut(), IDC_CROSS));
            1
        }
        WM_LBUTTONDOWN => {
            if let Some(overlay) = &mut app.overlay {
                if overlay.window_mode {
                    let p = mouse_point(l);
                    overlay.selection = overlay
                        .windows
                        .iter()
                        .copied()
                        .find(|a| a.contains(p))
                        .unwrap_or_default();
                    app.end_overlay(true);
                } else {
                    overlay.anchor = Some(mouse_point(l));
                    SetCapture(hwnd);
                }
            }
            0
        }
        WM_MOUSEMOVE => {
            if let Some(overlay) = &mut app.overlay {
                let p = mouse_point(l);
                overlay.selection = if overlay.window_mode {
                    overlay
                        .windows
                        .iter()
                        .copied()
                        .find(|a| a.contains(p))
                        .unwrap_or_default()
                } else if let Some(anchor) = overlay.anchor {
                    Area::between(anchor, p).intersect(overlay.image.bounds())
                } else {
                    overlay.selection
                };
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_LBUTTONUP => {
            if let Some(overlay) = &mut app.overlay {
                if !overlay.window_mode {
                    if let Some(anchor) = overlay.anchor {
                        overlay.selection =
                            Area::between(anchor, mouse_point(l)).intersect(overlay.image.bounds());
                    }
                    app.end_overlay(true);
                }
            }
            0
        }
        WM_RBUTTONDOWN | WM_CLOSE => {
            app.end_overlay(false);
            0
        }
        WM_KEYDOWN if w == VK_ESCAPE as usize => {
            app.end_overlay(false);
            0
        }
        WM_DISPLAYCHANGE => {
            app.end_overlay(false);
            0
        }
        WM_ACTIVATE if w & 0xffff == WA_INACTIVE as usize => {
            if !app.overlay_hwnd.is_null() {
                app.end_overlay(false);
            }
            0
        }
        _ => DefWindowProcW(hwnd, message, w, l),
    }
}

unsafe fn editor_controls(hwnd: HWND, app: &App) {
    let s = GetDpiForWindow(hwnd).max(96) as f64 / 96.0;
    for (i, (id, text)) in [
        (ARROW, "Arrow [1]"),
        (RECTANGLE, "Box [2]"),
        (PEN, "Pen [3]"),
        (REDACT, "Redact [4]"),
        (COLOR, "Color"),
        (WIDTH, "Width"),
        (UNDO, "Undo"),
        (REDO, "Redo"),
    ]
    .into_iter()
    .enumerate()
    {
        button(
            hwnd,
            id,
            text,
            Area {
                x: 16 + i as i32 * 92,
                y: 16,
                w: 84,
                h: 36,
            },
            s,
        );
    }
    button(
        hwnd,
        SAVE,
        "Save PNG",
        Area {
            x: 16,
            y: 66,
            w: 132,
            h: 34,
        },
        s,
    );
    button(
        hwnd,
        EDITOR_COPY,
        "Copy image",
        Area {
            x: 160,
            y: 66,
            w: 132,
            h: 34,
        },
        s,
    );
    let _ = app;
}

unsafe fn paint_editor(app: &mut App, hwnd: HWND, dc: HDC, rect: RECT) {
    let Some(editor) = &mut app.editor else {
        return;
    };
    let dpi = GetDpiForWindow(hwnd).max(96);
    let s = dpi as f64 / 96.0;
    let c = Canvas { dc, scale: 1.0 };
    c.fill(
        Area {
            x: 0,
            y: 0,
            w: rect.right,
            h: rect.bottom,
        },
        bg(),
    );
    let top = (120.0 * s) as i32;
    let bottom = rect.bottom - (46.0 * s) as i32;
    let available = Area {
        x: (20.0 * s) as i32,
        y: top,
        w: rect.right - (40.0 * s) as i32,
        h: bottom - top,
    };
    let ratio = (available.w.max(1) as f64 / editor.original.width as f64)
        .min(available.h.max(1) as f64 / editor.original.height as f64)
        .min(2.0);
    let w = (editor.original.width as f64 * ratio).max(1.0) as i32;
    let h = (editor.original.height as f64 * ratio).max(1.0) as i32;
    editor.view = Area {
        x: available.x + (available.w - w) / 2,
        y: available.y + (available.h - h) / 2,
        w,
        h,
    };
    c.image(&editor.rendered, editor.view);
    let swatch = COLORS[editor.color];
    c.rounded(
        Area {
            x: (312.0 * s) as i32,
            y: (74.0 * s) as i32,
            w: (20.0 * s) as i32,
            h: (18.0 * s) as i32,
        },
        rgb(swatch[2], swatch[1], swatch[0]),
        border(),
        4,
    );
    let detail = format!(
        "{} px  |  {} x {}  |  {} marks{}",
        editor.radius * 2 + 1,
        editor.original.width,
        editor.original.height,
        editor.marks.len(),
        if editor.dirty {
            "  |  Unsaved changes"
        } else {
            ""
        }
    );
    c.text(
        &detail,
        Area {
            x: (344.0 * s) as i32,
            y: (74.0 * s) as i32,
            w: rect.right - (360.0 * s) as i32,
            h: (24.0 * s) as i32,
        },
        editor.fonts.small,
        muted(),
        DT_SINGLELINE | DT_END_ELLIPSIS,
    );
    c.text(
        &editor.status,
        Area {
            x: (20.0 * s) as i32,
            y: rect.bottom - (30.0 * s) as i32,
            w: rect.right - (40.0 * s) as i32,
            h: (24.0 * s) as i32,
        },
        editor.fonts.small,
        accent(),
        DT_SINGLELINE | DT_END_ELLIPSIS,
    );
}

unsafe extern "system" fn editor_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let ptr = app_ptr(hwnd, message, l);
    if ptr.is_null() {
        return DefWindowProcW(hwnd, message, w, l);
    }
    let app = &mut *ptr;
    match message {
        WM_CREATE => {
            editor_controls(hwnd, app);
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint_buffered(hwnd, |dc, rect| paint_editor(app, hwnd, dc, rect));
            0
        }
        WM_DRAWITEM => {
            draw_button(app, &*(l as *const DRAWITEMSTRUCT));
            1
        }
        WM_COMMAND => {
            app.editor_command((w & 0xffff) as u16);
            SetFocus(hwnd);
            0
        }
        WM_SIZE => {
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(l as *mut MINMAXINFO);
            let s = GetDpiForWindow(hwnd).max(96) as i32;
            info.ptMinTrackSize = POINT {
                x: 790 * s / 96,
                y: 420 * s / 96,
            };
            0
        }
        WM_DPICHANGED => {
            if let Some(editor) = &mut app.editor {
                editor.fonts = Fonts::new((w & 0xffff) as u32);
            }
            let r = &*(l as *const RECT);
            SetWindowPos(
                hwnd,
                null_mut(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            for id in ARROW..=WIDTH {
                let child = GetDlgItem(hwnd, id as i32);
                if !child.is_null() {
                    DestroyWindow(child);
                }
            }
            editor_controls(hwnd, app);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_LBUTTONDOWN => {
            let p = mouse_point(l);
            if let Some(editor) = &mut app.editor {
                if editor.view.contains(p) {
                    editor.draft = Some(Mark {
                        tool: editor.tool,
                        points: vec![editor.point(p)],
                        color: COLORS[editor.color],
                        radius: editor.radius,
                    });
                    SetFocus(hwnd);
                    SetCapture(hwnd);
                }
            }
            0
        }
        WM_MOUSEMOVE | WM_LBUTTONUP => {
            if let Some(editor) = &mut app.editor {
                let p = editor.point(mouse_point(l));
                if let Some(mark) = &mut editor.draft {
                    if mark.tool == Tool::Pen {
                        if mark.points.last() != Some(&p) {
                            mark.points.push(p);
                        }
                    } else {
                        mark.points.truncate(1);
                        mark.points.push(p);
                    }
                    if message == WM_LBUTTONUP {
                        if let Some(mark) = editor.draft.take() {
                            editor.marks.push(mark);
                            editor.undone.clear();
                            editor.dirty = true;
                        }
                    }
                    editor.redraw();
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            if message == WM_LBUTTONUP {
                ReleaseCapture();
            }
            0
        }
        WM_CAPTURECHANGED => {
            if let Some(editor) = &mut app.editor {
                if editor.draft.take().is_some() {
                    editor.redraw();
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            0
        }
        WM_KEYDOWN => {
            let ctrl = GetKeyState(VK_CONTROL as i32) < 0;
            let shift = GetKeyState(VK_SHIFT as i32) < 0;
            let command = match w as u16 {
                0x53 if ctrl => SAVE,
                0x43 if ctrl => EDITOR_COPY,
                0x5a if ctrl && shift => REDO,
                0x5a if ctrl => UNDO,
                0x59 if ctrl => REDO,
                0x31 => ARROW,
                0x32 => RECTANGLE,
                0x33 => PEN,
                0x34 => REDACT,
                VK_ESCAPE => {
                    SendMessageW(hwnd, WM_CLOSE, 0, 0);
                    return 0;
                }
                _ => 0,
            };
            if command != 0 {
                app.editor_command(command);
            }
            0
        }
        WM_CLOSE => {
            if app.editor.as_ref().is_some_and(|e| e.dirty) {
                let answer = MessageBoxW(
                    hwnd,
                    wide("Save your annotations as a new PNG before closing?").as_ptr(),
                    wide("FeatherShot").as_ptr(),
                    MB_YESNOCANCEL | MB_ICONQUESTION,
                );
                if answer == IDCANCEL {
                    return 0;
                }
                if answer == IDYES {
                    app.editor_command(SAVE);
                    if app.editor.as_ref().is_some_and(|e| e.dirty) {
                        return 0;
                    }
                }
            }
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            app.editor_hwnd = null_mut();
            app.editor = None;
            EnableWindow(app.main, 1);
            app.show();
            0
        }
        _ => DefWindowProcW(hwnd, message, w, l),
    }
}

unsafe fn update_win_labels(app: &App) {
    for i in 0..3 {
        let input = GetDlgItem(app.hotkeys_hwnd, 401 + i as i32);
        SetWindowSubclass(
            input,
            Some(hotkey_input_proc),
            1,
            if app.hotkeys_win[i] {
                MOD_WIN as usize
            } else {
                0
            },
        );
        InvalidateRect(input, null(), 0);
        SetWindowTextW(
            GetDlgItem(app.hotkeys_hwnd, 411 + i as i32),
            wide(if app.hotkeys_win[i] {
                "[x] Win"
            } else {
                "[ ] Win"
            })
            .as_ptr(),
        );
    }
}

unsafe extern "system" fn hotkey_input_proc(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    if message == WM_PAINT {
        let mut ps: PAINTSTRUCT = zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        paint_shortcut_field(hwnd, dc, data as u32);
        EndPaint(hwnd, &ps);
        return 0;
    }
    if message == WM_PRINTCLIENT {
        paint_shortcut_field(hwnd, w as HDC, data as u32);
        return 0;
    }
    if message == WM_ERASEBKGND {
        return 1;
    }
    if matches!(message, WM_KEYUP | WM_SYSKEYUP) && w == VK_SNAPSHOT as usize {
        let mut modifiers = 0;
        for (key, flag) in [
            (VK_CONTROL, MOD_CONTROL),
            (VK_MENU, MOD_ALT),
            (VK_SHIFT, MOD_SHIFT),
        ] {
            if GetKeyState(key as i32) < 0 {
                modifiers |= flag;
            }
        }
        SendMessageW(
            hwnd,
            HKM_SETHOTKEY,
            Hotkey {
                modifiers,
                key: VK_SNAPSHOT as u32,
            }
            .control_value(),
            0,
        );
        return 0;
    }
    if message == WM_NCDESTROY {
        RemoveWindowSubclass(hwnd, Some(hotkey_input_proc), id);
    }
    let result = DefSubclassProc(hwnd, message, w, l);
    if message == WM_SETFOCUS {
        HideCaret(hwnd);
    }
    if matches!(
        message,
        HKM_SETHOTKEY
            | WM_KEYDOWN
            | WM_KEYUP
            | WM_SYSKEYDOWN
            | WM_SYSKEYUP
            | WM_SETFOCUS
            | WM_KILLFOCUS
    ) {
        InvalidateRect(hwnd, null(), 0);
    }
    result
}

unsafe fn paint_shortcut_field(hwnd: HWND, dc: HDC, extra_modifiers: u32) {
    let mut key = Hotkey::from_control(SendMessageW(hwnd, HKM_GETHOTKEY, 0, 0) as usize);
    key.modifiers |= extra_modifiers & MOD_WIN;
    let mut rect: RECT = zeroed();
    GetClientRect(hwnd, &mut rect);
    FillRect(dc, &rect, GetSysColorBrush(COLOR_WINDOW));
    let font = SendMessageW(hwnd, WM_GETFONT, 0, 0) as HFONT;
    let previous = SelectObject(
        dc,
        if font.is_null() {
            GetStockObject(DEFAULT_GUI_FONT)
        } else {
            font
        },
    );
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, GetSysColor(COLOR_WINDOWTEXT));
    rect.left += 6;
    rect.right -= 6;
    let text = wide(&key.display());
    DrawTextW(
        dc,
        text.as_ptr(),
        (text.len() - 1) as i32,
        &mut rect,
        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
    );
    SelectObject(dc, previous);
}

unsafe fn hotkeys_controls(hwnd: HWND, app: &App) {
    let dpi = GetDpiForWindow(hwnd).max(96);
    let s = dpi as f64 / 96.0;
    let fonts = app.hotkeys_fonts.as_ref().unwrap_or(&app.fonts);
    for (index, key) in app.settings.hotkeys.iter().enumerate() {
        let input = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            wide("msctls_hotkey32").as_ptr(),
            null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            (234.0 * s) as i32,
            ((116 + index * 58) as f64 * s) as i32,
            (252.0 * s) as i32,
            (34.0 * s) as i32,
            hwnd,
            (401 + index) as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        SendMessageW(input, WM_SETFONT, fonts.normal as usize, 1);
        SetWindowSubclass(
            input,
            Some(hotkey_input_proc),
            1,
            if app.hotkeys_win[index] {
                MOD_WIN as usize
            } else {
                0
            },
        );
        SendMessageW(input, HKM_SETHOTKEY, key.control_value(), 0);
        button(
            hwnd,
            411 + index as u16,
            if app.hotkeys_win[index] {
                "[x] Win"
            } else {
                "[ ] Win"
            },
            Area {
                x: 498,
                y: 116 + index as i32 * 58,
                w: 80,
                h: 34,
            },
            s,
        );
    }
    button(
        hwnd,
        WIN_SNIP,
        "Use Win+Shift+S",
        Area {
            x: 234,
            y: 270,
            w: 208,
            h: 24,
        },
        s,
    );
    button(
        hwnd,
        PRINT_SNIP,
        "PrtSc: All screens",
        Area {
            x: 454,
            y: 270,
            w: 124,
            h: 24,
        },
        s,
    );
    button(
        hwnd,
        BLOCK_SNIPPING,
        if app.hotkeys_block_snipping {
            "[x] Block Snipping Tool shortcuts while running"
        } else {
            "[ ] Block Snipping Tool shortcuts while running"
        },
        Area {
            x: 26,
            y: 302,
            w: 552,
            h: 32,
        },
        s,
    );
    for (id, label, x, w) in [
        (APPLY_KEYS, "Save shortcuts", 26, 172),
        (RESET_KEYS, "Restore defaults", 212, 178),
        (CANCEL_KEYS, "Cancel", 404, 174),
    ] {
        button(
            hwnd,
            id,
            label,
            Area {
                x,
                y: 348,
                w,
                h: 40,
            },
            s,
        );
    }
}

unsafe extern "system" fn hotkeys_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let ptr = app_ptr(hwnd, message, l);
    if ptr.is_null() {
        return DefWindowProcW(hwnd, message, w, l);
    }
    let app = &mut *ptr;
    match message {
        WM_CREATE => {
            hotkeys_controls(hwnd, app);
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint_buffered(hwnd, |dc, _| {
                let c = Canvas {
                    dc,
                    scale: GetDpiForWindow(hwnd).max(96) as f64 / 96.0,
                };
                let fonts = app.hotkeys_fonts.as_ref().unwrap_or(&app.fonts);
                c.fill(
                    Area {
                        x: 0,
                        y: 0,
                        w: 610,
                        h: 494,
                    },
                    bg(),
                );
                c.text(
                    "Keyboard shortcuts",
                    Area {
                        x: 26,
                        y: 22,
                        w: 552,
                        h: 32,
                    },
                    fonts.heading,
                    white(),
                    DT_SINGLELINE,
                );
                c.text("Record Ctrl / Alt / Shift + a key, then tick Win to add it. Backspace clears a shortcut. Save to apply.", Area { x: 26, y: 66, w: 552, h: 42 }, fonts.normal, muted(), DT_WORDBREAK);
                for (i, label) in ["Select region", "Pick a window", "All screens"]
                    .iter()
                    .enumerate()
                {
                    c.text(
                        label,
                        Area {
                            x: 26,
                            y: 116 + i as i32 * 58,
                            w: 192,
                            h: 34,
                        },
                        fonts.normal,
                        white(),
                        DT_SINGLELINE | DT_VCENTER,
                    );
                }
                c.text(
                    &app.hotkeys_status,
                    Area {
                        x: 26,
                        y: 410,
                        w: 552,
                        h: 66,
                    },
                    fonts.small,
                    accent(),
                    DT_WORDBREAK,
                );
            });
            0
        }
        WM_DRAWITEM => {
            draw_button(app, &*(l as *const DRAWITEMSTRUCT));
            1
        }
        WM_COMMAND => {
            app.hotkeys_command((w & 0xffff) as u16);
            0
        }
        WM_KEYDOWN if w == VK_ESCAPE as usize => {
            DestroyWindow(hwnd);
            0
        }
        WM_DPICHANGED => {
            app.hotkeys_fonts = Some(Fonts::new((w & 0xffff) as u32));
            // Keep unsaved key values while rebuilding controls at the new DPI.
            let values: [usize; 3] = std::array::from_fn(|i| {
                SendMessageW(GetDlgItem(hwnd, 401 + i as i32), HKM_GETHOTKEY, 0, 0) as usize
            });
            for id in 401..=413 {
                let child = GetDlgItem(hwnd, id);
                if !child.is_null() {
                    DestroyWindow(child);
                }
            }
            let r = &*(l as *const RECT);
            SetWindowPos(
                hwnd,
                null_mut(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            hotkeys_controls(hwnd, app);
            for (i, value) in values.into_iter().enumerate() {
                SendMessageW(GetDlgItem(hwnd, 401 + i as i32), HKM_SETHOTKEY, value, 0);
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            app.hotkeys_hwnd = null_mut();
            app.hotkeys_fonts = None;
            EnableWindow(app.main, 1);
            app.show();
            0
        }
        _ => DefWindowProcW(hwnd, message, w, l),
    }
}

unsafe fn create_icon() -> HICON {
    let mut pixels = vec![0u8; 32 * 32 * 4];
    for y in 2..30i32 {
        for x in 2..30i32 {
            if (4..28).contains(&x) && (8..26).contains(&y)
                || (10..20).contains(&x) && (5..8).contains(&y)
            {
                let i = (y as usize * 32 + x as usize) * 4;
                let color = if (x - 16).pow(2) + (y - 17).pow(2) <= 28 {
                    [25, 19, 15, 255]
                } else {
                    [185, 232, 122, 255]
                };
                pixels[i..i + 4].copy_from_slice(&color);
            }
        }
    }
    let info = bitmap_info(32, 32);
    let mut bits = null_mut();
    let bitmap = CreateDIBSection(null_mut(), &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if bitmap.is_null() || bits.is_null() {
        return CopyIcon(LoadIconW(null_mut(), IDI_APPLICATION));
    }
    std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());
    let mask = CreateBitmap(32, 32, 1, 1, [0u8; 128].as_ptr().cast());
    let icon = CreateIconIndirect(&ICONINFO {
        fIcon: 1,
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask,
        hbmColor: bitmap,
    });
    DeleteObject(mask);
    DeleteObject(bitmap);
    if icon.is_null() {
        CopyIcon(LoadIconW(null_mut(), IDI_APPLICATION))
    } else {
        icon
    }
}

unsafe fn register_class(name: &str, proc: WNDPROC, icon: HICON) -> Result<()> {
    let name_w = wide(name);
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: proc,
        hInstance: GetModuleHandleW(null()),
        hIcon: icon,
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        lpszClassName: name_w.as_ptr(),
        ..zeroed()
    };
    if RegisterClassW(&class) == 0 {
        Err(os_error("Register window class"))
    } else {
        Ok(())
    }
}

fn main() {
    if let Err(error) = unsafe { run() } {
        unsafe {
            MessageBoxW(
                null_mut(),
                wide(&error).as_ptr(),
                wide("FeatherShot").as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

unsafe fn run() -> Result<()> {
    SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    let args: Vec<_> = std::env::args_os().collect();
    let data = args
        .iter()
        .position(|a| a == "--data-dir")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or(
            std::env::current_exe()
                .map_err(|e| e.to_string())?
                .parent()
                .ok_or("No application directory")?
                .to_path_buf(),
        );
    std::fs::create_dir_all(&data)
        .map_err(|e| format!("Application directory is not writable: {e}"))?;
    let settings = Settings::load(&data.join("settings.ini"));
    let start_in_tray = !args.iter().any(|a| a == "--show")
        && (args.iter().any(|a| a == "--tray") || settings.start_in_tray);
    let lock_name = wide("Local\\FeatherShot.Native.0.1");
    let mutex = CreateMutexW(null(), 0, lock_name.as_ptr());
    if mutex.is_null() {
        return Err(os_error("Create application lock"));
    }
    struct Lock(HANDLE);
    impl Drop for Lock {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let _lock = Lock(mutex);
    if GetLastError() == ERROR_ALREADY_EXISTS {
        let existing = FindWindowW(wide(MAIN_CLASS).as_ptr(), null());
        if !existing.is_null() && !start_in_tray {
            PostMessageW(existing, SHOW_MESSAGE, 0, 0);
        }
        return Ok(());
    }
    let icon = create_icon();
    register_class(MAIN_CLASS, Some(main_proc), icon)?;
    register_class(OVERLAY_CLASS, Some(overlay_proc), icon)?;
    register_class(EDITOR_CLASS, Some(editor_proc), icon)?;
    register_class(HOTKEY_CLASS, Some(hotkeys_proc), icon)?;
    InitCommonControlsEx(&INITCOMMONCONTROLSEX {
        dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_HOTKEY_CLASS,
    });
    let dpi = GetDpiForSystem().max(96);
    let screenshots = data.join("Screenshots");
    let mut app = Box::new(App {
        main: null_mut(),
        overlay_hwnd: null_mut(),
        editor_hwnd: null_mut(),
        hotkeys_hwnd: null_mut(),
        hotkeys_fonts: None,
        hotkeys_status: String::new(),
        hotkeys_win: [false; 3],
        hotkeys_block_snipping: false,
        registered: Hotkeys::new(),
        fonts: Fonts::new(dpi),
        dpi,
        icon,
        data,
        screenshots,
        settings,
        overlay: None,
        editor: None,
        last: None,
        preview: None,
        dimensions: (0, 0),
        status: "Ready. Select a region or press Ctrl+Shift+S.".into(),
        busy: false,
        pending: 0,
        return_main: true,
        count: 0,
        hotkey_warning: String::new(),
        tray_added: false,
        taskbar_message: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
    });
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: (820 * dpi / 96) as i32,
        bottom: (674 * dpi / 96) as i32,
    };
    AdjustWindowRectExForDpi(&mut rect, style, 0, WS_EX_APPWINDOW, dpi);
    let hwnd = CreateWindowExW(
        WS_EX_APPWINDOW,
        wide(MAIN_CLASS).as_ptr(),
        wide("FeatherShot").as_ptr(),
        style,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        rect.right - rect.left,
        rect.bottom - rect.top,
        null_mut(),
        null_mut(),
        GetModuleHandleW(null()),
        app.as_mut() as *mut App as _,
    );
    if hwnd.is_null() {
        return Err(os_error("Create main window"));
    }
    let dark: i32 = 1;
    DwmSetWindowAttribute(hwnd, 20, (&dark as *const i32).cast(), 4);
    app.add_tray();
    let hotkeys = app.settings.hotkeys;
    if !app.registered.install(hwnd, &hotkeys).is_empty() {
        app.hotkey_warning =
            "Some shortcuts are busy. Click Customize hotkeys to change them.".into();
    }
    if let Err(error) = app
        .registered
        .set_block_snipping(hwnd, app.settings.block_snipping)
    {
        app.hotkey_warning = format!("Snipping Tool shortcut blocking could not start: {error}");
    }
    if !start_in_tray || !app.tray_added {
        app.show();
    }
    let mut message: MSG = zeroed();
    loop {
        let result = GetMessageW(&mut message, null_mut(), 0, 0);
        if result <= 0 {
            break;
        }
        // Preserve editor shortcuts even while a toolbar button owns keyboard focus.
        if !app.editor_hwnd.is_null()
            && message.message == WM_KEYDOWN
            && (message.hwnd == app.editor_hwnd || IsChild(app.editor_hwnd, message.hwnd) != 0)
            && (GetKeyState(VK_CONTROL as i32) < 0
                || matches!(message.wParam as u16, VK_ESCAPE | 0x31..=0x34))
        {
            SendMessageW(app.editor_hwnd, WM_KEYDOWN, message.wParam, message.lParam);
            continue;
        }
        let dialog = if !app.hotkeys_hwnd.is_null() {
            app.hotkeys_hwnd
        } else if app.editor_hwnd.is_null() {
            app.main
        } else {
            app.editor_hwnd
        };
        if IsDialogMessageW(dialog, &message) == 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    DestroyIcon(icon);
    Ok(())
}

#[cfg(test)]
mod settings_tests {
    use super::*;
    #[test]
    fn shortcut_fields_render_complete_labels() {
        unsafe {
            InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_HOTKEY_CLASS,
            });
            let parent = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                null_mut(),
                null_mut(),
                null(),
            );
            assert!(!parent.is_null());
            let fonts = Fonts::new(144);
            let dc = CreateCompatibleDC(null_mut());
            let info = bitmap_info(260, 88);
            let mut bits = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            assert!(!dc.is_null() && !bitmap.is_null() && !bits.is_null());
            let old = SelectObject(dc, bitmap);
            Canvas { dc, scale: 1.0 }.fill(
                Area {
                    x: 0,
                    y: 0,
                    w: 260,
                    h: 88,
                },
                bg(),
            );
            for (index, (key, extra)) in [
                (
                    Hotkey {
                        modifiers: MOD_SHIFT,
                        key: 0x53,
                    },
                    MOD_WIN,
                ),
                (
                    Hotkey {
                        modifiers: 0,
                        key: VK_SNAPSHOT as u32,
                    },
                    0,
                ),
            ]
            .into_iter()
            .enumerate()
            {
                let input = CreateWindowExW(
                    0,
                    wide("msctls_hotkey32").as_ptr(),
                    null(),
                    WS_CHILD,
                    0,
                    0,
                    252,
                    34,
                    parent,
                    null_mut(),
                    null_mut(),
                    null(),
                );
                assert!(!input.is_null());
                SendMessageW(input, WM_SETFONT, fonts.normal as usize, 0);
                SetWindowSubclass(input, Some(hotkey_input_proc), 1, extra as usize);
                SendMessageW(input, HKM_SETHOTKEY, key.control_value(), 0);
                SetViewportOrgEx(dc, 4, 4 + index as i32 * 44, null_mut());
                SendMessageW(input, WM_PRINTCLIENT, dc as usize, PRF_CLIENT as isize);
            }
            GdiFlush();
            let mut image = Image::new(260, 88).unwrap();
            std::ptr::copy_nonoverlapping(
                bits.cast::<u8>(),
                image.pixels.as_mut_ptr(),
                image.pixels.len(),
            );
            for pixel in image.pixels.chunks_exact_mut(4) {
                pixel[3] = 255;
            }
            SelectObject(dc, old);
            DeleteObject(bitmap);
            DeleteDC(dc);
            DestroyWindow(parent);
            for row in 0..2 {
                let dark_pixels = (4 + row * 44..38 + row * 44)
                    .flat_map(|y| (4..256).map(move |x| (y * 260 + x) * 4))
                    .filter(|i| image.pixels[*i] < 150)
                    .count();
                assert!(
                    dark_pixels > 40,
                    "Shortcut label must be rendered in each field"
                );
            }
            let path = std::path::Path::new("target/shortcut-fields.png");
            if path.exists() {
                std::fs::remove_file(path).unwrap();
            }
            image.save_png(path).unwrap();
        }
    }

    #[test]
    fn settings_roundtrip_and_replace_keep_user_hotkeys() {
        let path =
            std::env::temp_dir().join(format!("feathershot-settings-{}.ini", std::process::id()));
        let mut settings = Settings {
            block_snipping: true,
            delay: 3,
            no_copy: true,
            cursor: true,
            start_in_tray: true,
            hotkeys: DEFAULTS,
        };
        settings.hotkeys[0] = Hotkey {
            modifiers: MOD_WIN | MOD_SHIFT,
            key: 0x53,
        };
        settings.save(&path).unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.hotkeys, settings.hotkeys);
        assert!(loaded.start_in_tray);
        assert!(loaded.block_snipping);
        assert_eq!(
            (loaded.delay, loaded.no_copy, loaded.cursor),
            (3, true, true)
        );
        settings.hotkeys.swap(0, 1);
        settings.start_in_tray = false;
        settings.block_snipping = false;
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path).hotkeys, settings.hotkeys);
        assert!(!Settings::load(&path).start_in_tray);
        assert!(!Settings::load(&path).block_snipping);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(Settings::load(&path).hotkeys, DEFAULTS);
        assert!(!Settings::load(&path).start_in_tray);
    }
}
