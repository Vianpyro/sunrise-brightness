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
    use std::time::{Duration, Instant};

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

    /// Backoff for re-probing a monitor that has an EDID but does not answer
    /// DDC/CI. Usually it is in standby and wakes up within the hour. If its
    /// DDC/CI is switched off in the on-screen menu, it never answers, and the
    /// cap keeps that to one short scan every quarter of an hour.
    const RETRY_MIN: Duration = Duration::from_secs(30);
    const RETRY_MAX: Duration = Duration::from_secs(15 * 60);

    struct Monitor<H> {
        /// Unique display name, what `config.monitors[].name` matches.
        name: String,
        /// EDID model name (or `i2c-N`), before disambiguation.
        base: String,
        serial: Option<String>,
        bus: u32,
        /// Identifies the physical monitor on `bus` across rescans.
        edid: [u8; 128],
        handle: H,
        max: u16,
    }

    /// What one I2C bus looked like during a scan.
    enum Probe<H> {
        /// No valid EDID: nothing is plugged in, or this is not a display bus.
        Absent,
        /// A monitor is there but does not answer DDC/CI. Its EDID EEPROM is
        /// powered by the GPU and stays readable while the panel itself is in
        /// DPMS standby, which is exactly when DDC/CI goes quiet.
        Silent {
            bus: u32,
            edid: [u8; 128],
        },
        Ready(Monitor<H>),
    }

    #[derive(Default)]
    struct Cache {
        scanned: bool,
        dirty: bool,
        /// Snapshot of every DRM connector's status, to notice hotplug cheaply.
        connectors: Vec<(String, String)>,
        monitors: Vec<Monitor<I2cDeviceDdc>>,
        /// Set while some monitor is `Silent` and not yet known: DPMS wake-ups
        /// do not change any connector status, so nothing else would bring it in.
        retry_at: Option<Instant>,
        retry_delay: Duration,
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
            mon.handle
                .set_vcp_feature(VCP_BRIGHTNESS, value as u16)
                .is_ok()
        });
        if !ok {
            // Rescan to tell an unplugged monitor from one in standby. A
            // monitor in standby is kept (see `merge`). Its failed write is not
            // cached by `brightness::push`, so the next tick after it wakes up
            // re-sends the value.
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
                && c.retry_at.is_none_or(|at| Instant::now() < at)
            {
                return false;
            }
        }

        // Probe without holding the lock: this takes a while, and the old
        // handles stay usable for writes in the meantime.
        let probes = scan();

        let mut guard = CACHE.lock().unwrap();
        let cache = guard.get_or_insert_with(Cache::default);
        let old = std::mem::take(&mut cache.monitors);
        let old_names: Vec<String> = old.iter().map(|m| m.name.clone()).collect();
        let (monitors, pending) = merge(old, probes);
        let changed = monitors.iter().map(|m| &m.name).ne(old_names.iter());

        if pending {
            let delay = cache.retry_delay.max(RETRY_MIN);
            cache.retry_at = Some(Instant::now() + delay);
            cache.retry_delay = (delay * 2).min(RETRY_MAX);
        } else {
            cache.retry_at = None;
            cache.retry_delay = RETRY_MIN;
        }

        cache.scanned = true;
        cache.dirty = false;
        cache.connectors = connectors;
        cache.monitors = monitors;
        changed
    }

    /// Combines a fresh scan with what was known before it. Returns the
    /// monitors to use, and whether a `Silent` monitor is still waiting to be
    /// learned (so a re-probe should be scheduled).
    ///
    /// A known monitor that went `Silent` is kept as is, with the same handle,
    /// range and name. Dropping it, as an `Option`-returning probe did, made a
    /// screen in standby disappear until the next hourly rescan, long after it
    /// had woken up. It is matched by bus *and* full EDID, so a different
    /// monitor plugged into the same port is never mistaken for it.
    fn merge<H>(old: Vec<Monitor<H>>, probes: Vec<Probe<H>>) -> (Vec<Monitor<H>>, bool) {
        let mut old: Vec<Option<Monitor<H>>> = old.into_iter().map(Some).collect();
        let mut pending = false;
        let mut out = Vec::new();

        for probe in probes {
            match probe {
                Probe::Absent => {}
                Probe::Ready(m) => out.push(m),
                Probe::Silent { bus, edid } => {
                    let known = old
                        .iter_mut()
                        .find(|o| o.as_ref().is_some_and(|m| m.bus == bus && m.edid == edid))
                        .and_then(Option::take);
                    match known {
                        Some(m) => out.push(m),
                        // Its brightness range is unknown until it answers, so
                        // it cannot be driven yet.
                        None => pending = true,
                    }
                }
            }
        }

        // Two identical monitors would otherwise share one name, and with it one
        // per-monitor override. Disambiguate by serial, else by bus.
        let bases: Vec<String> = out.iter().map(|m| m.base.clone()).collect();
        for m in &mut out {
            m.name = if bases.iter().filter(|b| **b == m.base).count() > 1 {
                let suffix = m.serial.clone().unwrap_or_else(|| format!("i2c-{}", m.bus));
                format!("{} ({suffix})", m.base)
            } else {
                m.base.clone()
            };
        }

        (out, pending)
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

    fn scan() -> Vec<Probe<I2cDeviceDdc>> {
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
        buses.into_iter().map(probe).collect()
    }

    /// Opens one bus. Only a monitor with a valid EDID that answers a
    /// brightness read is `Ready`, so nothing is ever written to anything else.
    fn probe(bus: u32) -> Probe<I2cDeviceDdc> {
        let Ok(mut handle) = ddc_i2c::from_i2c_device(format!("/dev/i2c-{bus}")) else {
            return Probe::Absent;
        };

        let mut edid = [0u8; 128];
        if !handle
            .read_edid(0, &mut edid)
            .is_ok_and(|read| read >= edid.len())
        {
            return Probe::Absent;
        }
        let Some((model, serial)) = parse_edid(&edid) else {
            return Probe::Absent;
        };

        let max = (0..2)
            .find_map(|_| handle.get_vcp_feature(VCP_BRIGHTNESS).ok())
            .map(|vcp| vcp.maximum())
            .filter(|&max| max > 0);
        let Some(max) = max else {
            return Probe::Silent { bus, edid };
        };

        let base = model.unwrap_or_else(|| format!("i2c-{bus}"));
        Probe::Ready(Monitor {
            name: base.clone(),
            base,
            serial,
            bus,
            edid,
            handle,
            max,
        })
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

    #[cfg(test)]
    mod tests {
        use super::{Monitor, Probe, merge};

        fn edid(tag: u8) -> [u8; 128] {
            let mut e = [0u8; 128];
            e[0] = tag;
            e
        }

        fn mon(bus: u32, base: &str, serial: Option<&str>, tag: u8) -> Monitor<()> {
            Monitor {
                name: base.into(),
                base: base.into(),
                serial: serial.map(Into::into),
                bus,
                edid: edid(tag),
                handle: (),
                max: 100,
            }
        }

        fn names(monitors: &[Monitor<()>]) -> Vec<&str> {
            monitors.iter().map(|m| m.name.as_str()).collect()
        }

        #[test]
        fn monitor_in_standby_is_kept() {
            let mut known = mon(6, "Monitor A", None, 1);
            known.max = 75;
            let (out, pending) = merge(
                vec![known],
                vec![Probe::Silent {
                    bus: 6,
                    edid: edid(1),
                }],
            );
            assert_eq!(names(&out), ["Monitor A"]);
            assert_eq!(out[0].max, 75, "keeps the range learnt while awake");
            assert!(!pending);
        }

        #[test]
        fn unplugged_monitor_is_dropped() {
            let (out, pending) = merge(vec![mon(6, "Monitor A", None, 1)], vec![Probe::Absent]);
            assert!(out.is_empty());
            assert!(!pending);
        }

        #[test]
        fn different_monitor_on_same_bus_is_not_confused() {
            let (out, pending) = merge(
                vec![mon(6, "Monitor A", None, 1)],
                vec![Probe::Silent {
                    bus: 6,
                    edid: edid(2),
                }],
            );
            assert!(out.is_empty());
            assert!(pending);
        }

        #[test]
        fn unknown_silent_monitor_schedules_a_retry() {
            let (out, pending) = merge(
                Vec::new(),
                vec![
                    Probe::Ready(mon(5, "Monitor B", None, 1)),
                    Probe::Silent {
                        bus: 6,
                        edid: edid(2),
                    },
                ],
            );
            assert_eq!(names(&out), ["Monitor B"]);
            assert!(pending);
        }

        #[test]
        fn identical_models_are_disambiguated() {
            let (out, _) = merge(
                Vec::new(),
                vec![
                    Probe::Ready(mon(5, "Twin", Some("A1"), 1)),
                    Probe::Ready(mon(7, "Twin", None, 2)),
                ],
            );
            assert_eq!(names(&out), ["Twin (A1)", "Twin (i2c-7)"]);
        }

        #[test]
        fn kept_and_fresh_twins_still_get_distinct_names() {
            let mut asleep = mon(5, "Twin", Some("A1"), 1);
            asleep.name = "Twin (A1)".into();
            let (out, _) = merge(
                vec![asleep],
                vec![
                    Probe::Silent {
                        bus: 5,
                        edid: edid(1),
                    },
                    Probe::Ready(mon(7, "Twin", Some("B2"), 2)),
                ],
            );
            assert_eq!(names(&out), ["Twin (A1)", "Twin (B2)"]);
        }
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
        let e = edid_with(&[(0xFC, "Monitor A"), (0xFF, "SN0000001")]);
        assert_eq!(
            parse_edid(&e),
            Some((Some("Monitor A".into()), Some("SN0000001".into())))
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
