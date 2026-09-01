use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskbarGeom {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub edge: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BarPlacement {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

static FIRST_DOCK: AtomicBool = AtomicBool::new(true);
static DRAGGING: AtomicBool = AtomicBool::new(false);
static LAST_PLACE: Mutex<Option<BarPlacement>> = Mutex::new(None);

pub fn set_dragging(on: bool) {
    DRAGGING.store(on, Ordering::SeqCst);
}

pub fn is_dragging() -> bool {
    DRAGGING.load(Ordering::SeqCst)
}

#[cfg(windows)]
pub fn taskbar_geometry() -> Option<TaskbarGeom> {
    use windows::Win32::UI::Shell::{ABM_GETTASKBARPOS, APPBARDATA, SHAppBarMessage};

    unsafe {
        let mut data = APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            ..Default::default()
        };
        if SHAppBarMessage(ABM_GETTASKBARPOS, &mut data) == 0 {
            return None;
        }
        let r = data.rc;
        Some(TaskbarGeom {
            x: r.left,
            y: r.top,
            width: (r.right - r.left).max(1),
            height: (r.bottom - r.top).max(1),
            edge: data.uEdge,
        })
    }
}

#[cfg(not(windows))]
pub fn taskbar_geometry() -> Option<TaskbarGeom> {
    None
}

fn bar_rect(bar_width: u32, self_hwnd: isize, offset: Option<i32>) -> Option<BarPlacement> {
    let tb = taskbar_geometry()?;
    let width = bar_width.min(tb.width.saturating_sub(16) as u32).max(300) as i32;
    let height = tb.height.max(40);
    let max_off = (tb.width - width).max(0);

    let (x, y) = if let Some(off) = offset {
        let off = off.clamp(0, max_off);
        match tb.edge {
            0 => (tb.x, tb.y + off),
            2 => (tb.x, tb.y + off),
            _ => (tb.x + off, tb.y),
        }
    } else {
        let gap = 8;
        let right_edge = right_cluster_left(&tb, self_hwnd);
        match tb.edge {
            1 => ((right_edge - gap - width).max(tb.x), tb.y),
            0 => (tb.x, tb.y + 8),
            2 => (tb.x, tb.y + 8),
            _ => ((right_edge - gap - width).max(tb.x), tb.y),
        }
    };

    Some(BarPlacement {
        x,
        y,
        w: width,
        h: height,
    })
}

pub fn nudge_offset(current: Option<i32>, dx: i32, bar_width: u32) -> Option<i32> {
    let tb = taskbar_geometry()?;
    let width = bar_width.min(tb.width.saturating_sub(16) as u32).max(300) as i32;
    let max_off = (tb.width - width).max(0);
    let base = current.unwrap_or_else(|| {
        LAST_PLACE
            .lock()
            .ok()
            .and_then(|p| *p)
            .map(|pl| match tb.edge {
                0 | 2 => pl.y - tb.y,
                _ => pl.x - tb.x,
            })
            .unwrap_or(0)
    });
    Some((base + dx).clamp(0, max_off))
}

/// Left edge of the right-hand taskbar cluster (clock, tray, TrafficMonitor, …).
fn right_cluster_left(tb: &TaskbarGeom, self_hwnd: isize) -> i32 {
    let fallback = tb.x + tb.width - 180;
    let mut best = system_tray_left().unwrap_or(fallback);
    occupy_scan(tb, self_hwnd, &mut best);
    best
}

#[cfg(windows)]
fn system_tray_left() -> Option<i32> {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, GetWindowRect};

    unsafe {
        let tray = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()).ok()?;
        let notify = FindWindowExW(
            tray,
            windows::Win32::Foundation::HWND::default(),
            w!("TrayNotifyWnd"),
            PCWSTR::null(),
        )
        .ok()?;
        let mut rc = RECT::default();
        GetWindowRect(notify, &mut rc).ok()?;
        if rc.right - rc.left < 8 {
            return None;
        }
        Some(rc.left)
    }
}

#[cfg(not(windows))]
fn system_tray_left() -> Option<i32> {
    None
}

#[cfg(windows)]
fn occupy_scan(tb: &TaskbarGeom, self_hwnd: isize, best: &mut i32) {
    use windows::Win32::Foundation::LPARAM;
    use windows::Win32::UI::WindowsAndMessaging::{EnumChildWindows, EnumWindows, FindWindowW};
    use windows::core::{w, PCWSTR};

    let mut scan = Occupancy {
        tb: *tb,
        self_hwnd,
        best: *best,
    };
    let param = LPARAM(&mut scan as *mut Occupancy as isize);
    unsafe {
        let _ = EnumWindows(Some(occupancy_cb), param);
        if let Ok(tray) = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) {
            let _ = EnumChildWindows(tray, Some(occupancy_cb), param);
        }
    }
    *best = scan.best;
}

