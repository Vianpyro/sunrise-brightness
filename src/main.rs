#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod brightness;
mod config;
mod curve;
mod inactive;
mod solar;
mod ui;
mod updater;
mod weather;

use anyhow::Result;
use eframe::egui;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tray_icon::{
    TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

fn main() -> Result<()> {
    #[cfg(target_os = "linux")]
    gtk::init().expect("Failed to initialize GTK");

    let cfg = config::load_config();
    let state = Arc::new(config::SharedState::new(cfg));

    let state_bg = Arc::clone(&state);
    std::thread::spawn(move || brightness::run_loop(state_bg));

    let state_dim = Arc::clone(&state);
    std::thread::spawn(move || inactive::run_loop(state_dim));

    let tray_menu = Menu::new();
    let item_settings = MenuItem::new("Settings", true, None);
    let item_quit = MenuItem::new("Quit", true, None);
    tray_menu.append(&item_settings)?;
    tray_menu.append(&PredefinedMenuItem::separator())?;
    tray_menu.append(&item_quit)?;

    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip(format!("Sunrise Brightness v{}", config::VERSION))
        .with_icon(sun_icon())
        .build()?;

    // Nobody reads tray icon events, and the default sink is an unbounded
    // channel, so hovering the icon would grow it forever. Drop them instead.
    TrayIconEvent::set_event_handler(Some(|_| {}));

    let menu = Arc::new(ui::MenuFlags::default());
    let app = ui::SettingsApp::new(Arc::clone(&state), Arc::clone(&menu));

    let settings_id = item_settings.id().clone();
    let quit_id = item_quit.id().clone();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([480.0, 640.0])
            .with_decorations(cfg!(target_os = "linux"))
            .with_resizable(false)
            .with_maximized(false),
        ..Default::default()
    };

    eframe::run_native(
        "Sunrise Brightness",
        native_options,
        Box::new(move |cc| {
            // Waking the event loop on a menu click is what lets the app ask for
            // no periodic repaint at all while it sits hidden in the tray.
            let ctx = cc.egui_ctx.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if event.id == settings_id {
                    menu.open_settings.store(true, Ordering::Relaxed);
                } else if event.id == quit_id {
                    menu.quit.store(true, Ordering::Relaxed);
                }
                ctx.request_repaint();
            }));
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    Ok(())
}

fn sun_icon() -> tray_icon::Icon {
    let size: u32 = 32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let center = size as f32 / 2.0;
    let radius = center - 4.0;

    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - center + 0.5;
            let dy = y as f32 - center + 0.5;
            let dist = (dx * dx + dy * dy).sqrt();
            let i = ((y * size + x) * 4) as usize;

            if dist <= radius {
                let t = dist / radius;
                rgba[i] = 255;
                rgba[i + 1] = (210.0 + 45.0 * (1.0 - t)) as u8;
                rgba[i + 2] = (60.0 * (1.0 - t)) as u8;
                rgba[i + 3] = 255;
            }
        }
    }

    tray_icon::Icon::from_rgba(rgba, size, size).expect("failed to create tray icon")
}
