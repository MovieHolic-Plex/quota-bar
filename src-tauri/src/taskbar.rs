//! Docking the bar onto the Windows taskbar.
//!
//! Three rules govern everything in here. Each one is a freeze that happened.
//!
//! 1. **Only the UI thread may talk to the shell or to a window.**
//!    `SHAppBarMessage` is a synchronous `SendMessage` into Explorer's tray
//!    thread, and `SetWindowPos` on a window owned by another thread is a
//!    synchronous `SendMessage` into that thread. Issued from a background
//!    thread they deadlock against the shell:
//!
//!    ```text
//!    Explorer tray thread ──broadcast SendMessage──▶ our UI thread (busy)
//!    our poll thread      ──SHAppBarMessage───────▶ Explorer tray thread
//!    ```
//!
//!    Win32 breaks this cycle for a thread that owns a message pump — a nested
//!    `SendMessage` back into a blocked sender is dispatched rather than
//!    queued — but a background thread has no pump and no such protection.
//!    When the shell's thread is caught in that cycle every window on the
//!    desktop stops responding while the cursor, drawn by DWM, keeps moving.
//!    So all of it runs from `tick_dock` on the window timer, and the poll
//!    thread does networking and SQLite only.
//!
//! 2. **Talking to the shell is expensive**, so cache it. `SHAppBarMessage`
//!    and the `EnumWindows` desktop sweep are both cached, and `watch_shell`
//!    lets the shell tell us when the cache is stale instead of us asking.
//!
//! 3. **Never hold a lock across a window call.** The locks here guard caches
//!    only; take them, copy, drop, then call.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow};

/// ABE_LEFT / ABE_TOP / ABE_RIGHT / ABE_BOTTOM.
const ABE_LEFT: u32 = 0;
const ABE_RIGHT: u32 = 2;

const BAR_GAP: i32 = 8;
const MIN_BAR_LEN: i32 = 280;
const MIN_BAR_H: i32 = 28;
const MAX_BAR_H: i32 = 80;

/// Backstop for the cached shell geometry. `watch_shell` invalidates it the
/// moment the layout actually moves, so this only covers a shell that changes
/// without saying so — it does not need to be short, and every expiry is one
/// more synchronous call into Explorer.
const GEOM_TTL: Duration = Duration::from_secs(60);
/// The tray cluster only changes when icons come and go, so the `EnumWindows`
/// sweep that measures it can be very stale without anyone noticing.
const CLUSTER_TTL: Duration = Duration::from_secs(30);
/// How often the bar re-inserts itself at the top of the topmost band.
///
/// This cannot be made conditional. The Windows 11 taskbar composites above
/// ordinary topmost windows through DWM, so the bar loses the screen while
/// still winning on HWND z-order and while still owning WS_EX_TOPMOST — there
/// is nothing to test for. Measured: walking GW_HWNDPREV from the bar reaches
/// the top of the z-order without passing Shell_TrayWnd, and the bar is
/// nevertheless invisible until something re-asserts.
///
/// What it *can* be is cheap. This runs on a window timer, so it lands on the
/// thread that owns the window (no cross-thread marshalling) and carries none
/// of the geometry or enumeration work the old 1.5s heartbeat dragged along.
const ZORDER_TICK_MS: u32 = 1500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskbarGeom {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub edge: u32,
}

impl TaskbarGeom {
    fn vertical(&self) -> bool {
        self.edge == ABE_LEFT || self.edge == ABE_RIGHT
    }

    /// Length along the taskbar's long axis.
    fn span(&self) -> i32 {
        if self.vertical() {
            self.height
        } else {
            self.width
        }
    }

