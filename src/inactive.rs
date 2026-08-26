//! Dims monitors that don't hold the foreground window, to cut backlight power.
//!
//! Polls the foreground window's monitor and re-pushes brightness only when it
//! changes, so an idle 3-screen setup costs one cheap Win32 call twice a second.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::config::{Config, SharedState};

/// How often the foreground monitor is sampled. The user-visible latency to
/// restore a monitor is this plus one DDC/CI round-trip (~100 ms).
const POLL: Duration = Duration::from_millis(500);

/// Brightness multiplier for a monitor, given the currently active display.
///
/// `device_name` is a physical monitor (`\.\DISPLAY1\Monitor0`), `active` is a
/// logical display (`\.\DISPLAY1`), hence the prefix match on the separator —
/// comparing without it would make `DISPLAY1` swallow `DISPLAY10`.
pub fn dim_factor(device_name: &str, config: &Config, active: &Option<String>) -> f64 {
    let Some(active) = active else {
        // Unknown active display (other OS, or no foreground window): never dim.
        return 1.0;
    };
    if !config.dim_inactive || device_name.starts_with(&format!("{active}\\")) {
        1.0
    } else {
        config.dim_inactive_factor.clamp(0.0, 1.0)
    }
}

pub fn run_loop(state: Arc<SharedState>) {
    let mut last: Option<String> = None;

    loop {
        thread::sleep(POLL);

        let enabled = state.config.read().unwrap().dim_inactive;
        if !enabled {
            // Release every monitor once when the feature is switched off.
            if last.take().is_some() {
                *state.active_display.write().unwrap() = None;
                crate::brightness::reapply(&state);
            }
            continue;
        }

        let current = active_display();
        if current != last {
            last.clone_from(&current);
            *state.active_display.write().unwrap() = current;
            crate::brightness::reapply(&state);
        }
    }
}

/// Device name (`\.\DISPLAY1`) of the monitor holding the foreground window.
#[cfg(target_os = "windows")]
fn active_display() -> Option<String> {
    const MONITOR_DEFAULTTONULL: u32 = 0;
    const CCHDEVICENAME: usize = 32;

    #[repr(C)]
    struct MonitorInfoExW {
        cb_size: u32,
        rc_monitor: [i32; 4],
        rc_work: [i32; 4],
        dw_flags: u32,
        sz_device: [u16; CCHDEVICENAME],
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetForegroundWindow() -> isize;
        fn MonitorFromWindow(hwnd: isize, flags: u32) -> isize;
        fn GetMonitorInfoW(hmonitor: isize, info: *mut MonitorInfoExW) -> i32;
    }

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd == 0 {
            return None;
        }
        let hmonitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
        if hmonitor == 0 {
            return None;
        }

        let mut info = MonitorInfoExW {
            cb_size: size_of::<MonitorInfoExW>() as u32,
            rc_monitor: [0; 4],
            rc_work: [0; 4],
            dw_flags: 0,
            sz_device: [0; CCHDEVICENAME],
        };
        if GetMonitorInfoW(hmonitor, &mut info) == 0 {
            return None;
        }

        let len = info
            .sz_device
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(CCHDEVICENAME);
        Some(String::from_utf16_lossy(&info.sz_device[..len]))
    }
}

#[cfg(not(target_os = "windows"))]
fn active_display() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(dim_inactive: bool) -> Config {
        Config {
            dim_inactive,
            dim_inactive_factor: 0.3,
            ..Config::default()
        }
    }

    #[test]
    fn dims_only_the_inactive_displays() {
        let active = Some(r"\.\DISPLAY1".to_string());
        let cfg = config(true);

        assert_eq!(dim_factor(r"\.\DISPLAY1\Monitor0", &cfg, &active), 1.0);
        assert_eq!(dim_factor(r"\.\DISPLAY2\Monitor0", &cfg, &active), 0.3);

        // DISPLAY10 is a different display from DISPLAY1, not a match.
        assert_eq!(dim_factor(r"\.\DISPLAY10\Monitor0", &cfg, &active), 0.3);
    }

    #[test]
    fn never_dims_when_disabled_or_display_is_unknown() {
        let active = Some(r"\.\DISPLAY1".to_string());
        assert_eq!(
            dim_factor(r"\.\DISPLAY2\Monitor0", &config(false), &active),
            1.0
        );
        assert_eq!(
            dim_factor(r"\.\DISPLAY2\Monitor0", &config(true), &None),
            1.0
        );
    }
}
