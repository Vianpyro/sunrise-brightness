mod curve_editor;
mod monitors;
mod settings;
mod title_bar;

use eframe::egui;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tray_icon::menu::{MenuEvent, MenuId};

use crate::config::{self, SharedState};
use crate::curve::{BrightnessCurve, MonitorOverride};

pub struct SettingsApp {
    pub(crate) state: Arc<SharedState>,
    pub(crate) settings_menu_id: MenuId,
    pub(crate) quit_menu_id: MenuId,
    pub(crate) visible: bool,
    pub(crate) first_frame: bool,

    pub(crate) lat_input: String,
    pub(crate) lon_input: String,
    pub(crate) update_interval_mins: u64,
    pub(crate) start_on_startup: bool,
    pub(crate) weather_adaptive: bool,
    pub(crate) cloud_attenuation: f64,
    pub(crate) global_curve: BrightnessCurve,
    pub(crate) monitor_overrides: Vec<MonitorOverride>,

    pub(crate) active_tab: usize,
    pub(crate) dragging_point: Option<usize>,
}

impl SettingsApp {
    pub fn new(state: Arc<SharedState>, settings_menu_id: MenuId, quit_menu_id: MenuId) -> Self {
        let config = state.config.read().unwrap().clone();
        Self {
            state,
            settings_menu_id,
            quit_menu_id,
            visible: cfg!(target_os = "linux"),
            first_frame: true,
            lat_input: config
                .latitude
                .map(|v| format!("{v:.4}"))
                .unwrap_or_default(),
            lon_input: config
                .longitude
                .map(|v| format!("{v:.4}"))
                .unwrap_or_default(),
            update_interval_mins: config.update_interval_secs / 60,
            start_on_startup: config.start_on_startup,
            weather_adaptive: config.weather_adaptive,
            cloud_attenuation: config.cloud_attenuation,
            global_curve: config.global_curve,
            monitor_overrides: config.monitors,
            active_tab: 0,
            dragging_point: None,
        }
    }

    pub(crate) fn load_fields_from_config(&mut self) {
        let cfg = self.state.config.read().unwrap().clone();
        self.lat_input = cfg.latitude.map(|v| format!("{v:.4}")).unwrap_or_default();
        self.lon_input = cfg.longitude.map(|v| format!("{v:.4}")).unwrap_or_default();
        self.update_interval_mins = cfg.update_interval_secs / 60;
        self.start_on_startup = cfg.start_on_startup;
        self.weather_adaptive = cfg.weather_adaptive;
        self.cloud_attenuation = cfg.cloud_attenuation;
        self.global_curve = cfg.global_curve;
        self.monitor_overrides = cfg.monitors;
        self.active_tab = 0;
        self.dragging_point = None;
    }

    pub(crate) fn save_and_apply(&mut self) {
        let new_cfg = config::Config {
            latitude: self.lat_input.parse::<f64>().ok(),
            longitude: self.lon_input.parse::<f64>().ok(),
            update_interval_secs: self.update_interval_mins * 60,
            start_on_startup: self.start_on_startup,
            weather_adaptive: self.weather_adaptive,
            cloud_attenuation: self.cloud_attenuation,
            global_curve: self.global_curve.clone(),
            monitors: self.monitor_overrides.clone(),
        };

        if let Err(e) = config::save_config(&new_cfg) {
            eprintln!("Failed to save config: {e}");
        }

        *self.state.config.write().unwrap() = new_cfg;
        self.state.needs_refetch.store(true, Ordering::Relaxed);

        if let Err(e) = toggle_autostart(self.start_on_startup) {
            eprintln!("Auto-start toggle failed: {e}");
        }
    }

    pub(crate) fn active_curve_mut(&mut self) -> Option<&mut BrightnessCurve> {
        if self.active_tab == 0 {
            Some(&mut self.global_curve)
        } else {
            let idx = self.active_tab - 1;
            self.monitor_overrides.get_mut(idx).and_then(|m| {
                if m.mode == "custom" {
                    m.curve.as_mut()
                } else {
                    None
                }
            })
        }
    }

    pub(crate) fn active_curve_display(&self) -> BrightnessCurve {
        if self.active_tab == 0 {
            return self.global_curve.clone();
        }
        let idx = self.active_tab - 1;
        match self.monitor_overrides.get(idx) {
            Some(m) if m.mode == "custom" => m.curve.clone().unwrap_or_default(),
            _ => self.global_curve.clone(),
        }
    }

    pub(crate) fn active_curve_is_editable(&self) -> bool {
        self.active_tab == 0
            || self
                .monitor_overrides
                .get(self.active_tab.wrapping_sub(1))
                .is_some_and(|m| m.mode == "custom")
    }
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if self.first_frame {
            self.first_frame = false;
            // On Windows, hide to tray on startup. On Linux, the tray
            // may not work (GNOME/Wayland), so show the window directly.
            #[cfg(not(target_os = "linux"))]
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.settings_menu_id {
                self.load_fields_from_config();
                self.visible = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            } else if event.id == self.quit_menu_id {
                std::process::exit(0);
            }
        }

