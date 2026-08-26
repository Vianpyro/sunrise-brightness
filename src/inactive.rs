//! Dims monitors nothing is happening on, to cut backlight power.
//!
//! Two policies, see `DimMode`: dim everything but the focused screen, or dim
//! only the screens with no window on them at all. Polls cheaply and re-pushes
//! brightness only when the set of lit displays actually changes.
//!
//! The two backends answer the same question by completely different means and
//! at different granularity, so both speak in *display keys* rather than in
//! their own native identifiers:
//!
//! - Windows drives every monitor over DDC/CI, so a key is one logical display
//!   (`\\.\DISPLAY1`) and each screen is judged separately.
//! - Linux only ever exposes the built-in laptop panel through
//!   `/sys/class/backlight`, so there is a single key, [`x11::BUILTIN`]. The
//!   useful case is a docked laptop whose lid is open but unused.

use std::collections::HashSet;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::config::{Config, DimMode, SharedState};

/// How often the lit displays are sampled. The user-visible latency to restore a
/// monitor is this plus one round-trip to the panel (~100 ms over DDC/CI).
const POLL: Duration = Duration::from_millis(500);

/// Display keys that should stay at full brightness.
///
/// `None` means "could not tell" — unsupported platform, no window manager, or
/// a topology the backend cannot reason about. Callers must then dim nothing
/// rather than guess, so a machine we misread stays bright instead of going dark.
pub type LitDisplays = Option<HashSet<String>>;

/// Brightness multiplier for one brightness device.
pub fn dim_factor(device_name: &str, config: &Config, lit: &LitDisplays) -> f64 {
    let Some(lit) = lit else {
        return 1.0;
    };

    if !config.dim_inactive || lit.contains(&display_key(device_name)) {
        1.0
    } else {
        config.dim_inactive_factor.clamp(0.0, 1.0)
    }
}

pub fn run_loop(state: Arc<SharedState>) {
    let mut detector = Detector::new();
    let mut last: LitDisplays = None;

    loop {
        thread::sleep(POLL);

        let (enabled, mode) = {
            let config = state.config.read().unwrap();
            (config.dim_inactive, config.dim_mode)
        };

        if !enabled {
            // Release every monitor once, when the feature is switched off.
            if last.take().is_some() {
                *state.lit_displays.write().unwrap() = None;
                crate::brightness::reapply(&state);
            }
            continue;
        }

        let current = detector.lit_displays(mode);
        if current != last {
            last.clone_from(&current);
            *state.lit_displays.write().unwrap() = current;
            crate::brightness::reapply(&state);
        }
    }
}

/// Holds whatever the platform needs to keep open between polls.
struct Detector {
    #[cfg(target_os = "linux")]
    x11: x11::Detector,
}

impl Detector {
    fn new() -> Self {
        Self {
            #[cfg(target_os = "linux")]
            x11: x11::Detector::new(),
        }
    }

    #[cfg(target_os = "windows")]
    fn lit_displays(&mut self, mode: DimMode) -> LitDisplays {
        win::lit_displays(mode)
    }

    #[cfg(target_os = "linux")]
    fn lit_displays(&mut self, mode: DimMode) -> LitDisplays {
        self.x11.lit_displays(mode)
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    fn lit_displays(&mut self, _mode: DimMode) -> LitDisplays {
        None
    }
}

/// Groups brightness devices that share one screen, in the same vocabulary the
/// platform backend uses to report lit displays.
#[cfg(target_os = "windows")]
fn display_key(device_name: &str) -> String {
    // `\\.\DISPLAY1\Monitor0` is one physical monitor on logical display
    // `\\.\DISPLAY1`. Splitting beats a prefix test, which would let
    // `DISPLAY1` swallow `DISPLAY10`.
    match device_name.rsplit_once('\\') {
        Some((display, _monitor)) => display.to_string(),
        None => device_name.to_string(),
    }
}

/// Every `/sys/class/backlight` entry is the built-in panel.
#[cfg(target_os = "linux")]
fn display_key(_device_name: &str) -> String {
    x11::BUILTIN.to_string()
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn display_key(device_name: &str) -> String {
    device_name.to_string()
}

#[cfg(target_os = "windows")]
mod win {
    use std::collections::HashSet;
    use std::ffi::c_void;

    use crate::config::DimMode;

    use super::LitDisplays;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetForegroundWindow() -> isize;
        fn MonitorFromWindow(hwnd: isize, flags: u32) -> isize;
        fn GetMonitorInfoW(hmonitor: isize, info: *mut MonitorInfoExW) -> i32;
        fn EnumWindows(callback: EnumProc, lparam: isize) -> i32;
        fn IsWindowVisible(hwnd: isize) -> i32;
        fn IsIconic(hwnd: isize) -> i32;
        fn GetWindowLongPtrW(hwnd: isize, index: i32) -> isize;
        fn GetWindowRect(hwnd: isize, rect: *mut Rect) -> i32;
        fn GetClassNameW(hwnd: isize, buf: *mut u16, len: i32) -> i32;
    }

    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmGetWindowAttribute(hwnd: isize, attr: u32, value: *mut c_void, len: u32) -> i32;
    }

