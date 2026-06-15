use chrono::{DateTime, FixedOffset, Local, NaiveTime, TimeZone};
use serde::Deserialize;
use solar_positioning::{Horizon, spa, time::DeltaT, types::SunriseResult};
use std::time::Duration;

#[derive(Debug, Deserialize)]
struct IpApiResponse {
    lat: f64,
    lon: f64,
    #[serde(default)]
    city: String,
    #[serde(rename = "countryCode", default)]
    country_code: String,
}

pub fn detect_location() -> Option<(f64, f64, String)> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;
    let resp: IpApiResponse = client
        .get("http://ip-api.com/json/?fields=lat,lon,city,countryCode")
        .send()
        .ok()?
        .json()
        .ok()?;
    let label = if resp.city.is_empty() {
        format!("{:.2}, {:.2}", resp.lat, resp.lon)
    } else {
        format!("{}, {}", resp.city, resp.country_code)
    };
    Some((resp.lat, resp.lon, label))
}

pub struct SunTimes {
    pub sunrise: NaiveTime,
    pub transit: NaiveTime,
    pub sunset: NaiveTime,
}

impl SunTimes {
    pub fn fallback() -> Self {
        Self {
            sunrise: NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
            transit: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
            sunset: NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
        }
    }
}

pub fn compute_sun_times(lat: f64, lon: f64) -> Option<SunTimes> {
    let now: DateTime<FixedOffset> = Local::now().fixed_offset();
    let today_start = now.date_naive().and_hms_opt(0, 0, 0)?;
    let today = now.timezone().from_local_datetime(&today_start).single()?;
    let delta_t = DeltaT::estimate_from_date_like(today).unwrap_or(69.0);

    let result =
        spa::sunrise_sunset_for_horizon(today, lat, lon, delta_t, Horizon::SunriseSunset).ok()?;

    match result {
        SunriseResult::RegularDay {
            sunrise,
            transit,
            sunset,
        } => Some(SunTimes {
            sunrise: sunrise.with_timezone(&Local).time(),
            transit: transit.with_timezone(&Local).time(),
            sunset: sunset.with_timezone(&Local).time(),
        }),
        SunriseResult::AllDay { transit } => Some(SunTimes {
            sunrise: NaiveTime::from_hms_opt(0, 0, 0)?,
            transit: transit.with_timezone(&Local).time(),
            sunset: NaiveTime::from_hms_opt(23, 59, 59)?,
        }),
        SunriseResult::AllNight { transit: _ } => None,
    }
}

pub fn current_solar_elevation(lat: f64, lon: f64) -> f64 {
    let now: DateTime<FixedOffset> = Local::now().fixed_offset();
    let delta_t = DeltaT::estimate_from_date_like(now).unwrap_or(69.0);
    spa::solar_position(now, lat, lon, 0.0, delta_t, None)
        .map(|pos| pos.elevation_angle())
        .unwrap_or(0.0)
}

pub fn day_progress(now: NaiveTime, sunrise: NaiveTime, sunset: NaiveTime) -> f64 {
    if now <= sunrise {
        return 0.0;
    }
    if now >= sunset {
        return 1.0;
    }
    let day_secs = (sunset - sunrise).num_seconds() as f64;
    if day_secs <= 0.0 {
        return 0.0;
    }
    ((now - sunrise).num_seconds() as f64 / day_secs).clamp(0.0, 1.0)
}
