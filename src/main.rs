#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod render;
mod scenes;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Blender Render Queue")
            .with_inner_size([900.0, 650.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Blender Render Queue",
        options,
        Box::new(|_| Ok(Box::new(app::App::new()))),
    )
}