    type EnumProc = unsafe extern "system" fn(hwnd: isize, lparam: isize) -> i32;

    const MONITOR_DEFAULTTONULL: u32 = 0;
    const CCHDEVICENAME: usize = 32;
    const GWL_EXSTYLE: i32 = -20;
    const WS_EX_TOOLWINDOW: isize = 0x0000_0080;
    const DWMWA_CLOAKED: u32 = 14;
    const S_OK: i32 = 0;

    /// Shell windows that exist on every monitor and would otherwise make each
    /// one look occupied. `Shell_SecondaryTrayWnd` is the taskbar clone on
    /// non-primary screens — without it, "empty" mode would never fire.
    const SHELL_CLASSES: [&str; 4] = [
        "Progman",
        "WorkerW",
        "Shell_TrayWnd",
        "Shell_SecondaryTrayWnd",
    ];

    pub fn lit_displays(mode: DimMode) -> LitDisplays {
        match mode {
            DimMode::Focused => focused_display().map(|d| HashSet::from([d])),
            DimMode::Empty => occupied_displays(),
        }
    }

    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    impl Rect {
        fn zeroed() -> Self {
            Self {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            }
        }

        fn is_empty(&self) -> bool {
            self.right <= self.left || self.bottom <= self.top
        }
    }

    #[repr(C)]
    struct MonitorInfoExW {
        cb_size: u32,
        rc_monitor: Rect,
        rc_work: Rect,
        dw_flags: u32,
        sz_device: [u16; CCHDEVICENAME],
    }

    /// Display name (`\\.\DISPLAY1`) of the monitor a window mostly sits on.
    fn display_of(hwnd: isize) -> Option<String> {
        unsafe {
            let hmonitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
            if hmonitor == 0 {
                return None;
            }

            let mut info = MonitorInfoExW {
                cb_size: size_of::<MonitorInfoExW>() as u32,
                rc_monitor: Rect::zeroed(),
                rc_work: Rect::zeroed(),
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

    fn focused_display() -> Option<String> {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd == 0 {
            return None;
        }
        display_of(hwnd)
    }

    /// Displays carrying at least one real, user-visible window.
    fn occupied_displays() -> Option<HashSet<String>> {
        let mut found = HashSet::new();
        let ok = unsafe { EnumWindows(collect, &mut found as *mut HashSet<String> as isize) };

        if ok == 0 { None } else { Some(found) }
    }

    unsafe extern "system" fn collect(hwnd: isize, lparam: isize) -> i32 {
        if is_real_window(hwnd)
            && let Some(display) = display_of(hwnd)
        {
            // Safety: `lparam` is the set handed to EnumWindows above. It stays
            // alive for the whole enumeration and is only touched from here.
            unsafe { &mut *(lparam as *mut HashSet<String>) }.insert(display);
        }
        1 // keep enumerating
    }

    /// Whether a window actually paints something a user would notice.
    fn is_real_window(hwnd: isize) -> bool {
        unsafe {
            if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
                return false;
            }
            if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOOLWINDOW != 0 {
                return false;
            }

            // Suspended UWP apps stay "visible" but are cloaked by the compositor.
            let mut cloaked: u32 = 0;
            let cloaked_ptr = (&raw mut cloaked).cast::<c_void>();
            if DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, cloaked_ptr, 4) == S_OK && cloaked != 0 {
                return false;
            }

            let mut rect = Rect::zeroed();
            if GetWindowRect(hwnd, &mut rect) == 0 || rect.is_empty() {
                return false;
            }

            let mut buf = [0u16; 64];
            let len = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            if len > 0 {
                let class = String::from_utf16_lossy(&buf[..len as usize]);
                if SHELL_CLASSES.contains(&class.as_str()) {
                    return false;
                }
            }

            true
        }
    }
}

#[cfg(target_os = "linux")]
mod x11 {
    use std::collections::HashSet;

