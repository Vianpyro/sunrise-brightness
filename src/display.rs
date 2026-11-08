//! Every brightness device the app can drive, whatever the mechanism behind it.
//!
//! - **Backlight**: whatever the `brightness` crate finds. On Windows that is
//!   every monitor, over DDC/CI or the WMI panel API. On Linux it is only
//!   `/sys/class/backlight`, i.e. a laptop's built-in panel.
//! - **DDC/CI** (Linux only): external monitors, driven directly through
//!   `/dev/i2c-*`. This changes the panel's real backlight, unlike a
//!   compositor's software dimming. It needs the `i2c-dev` kernel module and
//!   read/write access to the nodes (`hardware.i2c.enable` on NixOS, or the
//!   `i2c` group on most distributions).

use brightness::blocking::{Brightness, BrightnessDevice, brightness_devices};

pub enum Display {
    Backlight {
        dev: BrightnessDevice,
        name: String,
    },
    #[cfg(target_os = "linux")]
    Ddc(String),
}

impl Display {
    pub fn name(&self) -> &str {
        match self {
            Display::Backlight { name, .. } => name,
            #[cfg(target_os = "linux")]
            Display::Ddc(name) => name,
        }
    }

    /// Sets brightness in percent. Returns whether the device accepted it.
    pub fn set(&self, percent: u32) -> bool {
        match self {
            Display::Backlight { dev, .. } => dev.set(percent).is_ok(),
            #[cfg(target_os = "linux")]
            Display::Ddc(name) => ddc::set(name, percent),
        }
    }

    /// Whether the dim-inactive detector can reason about this device. See
    /// `inactive::display_key`.
    #[cfg(target_os = "linux")]
    pub fn is_ddc(name: &str) -> bool {
        ddc::is_known(name)
    }
}

/// Every controllable device. Cheap: DDC/CI monitors come from a cache that
/// only [`refresh`] rebuilds.
pub fn devices() -> Vec<Display> {
    #[allow(unused_mut)]
    let mut out: Vec<Display> = brightness_devices()
        .flatten()
        .map(|dev| {
            let name = dev.device_name().unwrap_or_default();
            Display::Backlight { dev, name }
        })
        .collect();

    #[cfg(target_os = "linux")]
    out.extend(ddc::names().into_iter().map(Display::Ddc));

    out
}

pub fn names() -> Vec<String> {
    devices().iter().map(|d| d.name().to_string()).collect()
}

/// Re-probes the DDC/CI monitors if the cache is stale: on first call, after a
/// failed write, when a display connector changes state, or when `force` is set.
/// Returns whether the set of monitors changed. Blocking (EDID reads take tens
/// of milliseconds per bus), so call it from the background loop only.
pub fn refresh(force: bool) -> bool {
    #[cfg(target_os = "linux")]
    {
        ddc::refresh(force)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = force;
        false
    }
}

#[cfg(target_os = "linux")]
mod ddc {
    use std::fs;
    use std::sync::Mutex;

    use ddc::{Ddc as _, Edid as _};
    use ddc_i2c::I2cDeviceDdc;

    /// MCCS VCP code for luminance (backlight brightness).
    const VCP_BRIGHTNESS: u8 = 0x10;

    /// Substrings of `/sys/bus/i2c/devices/*/name` for adapters that cannot carry
    /// a display, matched case-insensitively. Motherboard SMBus is the one that
    /// matters: it hosts the RAM's SPD EEPROMs at the very addresses DDC uses
    /// (0x50, and 0x37 for DDR4 page select), so we never send it anything. The
    /// same list ddcutil skips.
    const IGNORED_ADAPTERS: [&str; 5] = ["smbus", "soc:i2cdsi", "smu", "mac-io", "u4"];

    struct Monitor {
        name: String,
        ddc: I2cDeviceDdc,
        max: u16,
    }

    #[derive(Default)]
    struct Cache {
        scanned: bool,
        dirty: bool,
        /// Snapshot of every DRM connector's status, to notice hotplug cheaply.
        connectors: Vec<(String, String)>,
        monitors: Vec<Monitor>,
    }

    static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

    pub fn names() -> Vec<String> {
        let cache = CACHE.lock().unwrap();
        cache
            .as_ref()
            .map(|c| c.monitors.iter().map(|m| m.name.clone()).collect())
            .unwrap_or_default()
    }

    pub fn is_known(name: &str) -> bool {
        let cache = CACHE.lock().unwrap();
        cache
            .as_ref()
            .is_some_and(|c| c.monitors.iter().any(|m| m.name == name))
    }

    pub fn set(name: &str, percent: u32) -> bool {
        let mut guard = CACHE.lock().unwrap();
        let Some(cache) = guard.as_mut() else {
            return false;
        };
        let Some(mon) = cache.monitors.iter_mut().find(|m| m.name == name) else {
            return false;
        };

        let value = (u32::from(mon.max) * percent.min(100) + 50) / 100;
        // DDC/CI over consumer monitors drops the odd transaction; one retry
        // absorbs that without hiding a monitor that is really gone.
        let ok = (0..2).any(|_| {
            mon.ddc
                .set_vcp_feature(VCP_BRIGHTNESS, value as u16)
                .is_ok()
        });
        if !ok {
            cache.dirty = true;
        }
        ok
    }