    /// Depth across the taskbar's short axis.
    fn thickness(&self) -> i32 {
        if self.vertical() {
            self.width
        } else {
            self.height
        }
    }
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
/// Set by `shell_proc` when the shell says the layout moved.
static GEOM_DIRTY: AtomicBool = AtomicBool::new(true);
/// Set by `shell_proc` on WM_QUERYENDSESSION / WM_ENDSESSION.
static SHELL_EXITING: AtomicBool = AtomicBool::new(false);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
/// Last offset actually used, so a nudge with no stored offset starts from
/// where the bar is rather than from zero.
static LAST_OFFSET: AtomicI64 = AtomicI64::new(i64::MIN);

/// The configured placement, mirrored out of `AppConfig` so `tick_dock` can
/// run entirely inside the window procedure. Reaching back into Tauri state
/// from there would mean taking a lock the UI thread already holds elsewhere.
static BAR_WIDTH: AtomicU32 = AtomicU32::new(340);
static BAR_OFFSET: AtomicI64 = AtomicI64::new(i64::MIN);

fn store_bar_config(width: u32, offset: Option<i32>) {
    BAR_WIDTH.store(width, Ordering::Relaxed);
    BAR_OFFSET.store(
        offset.map(|v| v as i64).unwrap_or(i64::MIN),
        Ordering::Relaxed,
    );
}

fn bar_config() -> (u32, Option<i32>) {
    let off = match BAR_OFFSET.load(Ordering::Relaxed) {
        i64::MIN => None,
        v => Some(v as i32),
    };
    (BAR_WIDTH.load(Ordering::Relaxed), off)
}

static GEOM: Mutex<Option<(TaskbarGeom, Instant)>> = Mutex::new(None);
static CLUSTER: Mutex<Option<(i32, Instant)>> = Mutex::new(None);
static LAST_PLACE: Mutex<Option<BarPlacement>> = Mutex::new(None);

/// A panic must not brick docking for the rest of the session, and every value
/// behind these locks is a cache that a torn write cannot corrupt.
fn guard<T>(m: &'static Mutex<T>) -> MutexGuard<'static, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn set_dragging(on: bool) {
    DRAGGING.store(on, Ordering::SeqCst);
}

pub fn is_dragging() -> bool {
    DRAGGING.load(Ordering::SeqCst)
}

/// True once the shell has asked us to shut down. Refusing to exit at that
/// point hangs logoff and shutdown behind a "this app is preventing you from
/// shutting down" prompt.
pub fn shell_exiting() -> bool {
    SHELL_EXITING.load(Ordering::Relaxed)
}

fn remember_offset(off: i32) {
    LAST_OFFSET.store(off as i64, Ordering::Relaxed);
}

fn remembered_offset() -> Option<i32> {
    match LAST_OFFSET.load(Ordering::Relaxed) {
        i64::MIN => None,
        v => Some(v as i32),
    }
}

/* ── shell geometry ──────────────────────────────────────────────────────── */

#[cfg(windows)]
fn read_taskbar_geometry() -> Option<TaskbarGeom> {
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
fn read_taskbar_geometry() -> Option<TaskbarGeom> {
    None
}

/// Cached taskbar geometry. The underlying call blocks on Explorer, so it runs
/// only when the shell told us something moved or the backstop TTL expired.
pub fn taskbar_geometry() -> Option<TaskbarGeom> {
    let dirty = GEOM_DIRTY.swap(false, Ordering::Relaxed);
    let cached = *guard(&GEOM);

    if !dirty {
        if let Some((geom, at)) = cached {
            if at.elapsed() < GEOM_TTL {
                return Some(geom);
            }
        }
    }

    match read_taskbar_geometry() {
        Some(geom) => {
            *guard(&GEOM) = Some((geom, Instant::now()));
            if cached.map(|(g, _)| g) != Some(geom) {
                // The taskbar moved, so whatever we measured on it is wrong.
                *guard(&CLUSTER) = None;
            }
            Some(geom)
        }
        // Explorer restarting: hold the last known good rather than undocking.
        None => cached.map(|(g, _)| g),
    }
}

fn placement_limits(tb: &TaskbarGeom, bar_width: u32) -> (i32, i32, i32) {
    let span = tb.span();
    let len = (bar_width as i32)
        .min((span - 2 * BAR_GAP).max(MIN_BAR_LEN))
        .max(MIN_BAR_LEN);
    let thick = tb.thickness().clamp(MIN_BAR_H, MAX_BAR_H);
    let max_off = if tb.vertical() {
        span - thick
    } else {
        span - len
    };
    (len, thick, max_off.max(0))
}

fn bar_rect(bar_width: u32, self_hwnd: isize, offset: Option<i32>) -> Option<BarPlacement> {
    let tb = taskbar_geometry()?;
    let (len, thick, max_off) = placement_limits(&tb, bar_width);

    if tb.vertical() {
        // A horizontal strip cannot live inside a ~60px column. Sizing it to
        // the column's *length* would produce a full-screen-height topmost
        // window, so instead it sits just outside the taskbar's inner edge and
        // rides up from the bottom of the screen.
        let off = offset.unwrap_or(0).clamp(0, max_off);
        remember_offset(off);
        let x = if tb.edge == ABE_LEFT {
            tb.x + tb.width + BAR_GAP
        } else {
            tb.x - len - BAR_GAP
        };
        let y = tb.y + tb.span() - thick - BAR_GAP - off;
        return Some(BarPlacement {
            x,
            y,
            w: len,
            h: thick,
        });
    }

    let x = match offset {
        Some(off) => {
            let off = off.clamp(0, max_off);
            remember_offset(off);
            tb.x + off
        }
        None => {
            let x = (right_cluster_left(&tb, self_hwnd) - BAR_GAP - len).max(tb.x);
            remember_offset((x - tb.x).clamp(0, max_off));
            x
        }
    };

    Some(BarPlacement {
        x,
        y: tb.y,
        w: len,
        h: thick,
    })
}

pub fn nudge_offset(current: Option<i32>, dx: i32, bar_width: u32) -> Option<i32> {
    let tb = taskbar_geometry()?;
    let (_, _, max_off) = placement_limits(&tb, bar_width);
    let base = current.or_else(remembered_offset).unwrap_or(0);
    Some((base + dx).clamp(0, max_off))
}

/// Left edge of the right-hand taskbar cluster (clock, tray, TrafficMonitor, …).
fn right_cluster_left(tb: &TaskbarGeom, self_hwnd: isize) -> i32 {
    if let Some((val, at)) = *guard(&CLUSTER) {
        if at.elapsed() < CLUSTER_TTL {
            return val;
        }
    }
    let fallback = tb.x + tb.width - 180;
    let mut best = system_tray_left().unwrap_or(fallback);
    occupy_scan(tb, self_hwnd, &mut best);
    *guard(&CLUSTER) = Some((best, Instant::now()));
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
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::LPARAM;
    use windows::Win32::UI::WindowsAndMessaging::{EnumChildWindows, EnumWindows, FindWindowW};

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

/* ── shell notifications ─────────────────────────────────────────────────── */

#[cfg(windows)]
const SUBCLASS_ID: usize = 0x9B0A_2C11;
#[cfg(windows)]
const ZORDER_TIMER: usize = 0x9B0A_2C12;

/// Subscribe to the layout and shutdown messages the shell already broadcasts,
/// so the dock heartbeat can be a slow backstop instead of a poll.
#[cfg(windows)]
pub fn watch_shell(window: &WebviewWindow) {
    use windows::core::w;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::SetWindowSubclass;
    use windows::Win32::UI::WindowsAndMessaging::{RegisterWindowMessageW, SetTimer};

    let Ok(raw) = window.hwnd() else {
        return;
    };
    unsafe {
        let hwnd = HWND(raw.0 as *mut _);
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
        let _ = SetWindowSubclass(hwnd, Some(shell_proc), SUBCLASS_ID, 0);
        SetTimer(hwnd, ZORDER_TIMER, ZORDER_TICK_MS, None);
    }
}

#[cfg(not(windows))]
pub fn watch_shell(_window: &WebviewWindow) {}

#[cfg(windows)]
unsafe extern "system" fn shell_proc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
    _id: usize,
    _data: usize,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        KillTimer, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ENDSESSION, WM_NCDESTROY,
        WM_QUERYENDSESSION, WM_SETTINGCHANGE, WM_TIMER,
    };

    // Everything here is flag-only or a single local call: this runs on the UI
    // thread's message pump and must never block it.
    match msg {
        WM_SETTINGCHANGE | WM_DISPLAYCHANGE | WM_DPICHANGED => {
            GEOM_DIRTY.store(true, Ordering::Relaxed);
        }
        WM_TIMER if wparam.0 == ZORDER_TIMER => unsafe {
            tick_dock(hwnd);
            return windows::Win32::Foundation::LRESULT(0);
        },
        WM_QUERYENDSESSION => SHELL_EXITING.store(true, Ordering::Relaxed),
        WM_ENDSESSION => {
            if wparam.0 != 0 {
                SHELL_EXITING.store(true, Ordering::Relaxed);
            }
        }
        WM_NCDESTROY => unsafe {
            let _ = KillTimer(hwnd, ZORDER_TIMER);
            let _ = RemoveWindowSubclass(hwnd, Some(shell_proc), SUBCLASS_ID);
        },
        other => {
            let created = TASKBAR_CREATED.load(Ordering::Relaxed);
            if created != 0 && other == created {
                // Explorer restarted; the styles and placement are both gone.
                FIRST_DOCK.store(true, Ordering::SeqCst);
                GEOM_DIRTY.store(true, Ordering::Relaxed);
                *guard(&LAST_PLACE) = None;
            }
        }
    }

    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/* ── docking ─────────────────────────────────────────────────────────────── */

/// Apply a placement immediately.
///
/// Must be called from the UI thread — the first dock in `setup`, or a command
/// handler. Background threads publish their intent through `store_bar_config`
/// and let `tick_dock` apply it; see rule 1 at the top of this file.
pub fn dock_bar(window: &WebviewWindow, bar_width: u32, offset: Option<i32>) -> Result<(), String> {
    store_bar_config(bar_width, offset);
    let self_hwnd = window.hwnd().map(|h| h.0 as isize).unwrap_or(0);
    let Some(place) = bar_rect(bar_width, self_hwnd, offset) else {
        return Ok(());
    };

    let first = FIRST_DOCK.swap(false, Ordering::SeqCst);
    let unchanged = {
        // Compare-and-store only. See the module note: no window call may run
        // while this is held.
        let mut last = guard(&LAST_PLACE);
        let same = !first && *last == Some(place);
        if !same {
            *last = Some(place);
        }
        same
    };

    if unchanged {
        // Staying above the taskbar is the timer's job, not this one's.
        ensure_visible(window);
        return Ok(());
    }

    if first {
        let _ = window.set_skip_taskbar(true);
        let _ = window.set_shadow(false);
        let _ = window.set_always_on_top(true);
        let _ = window.set_size(PhysicalSize::new(place.w as u32, place.h as u32));
        let _ = window.set_position(PhysicalPosition::new(place.x, place.y));
        if window.is_minimized().unwrap_or(false) {
            let _ = window.unminimize();
        }
        let _ = window.show();
        // After show(), not before: tao re-applies its own cached ex-style when
        // the visible flag changes, which drops WS_EX_TOOLWINDOW if we set it
        // first — and then the bar turns up in Alt-Tab.
        restore_toplevel(window);
        pin_at(window, place, true);
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
    // The style edits above only take effect once the frame is recalculated,
    // which the first `pin_at` does with SWP_FRAMECHANGED.
}

#[cfg(not(windows))]
fn restore_toplevel(_window: &WebviewWindow) {}

#[cfg(windows)]
fn pin_at(window: &WebviewWindow, place: BarPlacement, first: bool) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOP, HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOZORDER,
    };

    let Ok(raw) = window.hwnd() else {
        return;
    };
    unsafe {
        let hwnd = HWND(raw.0 as *mut _);
        let flags = if first {
            SWP_NOACTIVATE | SWP_FRAMECHANGED
        } else {
            SWP_NOACTIVATE | SWP_NOZORDER
        };
        let insert = if first { HWND_TOPMOST } else { HWND_TOP };
        let _ = SetWindowPos(hwnd, insert, place.x, place.y, place.w, place.h, flags);
    }
}

#[cfg(not(windows))]
fn pin_at(_window: &WebviewWindow, _place: BarPlacement, _first: bool) {}

/// The entire dock update, run on the UI thread from the window timer.
///
/// This is the only place that reads shell geometry or moves the window once
/// the app is up. Nothing here touches Tauri, allocates, or takes a lock for
/// longer than a compare-and-store.
#[cfg(windows)]
unsafe fn tick_dock(hwnd: windows::Win32::Foundation::HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };

    unsafe {
        // A drag owns the position; only defend the z-order.
        if is_dragging() {
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            return;
        }

        let (width, offset) = bar_config();
        let place = bar_rect(width, hwnd.0 as isize, offset);

        let moved = match place {
            Some(p) => {
                let mut last = guard(&LAST_PLACE);
                let changed = *last != Some(p);
                if changed {
                    *last = Some(p);
                }
                changed.then_some(p)
            }
            None => None,
        };

        match moved {
            // Position and z-order in a single call.
            Some(p) => {
                let _ = SetWindowPos(hwnd, HWND_TOPMOST, p.x, p.y, p.w, p.h, SWP_NOACTIVATE);
            }
            None => {
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
        show_raw(hwnd);
    }
}

#[cfg(windows)]
unsafe fn show_raw(hwnd: windows::Win32::Foundation::HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{IsWindowVisible, ShowWindow, SW_SHOWNOACTIVATE};
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }
}

/// Raw `ShowWindow`, not `WebviewWindow::show`. The Tauri call marshals to the
/// UI thread and blocks on the reply, which is exactly what the poll thread
/// must not do on every heartbeat.
#[cfg(windows)]
fn ensure_visible(window: &WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{IsWindowVisible, ShowWindow, SW_SHOWNOACTIVATE};

    let Ok(raw) = window.hwnd() else {
        return;
    };
    unsafe {
        let hwnd = HWND(raw.0 as *mut _);
        if !IsWindowVisible(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }
}

#[cfg(not(windows))]
fn ensure_visible(window: &WebviewWindow) {
    if !window.is_visible().unwrap_or(true) {
        let _ = window.show();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tb(edge: u32, x: i32, y: i32, w: i32, h: i32) -> TaskbarGeom {
        TaskbarGeom {
            x,
            y,
            width: w,
            height: h,
            edge,
        }
    }

    #[test]
    fn a_vertical_taskbar_does_not_produce_a_screen_height_bar() {
        // ABE_LEFT on a 1920x1080 screen: the column is 60 wide, full height.
        let (len, thick, max_off) = placement_limits(&tb(ABE_LEFT, 0, 0, 60, 1080), 340);
        assert_eq!(len, 340, "the bar keeps its own width, not the column's");
        assert_eq!(thick, 60, "height follows the column depth, not its length");
        assert!(thick <= MAX_BAR_H);
        assert_eq!(max_off, 1080 - 60, "it slides along the column instead");
    }

    #[test]
    fn a_horizontal_taskbar_sizes_along_its_width() {
        let (len, thick, max_off) = placement_limits(&tb(3, 0, 1032, 1920, 48), 340);
        assert_eq!(len, 340);
        assert_eq!(thick, 48);
        assert_eq!(max_off, 1920 - 340);
    }

    #[test]
    fn the_bar_never_shrinks_below_its_minimum() {
        let (len, _, max_off) = placement_limits(&tb(3, 0, 0, 200, 48), 340);
        assert_eq!(len, MIN_BAR_LEN);
        assert_eq!(max_off, 0, "no room to slide when the bar overhangs");
    }
}
