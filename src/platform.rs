use crate::image::{Area, Image, Result};
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::path::{Path, PathBuf};
use std::ptr::{copy_nonoverlapping, null, null_mut};
use std::time::Duration;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::{Dwm::*, Gdi::*};
use windows_sys::Win32::System::{DataExchange::*, Memory::*, SystemInformation::*};
use windows_sys::Win32::UI::{Shell::*, WindowsAndMessaging::*};

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub fn os_error(action: &str) -> String {
    format!("{action}: {}", std::io::Error::last_os_error())
}

pub fn virtual_screen() -> Area {
    unsafe {
        Area {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            w: GetSystemMetrics(SM_CXVIRTUALSCREEN),
            h: GetSystemMetrics(SM_CYVIRTUALSCREEN),
        }
    }
}

pub fn bitmap_info(width: u32, height: u32) -> BITMAPINFO {
    let mut info: BITMAPINFO = unsafe { zeroed() };
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width as i32,
        biHeight: -(height as i32),
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..unsafe { zeroed() }
    };
    info
}

// Every DC/bitmap is released even when a Win32 operation fails.
struct ScreenDc(HDC);
impl Drop for ScreenDc {
    fn drop(&mut self) {
        unsafe {
            ReleaseDC(null_mut(), self.0);
        }
    }
}
struct MemoryDc(HDC);
impl Drop for MemoryDc {
    fn drop(&mut self) {
        unsafe {
            DeleteDC(self.0);
        }
    }
}
struct Bitmap(HBITMAP);
impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.0);
        }
    }
}
struct Selection {
    dc: HDC,
    old: HGDIOBJ,
}
impl Drop for Selection {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
        }
    }
}

pub fn capture(area: Area, include_cursor: bool) -> Result<Image> {
    let mut out = Image::new(area.w as u32, area.h as u32)?;
    unsafe {
        let screen = ScreenDc(GetDC(null_mut()));
        if screen.0.is_null() {
            return Err(os_error("Get screen DC"));
        }
        let memory = MemoryDc(CreateCompatibleDC(screen.0));
        if memory.0.is_null() {
            return Err(os_error("Create capture DC"));
        }
        let info = bitmap_info(out.width, out.height);
        let mut bits: *mut c_void = null_mut();
        let bitmap = Bitmap(CreateDIBSection(
            screen.0,
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            null_mut(),
            0,
        ));
        if bitmap.0.is_null() || bits.is_null() {
            return Err(os_error("Allocate capture bitmap"));
        }
        let old = SelectObject(memory.0, bitmap.0);
        if old.is_null() || old as isize == -1 {
            return Err(os_error("Select capture bitmap"));
        }
        let _selection = Selection { dc: memory.0, old };
        if BitBlt(
            memory.0,
            0,
            0,
            area.w,
            area.h,
            screen.0,
            area.x,
            area.y,
            SRCCOPY | CAPTUREBLT,
        ) == 0
        {
            return Err(os_error("Capture screen"));
        }
        if include_cursor {
            let mut cursor: CURSORINFO = zeroed();
            cursor.cbSize = size_of::<CURSORINFO>() as u32;
            if GetCursorInfo(&mut cursor) != 0 && cursor.flags == CURSOR_SHOWING {
                let mut icon: ICONINFO = zeroed();
                if GetIconInfo(cursor.hCursor, &mut icon) != 0 {
                    DrawIconEx(
                        memory.0,
                        cursor.ptScreenPos.x - area.x - icon.xHotspot as i32,
                        cursor.ptScreenPos.y - area.y - icon.yHotspot as i32,
                        cursor.hCursor,
                        0,
                        0,
                        0,
                        null_mut(),
                        DI_NORMAL,
                    );
                    if !icon.hbmMask.is_null() {
                        DeleteObject(icon.hbmMask);
                    }
                    if !icon.hbmColor.is_null() {
                        DeleteObject(icon.hbmColor);
                    }
                }
            }
        }
        GdiFlush();
        copy_nonoverlapping(bits.cast::<u8>(), out.pixels.as_mut_ptr(), out.pixels.len());
    }
    for pixel in out.pixels.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    Ok(out)
}