    pub fn refresh(force: bool) -> bool {
        let connectors = drm_connectors();
        {
            let guard = CACHE.lock().unwrap();
            if let Some(c) = guard.as_ref()
                && c.scanned
                && !c.dirty
                && !force
                && c.connectors == connectors
            {
                return false;
            }
        }

        // Probe without holding the lock: this takes a while, and the old
        // handles stay usable for writes in the meantime.
        let monitors = scan();

        let mut guard = CACHE.lock().unwrap();
        let cache = guard.get_or_insert_with(Cache::default);
        let old: Vec<&str> = cache.monitors.iter().map(|m| m.name.as_str()).collect();
        let new: Vec<&str> = monitors.iter().map(|m| m.name.as_str()).collect();
        let changed = old != new;

        cache.scanned = true;
        cache.dirty = false;
        cache.connectors = connectors;
        cache.monitors = monitors;
        changed
    }

    fn drm_connectors() -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = fs::read_dir("/sys/class/drm")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let status = fs::read_to_string(e.path().join("status")).ok()?;
                Some((
                    e.file_name().to_string_lossy().into_owned(),
                    status.trim().to_string(),
                ))
            })
            .collect();
        out.sort();
        out
    }

    fn scan() -> Vec<Monitor> {
        let mut buses: Vec<u32> = fs::read_dir("/sys/bus/i2c/devices")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name();
                let n: u32 = name.to_str()?.strip_prefix("i2c-")?.parse().ok()?;
                let adapter = fs::read_to_string(e.path().join("name"))
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                (!IGNORED_ADAPTERS.iter().any(|s| adapter.contains(s))).then_some(n)
            })
            .collect();
        buses.sort_unstable();

        let mut found: Vec<(Monitor, Option<String>, u32)> =
            buses.into_iter().filter_map(probe).collect();

        // Two identical monitors would otherwise share one name, and with it one
        // per-monitor override. Disambiguate by serial, else by bus.
        let names: Vec<String> = found.iter().map(|(m, _, _)| m.name.clone()).collect();
        for (mon, serial, bus) in &mut found {
            if names.iter().filter(|n| **n == mon.name).count() > 1 {
                let suffix = serial.clone().unwrap_or_else(|| format!("i2c-{bus}"));
                mon.name = format!("{} ({suffix})", mon.name);
            }
        }

        found.into_iter().map(|(m, _, _)| m).collect()
    }

    /// Opens one bus. Returns `None` unless a monitor with a valid EDID answers
    /// a brightness read, so nothing is ever written to anything else.
    fn probe(bus: u32) -> Option<(Monitor, Option<String>, u32)> {
        let mut ddc = ddc_i2c::from_i2c_device(format!("/dev/i2c-{bus}")).ok()?;

        let mut edid = [0u8; 128];
        let read = ddc.read_edid(0, &mut edid).ok()?;
        if read < edid.len() {
            return None;
        }
        let (model, serial) = parse_edid(&edid)?;

        let vcp = (0..2).find_map(|_| ddc.get_vcp_feature(VCP_BRIGHTNESS).ok())?;
        let max = vcp.maximum();
        if max == 0 {
            return None;
        }

        let name = model.unwrap_or_else(|| format!("i2c-{bus}"));
        Some((Monitor { name, ddc, max }, serial, bus))
    }

    /// Validates the EDID base block and extracts the monitor name (descriptor
    /// 0xFC) and serial (0xFF).
    pub(super) fn parse_edid(edid: &[u8; 128]) -> Option<(Option<String>, Option<String>)> {
        const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
        if edid[..8] != HEADER || edid.iter().fold(0u8, |a, b| a.wrapping_add(*b)) != 0 {
            return None;
        }

        let mut model = None;
        let mut serial = None;
        for desc in edid[54..126].as_chunks::<18>().0 {
            if desc[..3] != [0, 0, 0] {
                continue; // a detailed timing, not a display descriptor
            }
            let text = String::from_utf8_lossy(&desc[5..18])
                .split('\n')
                .next()
                .unwrap_or_default()
                .trim()
                .to_string();
            if text.is_empty() {
                continue;
            }
            match desc[3] {
                0xFC => model = Some(text),
                0xFF => serial = Some(text),
                _ => {}
            }
        }
        Some((model, serial))
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::ddc::parse_edid;

    fn edid_with(descriptors: &[(u8, &str)]) -> [u8; 128] {
        let mut e = [0u8; 128];
        e[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
        for (i, (tag, text)) in descriptors.iter().enumerate() {
            let d = 54 + i * 18;
            e[d + 3] = *tag;
            let mut body = [b' '; 13];
            body[..text.len()].copy_from_slice(text.as_bytes());
            if text.len() < 13 {
                body[text.len()] = b'\n';
            }
            e[d + 5..d + 18].copy_from_slice(&body);
        }
        let sum = e[..127].iter().fold(0u8, |a, b| a.wrapping_add(*b));
        e[127] = 0u8.wrapping_sub(sum);
        e
    }

    #[test]
    fn edid_name_and_serial() {
        let e = edid_with(&[(0xFC, "VG272U W2"), (0xFF, "245218BFA4210")]);
        assert_eq!(
            parse_edid(&e),
            Some((Some("VG272U W2".into()), Some("245218BFA4210".into())))
        );
    }

    #[test]
    fn edid_bad_checksum_rejected() {
        let mut e = edid_with(&[(0xFC, "X")]);
        e[127] = e[127].wrapping_add(1);
        assert_eq!(parse_edid(&e), None);
    }

    #[test]
    fn edid_bad_header_rejected() {
        let mut e = edid_with(&[]);
        e[0] = 1;
        e[127] = e[127].wrapping_sub(1);
        assert_eq!(parse_edid(&e), None);
    }
}
