use brightness::blocking::{Brightness, brightness_devices};
use chrono::Local;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use crate::config::{Config, SharedState};
use crate::curve;
use crate::solar::{self, SunTimes};
use crate::updater;
use crate::weather;

fn set_all_displays(target: u32) {
    for dev in brightness_devices().flatten() {
        let _ = dev.set(target);
    }
}

fn fade_brightness(from: u32, to: u32) {
    let duration = Duration::from_millis(3000);
    let start = Instant::now();

    loop {
        let elapsed = start.elapsed();
        let t = (elapsed.as_secs_f32() / duration.as_secs_f32()).min(1.0);

        let eased = t * t * (3.0 - 2.0 * t);

        let value = from as f32 + (to as f32 - from as f32) * eased;
        set_all_displays(value.round() as u32);

        if t >= 1.0 {
            break;
        }

        thread::sleep(Duration::from_millis(16));
    }
}

pub fn run_loop(state: Arc<SharedState>) {
    updater::spawn(Arc::clone(&state));

    let mut sun_times = SunTimes::fallback();
    let mut has_valid_sun_times = false;
    let mut location: Option<(f64, f64)> = None;

    loop {
        let config = state.config.read().unwrap().clone();

        if state.needs_refetch.swap(false, Ordering::Relaxed)
            || location.is_none()
            || !has_valid_sun_times
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

                            sun_times = st;
                            has_valid_sun_times = true;
                            state.set_status("Running");
                        }
                        None => {
                            has_valid_sun_times = false;
                            state.set_status("Polar night — no sunrise today");
                            thread::sleep(Duration::from_secs(config.update_interval_secs));
                            continue;
                        }
                    }
                }
                None => {
                    location = None;
                    apply_brightness(&config, &state, &sun_times, None);
                    thread::sleep(Duration::from_secs(config.update_interval_secs));
                    continue;
                }
            }
        }

        apply_brightness(&config, &state, &sun_times, location);
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

    if config.monitors.is_empty() {
        let current = state.current_brightness.load(Ordering::Relaxed);
        if current != global_target {
            fade_brightness(current, global_target);
            state
                .current_brightness
                .store(global_target, Ordering::Relaxed);
        }
    } else {
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
                .unwrap_or(global_target as f64);
            let _ = dev.set(target as u32);
        }
        state
            .current_brightness
            .store(global_target, Ordering::Relaxed);
    }
}
