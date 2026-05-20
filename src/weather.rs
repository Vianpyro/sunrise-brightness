use chrono::NaiveTime;
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize)]
struct OpenMeteoResponse {
    hourly: OpenMeteoHourly,
}

#[derive(Debug, Deserialize)]
struct OpenMeteoHourly {
    time: Vec<String>,
    cloud_cover: Vec<f64>,
}

pub fn fetch_forecast(
    lat: f64,
    lon: f64,
    sunrise: NaiveTime,
    sunset: NaiveTime,
) -> Option<Vec<(f64, f64)>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;

    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&hourly=cloud_cover&forecast_days=1&timezone=auto"
    );

    let resp: OpenMeteoResponse = client.get(&url).send().ok()?.json().ok()?;

    let day_secs = (sunset - sunrise).num_seconds() as f64;
    if day_secs <= 0.0 {
        return None;
    }

    let mut forecast = Vec::new();
    for (time_str, &cloud_pct) in resp.hourly.time.iter().zip(resp.hourly.cloud_cover.iter()) {
        let Some(time) = time_str
            .split('T')
            .nth(1)
            .and_then(|t| NaiveTime::parse_from_str(t, "%H:%M").ok())
        else {
            continue;
        };

        if time < sunrise || time > sunset {
            continue;
        }

        let progress = (time - sunrise).num_seconds() as f64 / day_secs;
        forecast.push((progress.clamp(0.0, 1.0), cloud_pct / 100.0));
    }

    if forecast.is_empty() || forecast[0].0 > 0.01 {
        forecast.insert(0, (0.0, forecast.first().map(|f| f.1).unwrap_or(0.0)));
    }
    if forecast.last().map(|f| f.0).unwrap_or(0.0) < 0.99 {
        forecast.push((1.0, forecast.last().map(|f| f.1).unwrap_or(0.0)));
    }

    Some(forecast)
}

pub fn interpolate_cloud_cover(forecast: &[(f64, f64)], progress: f64) -> f64 {
    if forecast.is_empty() {
        return 0.0;
    }
    if progress <= forecast[0].0 {
        return forecast[0].1;
    }
    if progress >= forecast[forecast.len() - 1].0 {
        return forecast[forecast.len() - 1].1;
    }
    for i in 0..forecast.len() - 1 {
        let (p0, c0) = forecast[i];
        let (p1, c1) = forecast[i + 1];
        if progress >= p0 && progress <= p1 {
            let t = if (p1 - p0).abs() < 1e-12 {
                0.0
            } else {
                (progress - p0) / (p1 - p0)
            };
            return c0 + t * (c1 - c0);
        }
    }
    0.0
}
