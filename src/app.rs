use crate::config::{Config, Job, Scene};
use crate::render::Outcome;
use crate::{render, scenes};
use eframe::egui;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::JoinHandle;
use std::time::Duration;

const MAX_LOG_BYTES: usize = 500_000;

struct Run {
    log: Receiver<String>,
    cancel: Arc<AtomicBool>,
    handle: Option<JoinHandle<Outcome>>,
}

struct Scan {
    file: String,
    result: Receiver<Result<Vec<Scene>, String>>,
}

#[derive(Default)]
struct JobState {
    log: String,
    run: Option<Run>,
    scan: Option<Scan>,
    outcome: Option<Outcome>,
}

impl JobState {
    fn busy(&self) -> bool {
        self.run.is_some() || self.scan.is_some()
    }

    fn status(&self) -> (&'static str, &'static str) {
        if self.run.is_some() {
            return ("●", "rendering");
        }
        if self.scan.is_some() {
            return ("…", "reading scenes");
        }
        match self.outcome {
            None => ("○", "idle"),
            Some(Outcome::Done) => ("✔", "done"),
            Some(Outcome::Failed) => ("✖", "failed"),
            Some(Outcome::Cancelled) => ("⏹", "cancelled"),
        }
    }

    fn push(&mut self, line: &str) {
        self.log.push_str(line);
        self.log.push('\n');
    }

    fn start(&mut self, blender: &str, job: &Job) {
        self.log.clear();
        self.outcome = None;
        if let Err(e) = job.validate(blender) {
            self.push(&format!("[ERROR] {e}"));
            return;
        }
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let handle = render::spawn(blender.to_string(), job.clone(), cancel.clone(), tx);
        self.run = Some(Run {
            log: rx,
            cancel,
            handle: Some(handle),
        });
    }

    fn stop(&self) {
        if let Some(run) = &self.run {
            run.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn load_scenes(&mut self, blender: &str, job: &Job) {
        if let Err(e) = job.validate_paths(blender) {
            self.push(&format!("[ERROR] {e}"));
            return;
        }
        self.push("Reading scenes from .blend…");
        let (tx, rx) = mpsc::channel();
        let (blender, file) = (blender.to_string(), job.file.clone());
        self.scan = Some(Scan {
            file: file.clone(),
            result: rx,
        });
        std::thread::spawn(move || {
            let _ = tx.send(scenes::list(&blender, &file));
        });
    }

    fn poll_scan(&mut self, job: &mut Job) {
        let Some(scan) = &self.scan else { return };
        let msg = match scan.result.try_recv() {
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => "[ERROR] Scene reader stopped".to_string(),
            Ok(Err(e)) => format!("[ERROR] {e}"),
            Ok(Ok(scenes)) => {
                let found = scenes.len();
                match job.apply_scenes(&scan.file, scenes) {
                    (true, _) => format!("New .blend: scene list replaced with {found} scenes"),
                    (false, added) => {
                        format!("Found {found} scenes, added {added} new, frame ranges refreshed")
                    }
                }
            }
        };
        self.push(&msg);
        self.scan = None;
    }

    fn poll_run(&mut self) {
        let Some(run) = &mut self.run else { return };
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
            self.outcome = run.handle.take().and_then(|h| h.join().ok());
            self.run = None;
        }
        if self.log.len() > MAX_LOG_BYTES {
            let cut = self.log.len() - MAX_LOG_BYTES;
            let cut = self.log[cut..].find('\n').map_or(cut, |i| cut + i + 1);
            self.log.drain(..cut);
        }
    }
}

pub struct App {
    config: Config,
    saved: Config,
    states: Vec<JobState>,
    selected: usize,
}

impl App {
    pub fn new() -> Self {
        let config = Config::load();
        Self {
            states: config.jobs.iter().map(|_| JobState::default()).collect(),
            saved: config.clone(),
            config,
            selected: 0,
        }
    }

    fn add_job(&mut self) {
        self.config.jobs.push(Job::default());
        self.states.push(JobState::default());
        self.selected = self.config.jobs.len() - 1;
    }

    fn remove_job(&mut self, i: usize) {
        self.config.jobs.remove(i);
        self.states.remove(i);
        self.selected = self.selected.min(self.config.jobs.len().saturating_sub(1));
    }

    fn poll(&mut self) {
        for (state, job) in self.states.iter_mut().zip(&mut self.config.jobs) {
            state.poll_run();
            state.poll_scan(job);
        }
    }

