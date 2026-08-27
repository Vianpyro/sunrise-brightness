use brightness::blocking::{Brightness, brightness_devices};
use chrono::Local;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use crate::config::{Config, SharedState};
use crate::curve;
use crate::inactive;
use crate::solar::{self, SunTimes};
use crate::updater;
use crate::weather;

const FADE_DURATION: Duration = Duration::from_secs(3);

/// A DDC/CI write blocks for ~100 ms per monitor, so more steps than this cannot
/// fit inside [`FADE_DURATION`] anyway and would only add I2C traffic.
const FADE_STEPS: u32 = 8;

/// How long the skip-unchanged-writes cache is trusted before every display is
/// written again regardless. A monitor that was power-cycled or hot-plugged comes
/// back at its own default brightness without telling us, so the cache has to go
/// stale on its own or such a display would keep the wrong value until the curve
/// happened to move.
const REASSERT_AFTER: Duration = Duration::from_secs(3600);

/// Pushes `value` to `dev` unless that is already the last thing we sent it.
///
/// A write costs the same ~100 ms of blocked driver I/O whether or not it
/// changes anything, and the curve is flat for most of the night, so the naive
/// version spends most of its transactions re-asserting a value that is already
/// on the panel. The cache means a brightness set by the monitor's own buttons
/// now survives until the target actually moves, instead of being overwritten
/// on the next tick.
fn push<D: Brightness>(dev: &D, name: &str, value: u32, state: &SharedState) {
    if state.last_written.read().unwrap().get(name) == Some(&value) {
        return;
    }
    if dev.set(value).is_ok() {
        state
            .last_written
            .write()
            .unwrap()
            .insert(name.to_string(), value);
    }
}

/// Re-pushes the last computed targets through the current dim factor.
/// Called when the foreground window moves to another monitor.
pub fn reapply(state: &SharedState) {
    let config = state.config.read().unwrap().clone();
    let targets = state.base_targets.read().unwrap().clone();
    let lit = state.lit_displays.read().unwrap().clone();

    for dev in brightness_devices().flatten() {
        let name = dev.device_name().unwrap_or_default();
        let Some(&base) = targets.get(&name) else {
            continue;
        };
        let factor = inactive::dim_factor(&name, &config, &lit);
        push(&dev, &name, (base as f64 * factor).round() as u32, state);
    }
}

fn fade_brightness(
    from: u32,
    to: u32,
    config: &Config,
    lit: &inactive::LitDisplays,
    state: &SharedState,
) {
    // Enumerating costs ~2.5 ms and the set of monitors cannot change mid-fade,
    // so do it once rather than once per step.
    let devices: Vec<(_, String)> = brightness_devices()
        .flatten()
        .map(|dev| {
            let name = dev.device_name().unwrap_or_default();
            (dev, name)
        })
        .collect();

    let steps = to.abs_diff(from).clamp(1, FADE_STEPS);
    let slot = FADE_DURATION / steps;

    for i in 1..=steps {
        let deadline = Instant::now() + slot;

        let t = i as f32 / steps as f32;
        let eased = t * t * (3.0 - 2.0 * t);
        let value = (from as f32 + (to as f32 - from as f32) * eased).round() as u32;

        for (dev, name) in &devices {
            let factor = inactive::dim_factor(name, config, lit);
            push(dev, name, (value as f64 * factor).round() as u32, state);
        }

        // The writes usually overrun the slot on their own; only sleep if not.
        if let Some(rest) = deadline.checked_duration_since(Instant::now()) {
            thread::sleep(rest);
        }
    }
}