    use x11rb::connection::Connection as _;
    use x11rb::protocol::randr::{self, ConnectionExt as _};
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, MapState, Window};
    use x11rb::rust_connection::RustConnection;

    use crate::config::DimMode;

    use super::LitDisplays;

    /// The single display key this backend can report on.
    pub const BUILTIN: &str = "builtin";

    /// Connector prefixes the kernel uses for panels wired into the chassis.
    const BUILTIN_PREFIXES: [&str; 3] = ["EDP", "LVDS", "DSI"];

    /// Keeps the X connection open across polls, reconnecting if the server goes.
    pub struct Detector {
        conn: Option<(RustConnection, Window)>,
    }

    impl Detector {
        pub fn new() -> Self {
            Self { conn: None }
        }

        pub fn lit_displays(&mut self, mode: DimMode) -> LitDisplays {
            if self.conn.is_none() {
                let (conn, screen_num) = x11rb::connect(None).ok()?;
                let root = conn.setup().roots.get(screen_num)?.root;
                self.conn = Some((conn, root));
            }
            let (conn, root) = self.conn.as_ref()?;

            let result = panel_is_lit(conn, *root, mode);
            if result.is_none() {
                // Most likely the server went away; drop it so the next poll
                // reconnects instead of spinning on a dead socket.
                self.conn = None;
            }

            result.map(|lit| {
                if lit {
                    HashSet::from([BUILTIN.to_string()])
                } else {
                    HashSet::new()
                }
            })
        }
    }

    struct Rect {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    }

    impl Rect {
        fn center(&self) -> (i32, i32) {
            (self.x + self.w / 2, self.y + self.h / 2)
        }

        fn contains(&self, (x, y): (i32, i32)) -> bool {
            x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
        }
    }

    /// `None` when we cannot tell: no X server, or no built-in panel at all.
    ///
    /// The second case matters — a desktop has no panel, and answering "not lit"
    /// there would dim a `ddcci-backlight` monitor forever.
    fn panel_is_lit(conn: &RustConnection, root: Window, mode: DimMode) -> Option<bool> {
        let panel = builtin_rect(conn, root)?;

        let windows = match mode {
            DimMode::Focused => vec![focused_rect(conn, root)?],
            DimMode::Empty => visible_windows(conn, root)?,
        };

        Some(windows.iter().any(|w| panel.contains(w.center())))
    }

    /// Rect of the built-in panel, or `None` if this machine has no such output.
    fn builtin_rect(conn: &RustConnection, root: Window) -> Option<Rect> {
        let res = conn
            .randr_get_screen_resources_current(root)
            .ok()?
            .reply()
            .ok()?;

        for output in res.outputs {
            let Ok(info) = conn.randr_get_output_info(output, res.config_timestamp) else {
                continue;
            };
            let Ok(info) = info.reply() else { continue };

            if info.connection != randr::Connection::CONNECTED || info.crtc == 0 {
                continue;
            }

            let name = String::from_utf8_lossy(&info.name).to_ascii_uppercase();
            if !BUILTIN_PREFIXES.iter().any(|p| name.starts_with(p)) {
                continue;
            }

            let crtc = conn
                .randr_get_crtc_info(info.crtc, res.config_timestamp)
                .ok()?
                .reply()
                .ok()?;
            return Some(Rect {
                x: crtc.x as i32,
                y: crtc.y as i32,
                w: crtc.width as i32,
                h: crtc.height as i32,
            });
        }
        None
    }

