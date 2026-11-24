use crate::config::{Config, Job};
use crate::render;
use eframe::egui;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

const MAX_LOG_BYTES: usize = 500_000;

struct Run {
    log: Receiver<String>,
    cancel: Arc<AtomicBool>,
}

pub struct App {
    config: Config,
    saved: Config,
    log: String,
    run: Option<Run>,
}

impl App {
    pub fn new() -> Self {
        let config = Config::load();
        Self {
            saved: config.clone(),
            config,
            log: String::new(),
            run: None,
        }
    }

    fn start(&mut self) {
        self.log.clear();
        if let Err(e) = self.config.validate() {
            self.log.push_str(&format!("[ERROR] {e}\n"));
            return;
        }
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        render::spawn(self.config.clone(), cancel.clone(), tx);
        self.run = Some(Run { log: rx, cancel });
    }

    fn poll(&mut self) {
        let Some(run) = &self.run else { return };
        let finished = loop {
            match run.log.try_recv() {
                Ok(line) => {
                    self.log.push_str(&line);
                    self.log.push('\n');
                }
                Err(TryRecvError::Empty) => break false,
                Err(TryRecvError::Disconnected) => break true,
            }
        };
        if finished {
            self.run = None;
        }
        if self.log.len() > MAX_LOG_BYTES {
            let cut = self.log.len() - MAX_LOG_BYTES;
            let cut = self.log[cut..].find('\n').map_or(cut, |i| cut + i + 1);
            self.log.drain(..cut);
        }
    }

    fn autosave(&mut self) {
        if self.config == self.saved {
            return;
        }
        if let Err(e) = self.config.save() {
            self.log
                .push_str(&format!("[ERROR] Could not save settings: {e}\n"));
        }
        self.saved = self.config.clone();
    }

    fn setup_ui(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("paths").num_columns(3).show(ui, |ui| {
            ui.label("Blender exe");
            path_field(ui, &mut self.config.blender);
            if ui.button("Browse…").clicked()
                && let Some(p) = pick_file("Blender", "exe")
            {
                self.config.blender = p;
            }
            ui.end_row();

            ui.label(".blend file");
            path_field(ui, &mut self.config.file);
            if ui.button("Browse…").clicked()
                && let Some(p) = pick_file("Blend file", "blend")
            {
                self.config.file = p;
            }
            ui.end_row();
        });

        ui.separator();

        let mut remove = None;
        egui::Grid::new("jobs")
            .num_columns(6)
            .striped(true)
            .show(ui, |ui| {
                for header in [
                    "On",
                    "Scene",
                    "Start",
                    "End",
                    "Output folder (optional)",
                    "",
                ] {
                    ui.strong(header);
                }
                ui.end_row();

                for (i, job) in self.config.jobs.iter_mut().enumerate() {
                    ui.checkbox(&mut job.enabled, "");
                    ui.text_edit_singleline(&mut job.scene);
                    ui.add(egui::DragValue::new(&mut job.start));
                    ui.add(egui::DragValue::new(&mut job.end));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut job.output).desired_width(260.0));
                        if ui.button("…").clicked()
                            && let Some(dir) = rfd::FileDialog::new().pick_folder()
                        {
                            job.output = format!("{}{}", dir.display(), std::path::MAIN_SEPARATOR);
                        }
                    });
                    if ui.button("✖").on_hover_text("Remove job").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(i) = remove {
            self.config.jobs.remove(i);
        }
        if ui.button("➕ Add job").clicked() {
            self.config.jobs.push(Job::default());
        }
    }
}

fn path_field(ui: &mut egui::Ui, value: &mut String) {
    ui.add(egui::TextEdit::singleline(value).desired_width(520.0));
}

fn pick_file(name: &str, ext: &str) -> Option<String> {
    rfd::FileDialog::new()
        .add_filter(name, &[ext])
        .pick_file()
        .map(|p| p.display().to_string())
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        let running = self.run.is_some();
        if running {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        egui::TopBottomPanel::top("setup").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.add_enabled_ui(!running, |ui| self.setup_ui(ui));
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!running, egui::Button::new("▶ Render"))
                    .clicked()
                {
                    self.start();
                }
                if ui
                    .add_enabled(running, egui::Button::new("⏹ Stop"))
                    .clicked()
                    && let Some(run) = &self.run
                {
                    run.cancel.store(true, Ordering::Relaxed);
                }
                if running {
                    ui.spinner();
                    ui.label("Rendering…");
                }
            });
            ui.add_space(6.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .auto_shrink(false)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.log.as_str())
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY),
                    );
                });
        });

        self.autosave();
    }
}