pub fn run_loop(state: Arc<SharedState>) {
    updater::spawn(Arc::clone(&state));

    let mut sun_times: Option<SunTimes> = None;
    let mut location: Option<(f64, f64)> = None;
    let mut last_reassert = Instant::now();

    loop {
        let config = state.config.read().unwrap().clone();

        if last_reassert.elapsed() >= REASSERT_AFTER {
            last_reassert = Instant::now();
            state.last_written.write().unwrap().clear();
        }

        if state.needs_refetch.swap(false, Ordering::Relaxed)
            || sun_times.is_none()
            || location.is_none()
        {
            let monitors = curve::list_display_names();
            *state.detected_monitors.write().unwrap() = monitors;

            let resolved = resolve_location(&config, &state);
            match resolved {
                Some((lat, lon)) => {
                    location = Some((lat, lon));

                    state.set_status("Computing sun times...");
                    match solar::compute_sun_times(lat, lon) {
                        Some(st) => {
                            *state.sunrise_str.write().unwrap() =
                                st.sunrise.format("%H:%M").to_string();
                            *state.noon_str.write().unwrap() =
                                st.transit.format("%H:%M").to_string();
                            *state.sunset_str.write().unwrap() =
                                st.sunset.format("%H:%M").to_string();

                            if config.weather_adaptive
                                && let Some(forecast) =
                                    weather::fetch_forecast(lat, lon, st.sunrise, st.sunset)
                            {
                                *state.weather_forecast.write().unwrap() = forecast;
                            }

                            sun_times = Some(st);
                            state.set_status("Running");
                        }
                        None => {
                            state.set_status("Polar night — no sunrise today");
                            thread::sleep(Duration::from_secs(config.update_interval_secs));
                            continue;
                        }
                    }
                }
                None => {
                    location = None;
                    if let Some(ref st) = sun_times {
                        apply_brightness(&config, &state, st, None);
                    }
                    thread::sleep(Duration::from_secs(config.update_interval_secs));
                    continue;
                }
            }
        }

        if let Some(ref st) = sun_times {
            apply_brightness(&config, &state, st, location);
        }

        thread::sleep(Duration::from_secs(config.update_interval_secs));
    }
}

fn resolve_location(config: &Config, state: &SharedState) -> Option<(f64, f64)> {
    if let (Some(lat), Some(lon)) = (config.latitude, config.longitude) {
        *state.location_str.write().unwrap() = format!("{lat:.2}°, {lon:.2}°");
        return Some((lat, lon));
    }

    state.set_status("Detecting location...");
    match solar::detect_location() {
        Some((lat, lon, label)) => {
            *state.location_str.write().unwrap() = label;
            Some((lat, lon))
        }
        None => {
            *state.location_str.write().unwrap() = "Unknown (using defaults)".into();
            state.set_status("Location detection failed, using 6:00-18:00 defaults");

            *state.sunrise_str.write().unwrap() = "06:00".into();
            *state.noon_str.write().unwrap() = "12:00".into();
            *state.sunset_str.write().unwrap() = "18:00".into();
            None
        }
    }
}

fn apply_brightness(
    config: &Config,
    state: &SharedState,
    st: &SunTimes,
    location: Option<(f64, f64)>,
) {
    let now = Local::now().time();
    let progress = solar::day_progress(now, st.sunrise, st.sunset);

    let elevation_deg = location
        .map(|(lat, lon)| solar::current_solar_elevation(lat, lon))
        .unwrap_or(0.0);

    *state.current_elevation.write().unwrap() = (elevation_deg / 90.0).clamp(0.0, 1.0);
    *state.current_day_progress.write().unwrap() = progress;

    let weather_factor = if config.weather_adaptive {
        let forecast = state.weather_forecast.read().unwrap();
        let cloud = weather::interpolate_cloud_cover(&forecast, progress);
        *state.current_cloud_cover.write().unwrap() = cloud;
        1.0 - cloud * config.cloud_attenuation
    } else {
        1.0
    };

    let base = config.global_curve.evaluate(progress);
    let global_target = (base * weather_factor).clamp(0.0, 100.0) as u32;

    let lit = state.lit_displays.read().unwrap().clone();

    if config.monitors.is_empty() {
        let names = curve::list_display_names();
        *state.base_targets.write().unwrap() =
            names.into_iter().map(|n| (n, global_target)).collect();

        let current = state.current_brightness.load(Ordering::Relaxed);
        if current != global_target {
            fade_brightness(current, global_target, config, &lit, state);
        }
    } else {
        let mut targets = HashMap::new();
        for dev in brightness_devices().flatten() {
            let name = dev.device_name().unwrap_or_default();
            let target = config
                .monitors
                .iter()
                .find(|m| m.name == name)
                .and_then(|m| {
                    m.evaluate(progress, &config.global_curve)
                        .map(|v| (v * weather_factor).clamp(0.0, 100.0))
                })
                .unwrap_or(global_target as f64) as u32;
            let factor = inactive::dim_factor(&name, config, &lit);
            push(&dev, &name, (target as f64 * factor).round() as u32, state);
            targets.insert(name, target);
        }
        *state.base_targets.write().unwrap() = targets;
    }

    state
        .current_brightness
        .store(global_target, Ordering::Relaxed);
}