    fn focused_rect(conn: &RustConnection, root: Window) -> Option<Rect> {
        let atom = intern(conn, b"_NET_ACTIVE_WINDOW")?;
        let prop = conn
            .get_property(false, root, atom, AtomEnum::WINDOW, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        let window = prop.value32()?.next()?;

        window_rect(conn, root, window)
    }

    /// Client windows a user would actually notice, as root-relative rects.
    ///
    /// Prefers the WM-maintained `_NET_CLIENT_LIST`: under a reparenting WM the
    /// children of the root are frames, and window-type/state properties live on
    /// the client, so reading them off a frame silently misses every dock.
    fn visible_windows(conn: &RustConnection, root: Window) -> Option<Vec<Rect>> {
        let skip_type = [
            &b"_NET_WM_WINDOW_TYPE_DOCK"[..],
            &b"_NET_WM_WINDOW_TYPE_DESKTOP"[..],
        ]
        .iter()
        .filter_map(|n| intern(conn, n))
        .collect();
        let type_atom = intern(conn, b"_NET_WM_WINDOW_TYPE")?;
        let state_atom = intern(conn, b"_NET_WM_STATE")?;
        let hidden = HashSet::from([intern(conn, b"_NET_WM_STATE_HIDDEN")?]);

        let windows = match client_list(conn, root) {
            Some(list) => list,
            None => conn.query_tree(root).ok()?.reply().ok()?.children,
        };

        let mut rects = Vec::new();
        for w in windows {
            // Panels and the wallpaper cover a screen without anyone using it,
            // and minimised windows stay mapped on some window managers.
            if has_atom(conn, w, type_atom, &skip_type) || has_atom(conn, w, state_atom, &hidden) {
                continue;
            }
            if let Some(rect) = window_rect(conn, root, w) {
                rects.push(rect);
            }
        }
        Some(rects)
    }

    fn window_rect(conn: &RustConnection, root: Window, w: Window) -> Option<Rect> {
        let attrs = conn.get_window_attributes(w).ok()?.reply().ok()?;
        if attrs.map_state != MapState::VIEWABLE || attrs.override_redirect {
            return None;
        }

        let geom = conn.get_geometry(w).ok()?.reply().ok()?;
        if geom.width == 0 || geom.height == 0 {
            return None;
        }

        // Goes through the frame, so this is right under a reparenting WM too.
        let pos = conn
            .translate_coordinates(w, root, 0, 0)
            .ok()?
            .reply()
            .ok()?;
        Some(Rect {
            x: pos.dst_x as i32,
            y: pos.dst_y as i32,
            w: geom.width as i32,
            h: geom.height as i32,
        })
    }

    fn client_list(conn: &RustConnection, root: Window) -> Option<Vec<Window>> {
        let atom = intern(conn, b"_NET_CLIENT_LIST")?;
        let prop = conn
            .get_property(false, root, atom, AtomEnum::WINDOW, 0, u32::MAX)
            .ok()?
            .reply()
            .ok()?;

        let list: Vec<Window> = prop.value32()?.collect();
        if list.is_empty() { None } else { Some(list) }
    }

    fn intern(conn: &RustConnection, name: &[u8]) -> Option<u32> {
        Some(conn.intern_atom(false, name).ok()?.reply().ok()?.atom)
    }

    /// Whether one of `wanted` appears in the atom-list property `prop` of `w`.
    fn has_atom(conn: &RustConnection, w: Window, prop: u32, wanted: &HashSet<u32>) -> bool {
        let Ok(cookie) = conn.get_property(false, w, prop, AtomEnum::ATOM, 0, 32) else {
            return false;
        };
        let Ok(reply) = cookie.reply() else {
            return false;
        };

        reply
            .value32()
            .is_some_and(|mut v| v.any(|a| wanted.contains(&a)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE: &str = if cfg!(target_os = "windows") {
        r"\\.\DISPLAY2\Monitor0"
    } else {
        "intel_backlight"
    };

    fn config(dim_inactive: bool) -> Config {
        Config {
            dim_inactive,
            dim_inactive_factor: 0.3,
            ..Config::default()
        }
    }

    fn lit(devices: &[&str]) -> LitDisplays {
        Some(devices.iter().map(|d| display_key(d)).collect())
    }

    #[test]
    fn keeps_a_device_whose_display_is_lit() {
        assert_eq!(dim_factor(DEVICE, &config(true), &lit(&[DEVICE])), 1.0);
    }

    #[test]
    fn dims_a_device_whose_display_is_not_lit() {
        assert_eq!(dim_factor(DEVICE, &config(true), &lit(&[])), 0.3);
    }

    #[test]
    fn never_dims_when_disabled_or_displays_are_unknown() {
        assert_eq!(dim_factor(DEVICE, &config(false), &lit(&[])), 1.0);
        assert_eq!(dim_factor(DEVICE, &config(true), &None), 1.0);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn groups_monitors_by_logical_display_without_prefix_confusion() {
        assert_eq!(display_key(r"\\.\DISPLAY1\Monitor0"), r"\\.\DISPLAY1");
        assert_ne!(
            display_key(r"\\.\DISPLAY10\Monitor0"),
            display_key(r"\\.\DISPLAY1\Monitor0")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn every_backlight_maps_to_the_builtin_panel() {
        assert_eq!(display_key("intel_backlight"), display_key("amdgpu_bl0"));
    }
}
