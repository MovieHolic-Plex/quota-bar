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
        let ok = SHAppBarMessage(ABM_GETTASKBARPOS, &mut data);
        if ok == 0 {
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

#[cfg(windows)]
fn tray_hwnd() -> Option<windows::Win32::Foundation::HWND> {
    use windows::core::{w, PCWSTR};
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

    unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()).ok() }
}

pub fn bar_rect(bar_width: u32) -> Option<(PhysicalPosition<i32>, PhysicalSize<u32>)> {
    let tb = taskbar_geometry()?;
    let width = bar_width.min(tb.width as u32).max(300);

    // Sit flush inside the taskbar strip (left side of a bottom/top bar).
    let (x, y, w, h) = match tb.edge {
        1 | 3 => (tb.x, tb.y, width, tb.height as u32),
        0 => (tb.x, tb.y, tb.width as u32, width.min(tb.height as u32)),
        2 => (tb.x, tb.y, tb.width as u32, width.min(tb.height as u32)),
        _ => (tb.x, tb.y, width, tb.height as u32),
    };

    Some((PhysicalPosition::new(x, y), PhysicalSize::new(w, h.max(40))))
}

pub fn dock_bar(window: &WebviewWindow, bar_width: u32) -> Result<(), String> {
    let _ = window.set_skip_taskbar(true);
    let _ = window.set_shadow(false);
    let _ = window.set_always_on_top(true);

    let embedded = {
        #[cfg(windows)]
        {
            embed_into_taskbar(window, bar_width)
        }
        #[cfg(not(windows))]
        {
            false
        }
    };

    if !embedded {
        if let Some((pos, size)) = bar_rect(bar_width) {
            window.set_size(size).map_err(|e| e.to_string())?;
            window.set_position(pos).map_err(|e| e.to_string())?;
        }
    }

    if !window.is_visible().unwrap_or(false) {
        window.show().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(windows)]
fn embed_into_taskbar(window: &WebviewWindow, bar_width: u32) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetParent, GetWindowLongW, SetParent, SetWindowLongW, SetWindowPos,
        GWL_EXSTYLE, GWL_STYLE, HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_SHOWWINDOW,
        WS_CHILD, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_POPUP, WS_VISIBLE,
    };

    let Ok(raw) = window.hwnd() else {
        return false;
    };
    let Some(tb) = taskbar_geometry() else {
        return false;
    };
    let Some(tray) = tray_hwnd() else {
        return false;
    };

    unsafe {
        let hwnd = HWND(raw.0 as *mut _);
        let parent = GetParent(hwnd).unwrap_or_default();
        if parent != tray {
            let mut style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
            style &= !WS_POPUP.0;
            style |= WS_CHILD.0 | WS_VISIBLE.0;
            SetWindowLongW(hwnd, GWL_STYLE, style as i32);

            let mut ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            ex |= WS_EX_TOOLWINDOW.0;
            ex &= !WS_EX_LAYERED.0;
            SetWindowLongW(hwnd, GWL_EXSTYLE, ex as i32);

            let _ = SetParent(hwnd, tray);
        }

        let width = bar_width.min(tb.width as u32).max(300) as i32;
        let (rel_x, rel_y, w, h) = match tb.edge {
            1 | 3 => (0, 0, width, tb.height),
            0 | 2 => (0, 0, tb.width, tb.height.min(48)),
            _ => (0, 0, width, tb.height),
        };

        let _ = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            rel_x,
            rel_y,
            w,
            h,
            SWP_SHOWWINDOW | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
    true
}