#[cfg(not(windows))]
fn occupy_scan(_tb: &TaskbarGeom, _self_hwnd: isize, _best: &mut i32) {}

#[cfg(windows)]
struct Occupancy {
    tb: TaskbarGeom,
    self_hwnd: isize,
    best: i32,
}

#[cfg(windows)]
unsafe extern "system" fn occupancy_cb(
    hwnd: windows::Win32::Foundation::HWND,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::BOOL {
    use windows::Win32::Foundation::{BOOL, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsWindowVisible};

    let scan = unsafe { &mut *(lparam.0 as *mut Occupancy) };
    if hwnd.0 as isize == scan.self_hwnd {
        return BOOL(1);
    }
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }
        let mut rc = RECT::default();
        if GetWindowRect(hwnd, &mut rc).is_err() {
            return BOOL(1);
        }
        let w = rc.right - rc.left;
        let h = rc.bottom - rc.top;
        if h > 80 || w < 16 || w >= scan.tb.width - 20 {
            return BOOL(1);
        }
        let overlaps_y = rc.top < scan.tb.y + scan.tb.height && rc.bottom > scan.tb.y;
        if !overlaps_y {
            return BOOL(1);
        }
        let mid = scan.tb.x + scan.tb.width / 2;
        if rc.left <= mid {
            return BOOL(1);
        }
        if rc.left < scan.best {
            scan.best = rc.left;
        }
    }
    BOOL(1)
}

pub fn dock_bar(
    window: &WebviewWindow,
    bar_width: u32,
    offset: Option<i32>,
) -> Result<(), String> {
    let self_hwnd = window
        .hwnd()
        .map(|h| h.0 as isize)
        .unwrap_or(0);
    let Some(place) = bar_rect(bar_width, self_hwnd, offset) else {
        return Ok(());
    };

    let first = FIRST_DOCK.swap(false, Ordering::SeqCst);
    {
        let mut last = LAST_PLACE.lock().unwrap();
        if !first && *last == Some(place) {
            keep_above(window);
            if !window.is_visible().unwrap_or(true) {
                let _ = window.show();
            }
            return Ok(());
        }
        *last = Some(place);
    }

    if first {
        let _ = window.set_skip_taskbar(true);
        let _ = window.set_shadow(false);
        let _ = window.set_always_on_top(true);
        restore_toplevel(window);
        let _ = window.set_size(PhysicalSize::new(place.w as u32, place.h as u32));
        let _ = window.set_position(PhysicalPosition::new(place.x, place.y));
        pin_at(window, place, true);
        if window.is_minimized().unwrap_or(false) {
            let _ = window.unminimize();
        }
        let _ = window.show();
    } else {
        pin_at(window, place, false);
    }
    Ok(())
}

#[cfg(windows)]
fn restore_toplevel(window: &WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, SetParent, SetWindowLongW, GWL_EXSTYLE, GWL_STYLE, WS_CHILD,
        WS_EX_TOOLWINDOW, WS_POPUP, WS_VISIBLE,
    };

    let Ok(raw) = window.hwnd() else {
        return;
    };
    unsafe {
        let hwnd = HWND(raw.0 as *mut _);
        let _ = SetParent(hwnd, HWND::default());
        let mut style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        style &= !WS_CHILD.0;
        style |= WS_POPUP.0 | WS_VISIBLE.0;
        SetWindowLongW(hwnd, GWL_STYLE, style as i32);
        let mut ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        ex |= WS_EX_TOOLWINDOW.0;
        SetWindowLongW(hwnd, GWL_EXSTYLE, ex as i32);
    }
}

#[cfg(not(windows))]
fn restore_toplevel(_window: &WebviewWindow) {}

#[cfg(windows)]
fn pin_at(window: &WebviewWindow, place: BarPlacement, topmost: bool) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOP, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOZORDER,
    };

    let Ok(raw) = window.hwnd() else {
        return;
    };
    unsafe {
        let hwnd = HWND(raw.0 as *mut _);
        let flags = if topmost {
            SWP_NOACTIVATE
        } else {
            SWP_NOACTIVATE | SWP_NOZORDER
        };
        let insert = if topmost { HWND_TOPMOST } else { HWND_TOP };
        let _ = SetWindowPos(hwnd, insert, place.x, place.y, place.w, place.h, flags);
    }
}

#[cfg(not(windows))]
fn pin_at(_window: &WebviewWindow, _place: BarPlacement, _topmost: bool) {}

#[cfg(windows)]
fn keep_above(window: &WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };

    let Ok(raw) = window.hwnd() else {
        return;
    };
    unsafe {
        let hwnd = HWND(raw.0 as *mut _);
        let _ = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

#[cfg(not(windows))]
fn keep_above(_window: &WebviewWindow) {}