/// CF_DIB memory belongs to Windows after SetClipboardData succeeds.
pub fn copy_image(owner: HWND, image: &Image) -> Result<()> {
    const CF_DIB: u32 = 8;
    unsafe {
        let header_size = size_of::<BITMAPINFOHEADER>();
        let memory = GlobalAlloc(GMEM_MOVEABLE, header_size + image.pixels.len());
        if memory.is_null() {
            return Err(os_error("Allocate clipboard image"));
        }
        let data = GlobalLock(memory).cast::<u8>();
        if data.is_null() {
            GlobalFree(memory);
            return Err(os_error("Lock clipboard image"));
        }
        let info = bitmap_info(image.width, image.height);
        copy_nonoverlapping(
            (&info.bmiHeader as *const BITMAPINFOHEADER).cast::<u8>(),
            data,
            header_size,
        );
        copy_nonoverlapping(
            image.pixels.as_ptr(),
            data.add(header_size),
            image.pixels.len(),
        );
        GlobalUnlock(memory);
        let mut opened = false;
        for _ in 0..8 {
            if OpenClipboard(owner) != 0 {
                opened = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        if !opened {
            GlobalFree(memory);
            return Err("Clipboard is busy. The PNG was saved; try Copy again.".into());
        }
        let result = if EmptyClipboard() == 0 || SetClipboardData(CF_DIB, memory).is_null() {
            GlobalFree(memory);
            Err(os_error("Copy image to clipboard"))
        } else {
            Ok(())
        };
        CloseClipboard();
        result
    }
}

pub fn open_path(path: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let path_w: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            wide("open").as_ptr(),
            path_w.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        Err(format!(
            "Windows could not open {} (code {})",
            path.display(),
            result as isize
        ))
    } else {
        Ok(())
    }
}

pub fn new_capture_path(directory: &Path, edited: bool) -> PathBuf {
    let mut time: SYSTEMTIME = unsafe { zeroed() };
    unsafe {
        GetLocalTime(&mut time);
    }
    let suffix = if edited { "-edited" } else { "" };
    let stem = format!(
        "FeatherShot_{:04}-{:02}-{:02}_{:02}-{:02}-{:02}-{:03}{suffix}",
        time.wYear,
        time.wMonth,
        time.wDay,
        time.wHour,
        time.wMinute,
        time.wSecond,
        time.wMilliseconds
    );
    let mut path = directory.join(format!("{stem}.png"));
    for i in 1..10_000 {
        if !path.exists() {
            break;
        }
        path = directory.join(format!("{stem}-{i}.png"));
    }
    path
}

pub fn window_areas(exclude: HWND, screen: Area) -> Vec<Area> {
    struct Enumeration {
        exclude: HWND,
        screen: Area,
        areas: Vec<Area>,
    }
    unsafe extern "system" fn visit(hwnd: HWND, param: LPARAM) -> i32 {
        let state = &mut *(param as *mut Enumeration);
        if hwnd == state.exclude || IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
            return 1;
        }
        let mut cloaked: u32 = 0;
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            (&mut cloaked as *mut u32).cast(),
            4,
        );
        if cloaked != 0 {
            return 1;
        }
        let mut rect: RECT = zeroed();
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            (&mut rect as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        ) < 0
        {
            GetWindowRect(hwnd, &mut rect);
        }
        let area = Area {
            x: rect.left,
            y: rect.top,
            w: rect.right - rect.left,
            h: rect.bottom - rect.top,
        }
        .intersect(state.screen);
        if area.w >= 16 && area.h >= 16 {
            state.areas.push(Area {
                x: area.x - state.screen.x,
                y: area.y - state.screen.y,
                ..area
            });
        }
        1
    }
    let mut state = Enumeration {
        exclude,
        screen,
        areas: Vec::new(),
    };
    unsafe {
        EnumWindows(Some(visit), &mut state as *mut Enumeration as LPARAM);
    }
    state.areas
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_capture_releases_gdi_objects_and_normalizes_alpha() {
        use windows_sys::Win32::System::Threading::{
            GetCurrentProcess, GetGuiResources, GR_GDIOBJECTS,
        };
        let screen = virtual_screen();
        let area = Area {
            x: screen.x,
            y: screen.y,
            w: 32,
            h: 32,
        };
        capture(area, false).unwrap();
        let before = unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) };
        for _ in 0..24 {
            let image = capture(area, false).unwrap();
            assert_eq!((image.width, image.height), (32, 32));
            assert!(image.pixels.chunks_exact(4).all(|p| p[3] == 255));
        }
        let after = unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) };
        assert!(after <= before, "GDI objects leaked: {before} -> {after}");
    }
}
