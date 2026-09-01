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

pub fn bar_rect(bar_width: u32) -> Option<(PhysicalPosition<i32>, PhysicalSize<u32>)> {
    let tb = taskbar_geometry()?;
    let width = bar_width.min(tb.width as u32).max(280);
    let pad = 8i32;

    // ABE_LEFT=0, ABE_TOP=1, ABE_RIGHT=2, ABE_BOTTOM=3
    let (x, y, w, h) = match tb.edge {
        1 => (tb.x + pad, tb.y, width, tb.height as u32), // top
        0 => (tb.x, tb.y + tb.height - 48, tb.width as u32, 48), // left
        2 => (tb.x, tb.y + tb.height - 48, tb.width as u32, 48), // right
        _ => (tb.x + pad, tb.y, width, tb.height as u32),  // bottom (default)
    };

    Some((PhysicalPosition::new(x, y), PhysicalSize::new(w, h.max(40))))
}

pub fn dock_bar(window: &WebviewWindow, bar_width: u32) -> Result<(), String> {
    if let Some((pos, size)) = bar_rect(bar_width) {
        window.set_size(size).map_err(|e| e.to_string())?;
        window.set_position(pos).map_err(|e| e.to_string())?;
    }
    apply_tool_window(window);
    let _ = window.set_always_on_top(true);
    let _ = window.set_skip_taskbar(true);
    if !window.is_visible().unwrap_or(false) {
        window.show().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(windows)]
fn apply_tool_window(window: &WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, SetWindowLongW, GWL_EXSTYLE, HWND_TOPMOST, SetWindowPos,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, WS_EX_TOOLWINDOW,
    };

    let Ok(raw) = window.hwnd() else {
        return;
    };
    unsafe {
        let hwnd = HWND(raw.0 as *mut _);
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE);
        SetWindowLongW(hwnd, GWL_EXSTYLE, ex | WS_EX_TOOLWINDOW.0 as i32);
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
fn apply_tool_window(_window: &WebviewWindow) {}