    fn autosave(&mut self) {
        if self.config == self.saved {
            return;
        }
        if let Err(e) = self.config.save()
            && let Some(state) = self.states.get_mut(self.selected)
        {
            state.push(&format!("[ERROR] Could not save settings: {e}"));
        }
        self.saved = self.config.clone();
    }

    fn blender_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Blender exe");
            path_field(ui, &mut self.config.blender);
            if ui.button("Browse…").clicked()
                && let Some(p) = pick_file("Blender", "exe")
            {
                self.config.blender = p;
            }
        });
    }

    fn jobs_ui(&mut self, ui: &mut egui::Ui) {
        for (i, (job, state)) in self.config.jobs.iter().zip(&self.states).enumerate() {
            let (icon, status) = state.status();
            if ui
                .selectable_label(self.selected == i, format!("{icon} {}", job.name()))
                .on_hover_text(status)
                .clicked()
            {
                self.selected = i;
            }
        }
        ui.add_space(6.0);
        if ui.button("➕ New job").clicked() {
            self.add_job();
        }
    }

    fn job_ui(&mut self, ui: &mut egui::Ui) {
        let i = self.selected;
        let blender = self.config.blender.clone();
        let (Some(job), Some(state)) = (self.config.jobs.get_mut(i), self.states.get_mut(i))
        else {
            ui.label("Add a job with ➕ New job.");
            return;
        };
        let busy = state.busy();

        ui.add_enabled_ui(!busy, |ui| setup_ui(ui, &blender, job, state));
        ui.separator();
        let mut remove = false;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("▶ Render"))
                .clicked()
            {
                state.start(&blender, job);
            }
            if ui
                .add_enabled(state.run.is_some(), egui::Button::new("⏹ Stop"))
                .clicked()
            {
                state.stop();
            }
            if state.busy() {
                ui.spinner();
                ui.label(state.status().1);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                remove = ui
                    .add_enabled(!state.busy(), egui::Button::new("🗑 Remove job"))
                    .clicked();
            });
        });
        ui.separator();

        egui::ScrollArea::vertical()
            .id_salt(i)
            .stick_to_bottom(true)
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut state.log.as_str())
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY),
                );
            });

        if remove {
            self.remove_job(i);
        }
    }
}

fn setup_ui(ui: &mut egui::Ui, blender: &str, job: &mut Job, state: &mut JobState) {
    ui.horizontal(|ui| {
        ui.label(".blend file");
        if path_field(ui, &mut job.file).changed() && job.has_new_blend() {
            state.load_scenes(blender, job);
        }
        if ui.button("Browse…").clicked()
            && let Some(p) = pick_file("Blend file", "blend")
        {
            job.file = p;
            state.load_scenes(blender, job);
        }
        if ui
            .button("Load scenes")
            .on_hover_text("Add missing scenes and refresh frame ranges from the .blend")
            .clicked()
        {
            state.load_scenes(blender, job);
        }
    });

    ui.separator();

    let mut remove = None;
    egui::Grid::new("scenes")
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

            for (i, scene) in job.scenes.iter_mut().enumerate() {
                ui.checkbox(&mut scene.enabled, "");
                ui.text_edit_singleline(&mut scene.name);
                ui.add(egui::DragValue::new(&mut scene.start));
                ui.add(egui::DragValue::new(&mut scene.end));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut scene.output).desired_width(260.0));
                    if ui.button("…").clicked()
                        && let Some(dir) = rfd::FileDialog::new().pick_folder()
                    {
                        scene.output = format!("{}{}", dir.display(), std::path::MAIN_SEPARATOR);
                    }
                });
                if ui.button("✖").on_hover_text("Remove scene").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = remove {
        job.scenes.remove(i);
    }
    if ui.button("➕ Add scene").clicked() {
        job.scenes.push(Scene::default());
    }
}

fn path_field(ui: &mut egui::Ui, value: &mut String) -> egui::Response {
    ui.add(egui::TextEdit::singleline(value).desired_width(520.0))
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
        if self.states.iter().any(JobState::busy) {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        egui::TopBottomPanel::top("blender").show(ctx, |ui| {
            ui.add_space(6.0);
            self.blender_ui(ui);
            ui.add_space(6.0);
        });

        egui::SidePanel::left("jobs")
            .resizable(true)
            .default_width(180.0)
            .show(ctx, |ui| {
                ui.add_space(6.0);
                ui.strong("Jobs");
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| self.jobs_ui(ui));
            });

        egui::CentralPanel::default().show(ctx, |ui| self.job_ui(ui));

        self.autosave();
    }
}
