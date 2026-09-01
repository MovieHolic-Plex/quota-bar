use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow};

#[derive(Clone, Copy, Debug)]
pub struct TaskbarGeom {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub edge: u32,
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

/// Place the bar on the taskbar strip, immediately left of the system tray/clock.
/// Win11 XAML hides explorer child windows, so this stays a top-level TOPMOST popup.
pub fn bar_rect(bar_width: u32) -> Option<(PhysicalPosition<i32>, PhysicalSize<u32>)> {
    let tb = taskbar_geometry()?;
    let width = bar_width.min(tb.width.saturating_sub(16) as u32).max(300) as i32;
    let height = tb.height.max(40);
    let gap = 6;

    let notify = system_tray_rect();
    let (x, y) = match tb.edge {
        // top
        1 => {
            let right = notify
                .map(|n| n.0)
                .unwrap_or(tb.x + tb.width - 140)
                .min(tb.x + tb.width);
            (right - gap - width, tb.y)
        }
        // left
        0 => (tb.x, tb.y + tb.height - gap - width),
        // right
        2 => (tb.x, tb.y + 8),
        // bottom (Win11 default)
        _ => {
            let right = notify
                .map(|n| n.0)
                .filter(|left| *left > tb.x + width)
                .unwrap_or(tb.x + tb.width - 180);
            ((right - gap - width).max(tb.x), tb.y)
        }
    };

    Some((
        PhysicalPosition::new(x, y),
        PhysicalSize::new(width as u32, height as u32),
    ))
}

#[cfg(windows)]
fn system_tray_rect() -> Option<(i32, i32, i32, i32)> {
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
        Some((rc.left, rc.top, rc.right, rc.bottom))
    }
}

#[cfg(not(windows))]
fn system_tray_rect() -> Option<(i32, i32, i32, i32)> {
    None
}

pub fn dock_bar(window: &WebviewWindow, bar_width: u32) -> Result<(), String> {
    let _ = window.set_skip_taskbar(true);
    let _ = window.set_shadow(false);
    let _ = window.set_always_on_top(true);

    restore_toplevel(window);

    if let Some((pos, size)) = bar_rect(bar_width) {
        window.set_size(size).map_err(|e| e.to_string())?;
        window.set_position(pos).map_err(|e| e.to_string())?;
        pin_at(window, pos.x, pos.y, size.width as i32, size.height as i32);
    } else {
        pin_topmost(window);
    }

    if window.is_minimized().unwrap_or(false) {
        let _ = window.unminimize();
    }
    window.show().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(windows)]
fn restore_toplevel(window: &WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, SetParent, SetWindowLongW, GWL_EXSTYLE, GWL_STYLE, HWND_TOPMOST,
        SetWindowPos, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
        WS_CHILD, WS_EX_TOOLWINDOW, WS_POPUP, WS_VISIBLE,
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

        let _ = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_FRAMECHANGED,
        );
    }
}

#[cfg(not(windows))]
fn restore_toplevel(_window: &WebviewWindow) {}

#[cfg(windows)]
fn pin_at(window: &WebviewWindow, x: i32, y: i32, w: i32, h: i32) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    };

    let Ok(raw) = window.hwnd() else {
        return;
    };
    unsafe {
        let hwnd = HWND(raw.0 as *mut _);
        let _ = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            x,
            y,
            w,
            h,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
}

#[cfg(windows)]
fn pin_topmost(window: &WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
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
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
}

#[cfg(not(windows))]
fn pin_topmost(_window: &WebviewWindow) {}
