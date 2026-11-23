use eframe::egui;

use crate::config;

use super::SettingsApp;

impl SettingsApp {
    pub(crate) fn draw_title_bar(&self, ui: &mut egui::Ui, ctx: &egui::Context) -> bool {
        let mut should_close = false;

        let bar_rect =
            egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), 28.0));
        let drag = ui.interact(
            bar_rect,
            egui::Id::unique("title_drag"),
            egui::Sense::drag(),
        );
        if drag.dragged() {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        ui.horizontal(|ui| {
            ui.set_min_height(28.0);
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("Sunrise Brightness v{}", config::VERSION))
                    .strong()
                    .size(18.0),
            );

            let latest = self.state.latest_version.read().unwrap().clone();
            if let Some(ref latest) = latest
                && latest.as_str() != config::VERSION
                && ui
                    .small_button(format!("v{latest} available"))
                    .on_hover_text("Open releases page")
                    .clicked()
            {
                ctx.open_url(egui::OpenUrl {
                    url: format!("{}/releases/latest", config::REPO_URL),
                    new_tab: true,
                });
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("✕").on_hover_text("Hide to tray").clicked() {
                    should_close = true;
                }
            });
        });

        should_close
    }
}