        if ctx.input(|i| i.viewport().close_requested()) {
            #[cfg(target_os = "linux")]
            std::process::exit(0);
            #[cfg(not(target_os = "linux"))]
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                self.visible = false;
            }
        }

        ctx.request_repaint_after(Duration::from_millis(250));
        if !self.visible {
            return;
        }

        let mut should_close = false;

        egui::CentralPanel::default().show(ui, |ui| {
            should_close = self.draw_title_bar(ui, &ctx);
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                let (lat_ok, lon_ok) = self.draw_settings(ui);

                ui.separator();

                let monitors = self.state.detected_monitors.read().unwrap().clone();
                self.draw_monitor_tabs(ui, &monitors);
                ui.add_space(4.0);

                if self.active_tab > 0 {
                    self.draw_monitor_mode_selector(ui);
                    ui.add_space(4.0);
                }

                if self.active_curve_is_editable() {
                    self.draw_preset_buttons(ui);
                    ui.add_space(4.0);
                }

                let elevation = *self.state.current_elevation.read().unwrap();
                let day_progress = *self.state.current_day_progress.read().unwrap();

                let is_ignored = self.active_tab > 0
                    && self
                        .monitor_overrides
                        .get(self.active_tab.wrapping_sub(1))
                        .is_some_and(|m| m.mode == "ignored");

                if is_ignored {
                    ui.add_space(8.0);
                    ui.colored_label(
                        egui::Color32::GRAY,
                        "This display is ignored, brightness will not be adjusted.",
                    );
                    ui.add_space(8.0);
                } else {
                    let weather_forecast = if self.weather_adaptive {
                        self.state.weather_forecast.read().unwrap().clone()
                    } else {
                        Vec::new()
                    };

                    self.draw_curve_editor(
                        ui,
                        elevation,
                        day_progress,
                        &weather_forecast,
                        self.cloud_attenuation,
                    );

                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new(
                            "Drag points to reshape. Double-click to add. Right-click to remove. Hold Shift to disable snapping.",
                        )
                        .small()
                        .color(egui::Color32::GRAY),
                    );
                }

                ui.separator();

                self.draw_status(ui, elevation);
                ui.separator();

                ui.horizontal(|ui| {
                    let valid = lat_ok && lon_ok;
                    if ui
                        .add_enabled(valid, egui::Button::new("Save & Apply"))
                        .clicked()
                    {
                        self.save_and_apply();
                    }
                    if ui.button("Close").clicked() {
                        should_close = true;
                    }
                });

                ui.add_space(8.0);
                ui.separator();

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(
                        egui::RichText::new("Made by Vianpyro")
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                    ui.label(
                        egui::RichText::new("·")
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                    if ui
                        .small_button("GitHub")
                        .on_hover_text(config::REPO_URL)
                        .clicked()
                    {
                        ctx.open_url(egui::OpenUrl {
                            url: config::REPO_URL.to_string(),
                            new_tab: true,
                        });
                    }
                });
            });
        });

        if should_close {
            #[cfg(target_os = "linux")]
            std::process::exit(0);
            #[cfg(not(target_os = "linux"))]
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                self.visible = false;
            }
        }
    }
}

impl SettingsApp {
    fn draw_status(&self, ui: &mut egui::Ui, elevation: f64) {
        let status = self.state.status.read().unwrap().clone();
        let brightness = self.state.current_brightness.load(Ordering::Relaxed);
        let sunrise = self.state.sunrise_str.read().unwrap().clone();
        let noon = self.state.noon_str.read().unwrap().clone();
        let sunset = self.state.sunset_str.read().unwrap().clone();

        egui::Grid::new("status_grid")
            .num_columns(2)
            .spacing([12.0, 3.0])
            .show(ui, |ui| {
                ui.label("Status:");
                ui.label(&status);
                ui.end_row();

                ui.label("Brightness:");
                ui.label(format!("{brightness} %"));
                ui.end_row();

                if !sunrise.is_empty() {
                    ui.label("Sun:");
                    ui.label(format!("{sunrise} / {noon} / {sunset}"));
                    ui.end_row();

                    ui.label("Elevation:");
                    ui.label(format!("{:.1}°", elevation * 90.0));
                    ui.end_row();
                }

                if self.weather_adaptive {
                    let cloud = *self.state.current_cloud_cover.read().unwrap();
                    ui.label("Cloud cover:");
                    ui.label(format!("{:.0} %", cloud * 100.0));
                    ui.end_row();
                }
            });
    }
}

fn toggle_autostart(enable: bool) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?.to_string_lossy().to_string();

    let launcher = auto_launch::AutoLaunchBuilder::new()
        .set_app_name("sunrise-brightness")
        .set_app_path(&exe)
        .set_args(&["--startup"])
        .build()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if enable {
        launcher.enable().map_err(|e| anyhow::anyhow!("{e}"))?;
    } else {
        launcher.disable().map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    Ok(())
}
