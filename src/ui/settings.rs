use eframe::egui;

use super::SettingsApp;
use crate::config::DimMode;

impl SettingsApp {
    pub(crate) fn draw_settings(&mut self, ui: &mut egui::Ui) -> (bool, bool) {
        egui::Grid::new("settings_grid")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Latitude:");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.lat_input)
                            .hint_text("auto")
                            .desired_width(90.0),
                    );
                    ui.label("Longitude:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.lon_input)
                            .hint_text("auto")
                            .desired_width(90.0),
                    );
                });
                ui.end_row();

                ui.label("");
                ui.horizontal(|ui| {
                    let loc = self.state.location_str.read().unwrap().clone();
                    if !loc.is_empty() {
                        ui.label(
                            egui::RichText::new(format!("Detected: {loc}"))
                                .small()
                                .color(egui::Color32::GRAY),
                        );
                    } else if self.lat_input.is_empty() && self.lon_input.is_empty() {
                        ui.label(
                            egui::RichText::new("(auto-detected from IP)")
                                .small()
                                .color(egui::Color32::GRAY),
                        );
                    }
                });
                ui.end_row();

                ui.label("Update interval:");
                let mut mins = self.update_interval_mins as i32;
                ui.add(egui::Slider::new(&mut mins, 1..=30).suffix(" min"));
                self.update_interval_mins = mins as u64;
                ui.end_row();

                ui.label("Start on boot:");
                ui.checkbox(&mut self.start_on_startup, "");
                ui.end_row();

                ui.label("Adapt to weather:");
                ui.checkbox(&mut self.weather_adaptive, "")
                    .on_hover_text("Dim brightness when cloudy/rainy (Open-Meteo, no API key)");
                ui.end_row();

                ui.label("Dim inactive screens:");
                ui.checkbox(&mut self.dim_inactive, "")
                    .on_hover_text("Dim monitors that don't hold the focused window");
                ui.end_row();

                if self.dim_inactive {
                    ui.label("Dim which screens:");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.dim_mode, DimMode::Empty, "Empty")
                            .on_hover_text("Only screens showing no window at all");
                        ui.selectable_value(&mut self.dim_mode, DimMode::Focused, "Unfocused")
                            .on_hover_text("Every screen but the one holding the focused window");
                    });
                    ui.end_row();

                    // Expressed as how much brightness to cut, so it reads the
                    // same way round as "Cloud dimming" just below it.
                    ui.label("Inactive dimming:");
                    let mut pct = ((1.0 - self.dim_inactive_factor) * 100.0).round() as i32;
                    ui.add(egui::Slider::new(&mut pct, 0..=100).suffix(" %"))
                        .on_hover_text("How much to cut brightness on an idle screen (0% = off)");
                    self.dim_inactive_factor = 1.0 - pct as f64 / 100.0;
                    ui.end_row();
                }

                if self.weather_adaptive {
                    ui.label("Cloud dimming:");
                    let mut att = (self.cloud_attenuation * 100.0) as i32;
                    ui.add(egui::Slider::new(&mut att, 0..=100).suffix(" %"));
                    self.cloud_attenuation = att as f64 / 100.0;
                    ui.end_row();
                }
            });

        let lat_ok = self.lat_input.is_empty() || self.lat_input.parse::<f64>().is_ok();
        let lon_ok = self.lon_input.is_empty() || self.lon_input.parse::<f64>().is_ok();

        if !lat_ok || !lon_ok {
            ui.colored_label(
                egui::Color32::from_rgb(255, 80, 80),
                "Invalid coordinates — use decimal degrees (e.g. 45.5, -73.6)",
            );
        }

        (lat_ok, lon_ok)
    }
}
