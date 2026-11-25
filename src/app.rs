use crate::config::{Config, Job, Scene};
use crate::gpus::{self, Gpu};
use crate::render::{Outcome, Task};
use crate::{render, scenes};
use eframe::egui;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

const MAX_LOG_BYTES: usize = 500_000;

struct Run {
    log: Receiver<String>,
    cancel: Arc<AtomicBool>,
    done: Receiver<Outcome>,
    started: bool,
}

struct Scan {
    file: String,
    result: Receiver<Result<Vec<Scene>, String>>,
}

struct GpuScan {
    blender: String,
    rx: Receiver<Result<Vec<Gpu>, String>>,
    result: Option<Result<Vec<Gpu>, String>>,
}

struct Picker {
    picks: Vec<bool>,
    parallel: usize,
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
        if let Some(run) = &self.run {
            return match (run.cancel.load(Ordering::Relaxed), run.started) {
                (true, _) => ("⏹", "stopping"),
                (false, false) => ("⏳", "queued"),
                (false, true) => ("●", "rendering"),
            };
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
        if let Some(task) = self.prepare(blender, job) {
            render::spawn(blender.to_string(), vec![task], vec![None]);
        }
    }

    fn prepare(&mut self, blender: &str, job: &Job) -> Option<Task> {
        self.log.clear();
        self.outcome = None;
        if let Err(e) = job.validate(blender) {
            self.push(&format!("[ERROR] {e}"));
            return None;
        }
        let (log, log_rx) = mpsc::channel();
        let (done, done_rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.run = Some(Run {
            log: log_rx,
            cancel: cancel.clone(),
            done: done_rx,
            started: false,
        });
        Some(Task {
            job: job.clone(),
            cancel,
            log,
            done,
        })
    }

    fn stop(&mut self) {
        let Some(run) = &self.run else { return };
        run.cancel.store(true, Ordering::Relaxed);
        if !run.started {
            self.run = None;
            self.outcome = Some(Outcome::Cancelled);
            self.push("Cancelled.");
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
                    run.started = true;
                    self.log.push_str(&line);
                    self.log.push('\n');
                }
                Err(TryRecvError::Empty) => break false,
                Err(TryRecvError::Disconnected) => break true,
            }
        };
        if finished {
            self.outcome = run.done.try_recv().ok();
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
    picker: Option<Picker>,
    gpus: Option<GpuScan>,
}

impl App {
    pub fn new() -> Self {
        let config = Config::load();
        Self {
            states: config.jobs.iter().map(|_| JobState::default()).collect(),
            saved: config.clone(),
            config,
            selected: 0,
            picker: None,
            gpus: None,
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

    fn problem(&self, i: usize) -> Option<String> {
        let state = &self.states[i];
        if state.busy() {
            return Some(state.status().1.into());
        }
        self.config.jobs[i].validate(&self.config.blender).err()
    }

    fn open_picker(&mut self) {
        let picks = (0..self.config.jobs.len())
            .map(|i| {
                self.problem(i).is_none() && !matches!(self.states[i].outcome, Some(Outcome::Done))
            })
            .collect();
        self.picker = Some(Picker { picks, parallel: 1 });
        self.detect_gpus();
    }

    fn detect_gpus(&mut self) {
        let blender = self.config.blender.clone();
        let cached = |g: &GpuScan| g.blender == blender && !matches!(g.result, Some(Err(_)));
        if self.gpus.as_ref().is_some_and(cached) {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let probe = blender.clone();
        std::thread::spawn(move || {
            let _ = tx.send(gpus::list(&probe));
        });
        self.gpus = Some(GpuScan {
            blender,
            rx,
            result: None,
        });
    }

    fn detected_gpus(&self) -> Option<Result<&[Gpu], &str>> {
        match self.gpus.as_ref()?.result.as_ref()? {
            Ok(gpus) => Some(Ok(gpus)),
            Err(e) => Some(Err(e)),
        }
    }

    fn render_all(&mut self, picks: &[bool], parallel: usize) {
        let tasks = (self.states.iter_mut().zip(&self.config.jobs).zip(picks))
            .filter(|((state, _), on)| **on && !state.busy())
            .filter_map(|((state, job), _)| state.prepare(&self.config.blender, job))
            .collect::<Vec<_>>();
        if tasks.is_empty() {
            return;
        }
        let gpus = match self.detected_gpus() {
            Some(Ok(gpus)) => gpus,
            _ => &[],
        };
        let lanes = gpus::lanes(parallel.min(tasks.len()), gpus);
        render::spawn(self.config.blender.clone(), tasks, lanes);
    }

    fn stop_all(&mut self) {
        self.states.iter_mut().for_each(JobState::stop);
    }

    fn poll(&mut self) {
        for (state, job) in self.states.iter_mut().zip(&mut self.config.jobs) {
            state.poll_run();
            state.poll_scan(job);
        }
        if let Some(scan) = &mut self.gpus
            && scan.result.is_none()
        {
            scan.result = match scan.rx.try_recv() {
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err("GPU detection stopped".into())),
                Ok(result) => Some(result),
            };
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
        ui.horizontal(|ui| {
            let idle = self.states.iter().any(|s| !s.busy());
            if ui
                .add_enabled(idle, egui::Button::new("▶ Render all"))
                .on_hover_text("Pick jobs to render one after another")
                .clicked()
            {
                self.open_picker();
            }
            let running = self.states.iter().any(|s| s.run.is_some());
            if ui
                .add_enabled(running, egui::Button::new("⏹ Stop all"))
                .clicked()
            {
                self.stop_all();
            }
        });
        ui.separator();
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
        let (Some(job), Some(state)) = (self.config.jobs.get_mut(i), self.states.get_mut(i)) else {
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

    fn picker_ui(&mut self, ctx: &egui::Context) {
        let Some(Picker {
            mut picks,
            mut parallel,
        }) = self.picker.take()
        else {
            return;
        };
        let problems: Vec<_> = (0..picks.len()).map(|i| self.problem(i)).collect();
        for (on, problem) in picks.iter_mut().zip(&problems) {
            *on &= problem.is_none();
        }
        let chosen = || self.config.jobs.iter().zip(&picks).filter(|(_, on)| **on);
        let count = chosen().count();
        let scenes: usize = chosen().map(|(j, _)| j.enabled_scenes().count()).sum();
        let frames: u32 = chosen().map(|(j, _)| j.frames()).sum();
        let detected = self.detected_gpus();
        let mut at_once = parallel.clamp(1, count.max(1));
        let gpu_line = match detected {
            None => "GPUs: detecting…".to_string(),
            Some(Err(e)) => format!("GPU detection failed: {}", e.lines().next().unwrap_or("")),
            Some(Ok([])) => "GPUs: none found by Cycles".to_string(),
            Some(Ok(g)) => format!(
                "GPUs ({}): {}",
                g[0].backend,
                g.iter()
                    .map(|g| g.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        let gpu_count = match detected {
            Some(Ok(g)) => g.len(),
            _ => 0,
        };
        let note = match (gpu_count >= 2, at_once) {
            (true, 1) => format!("Set Max at once to {gpu_count} to run one job per GPU."),
            (true, _) => "Cycles jobs are spread across GPUs. Eevee uses the display GPU.".into(),
            (false, 1) => String::new(),
            (false, _) => {
                "Parallel jobs share the same CPU/GPU and memory; heavy scenes may fail.".into()
            }
        };

        let mut start = false;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("render_all")).show(ctx, |ui| {
            ui.heading("Render all");
            ui.label(if at_once == 1 {
                "Selected jobs render one after another.".to_string()
            } else {
                format!("Selected jobs render {at_once} at a time.")
            });
            ui.separator();
            egui::Grid::new("render_all_jobs")
                .num_columns(5)
                .striped(true)
                .show(ui, |ui| {
                    for header in ["", "Job", "Scenes", "Frames", ""] {
                        ui.strong(header);
                    }
                    ui.end_row();
                    for (i, job) in self.config.jobs.iter().enumerate() {
                        let note = match (&problems[i], &self.states[i].outcome) {
                            (Some(p), _) => p.as_str(),
                            (None, Some(Outcome::Done)) => "already done",
                            _ => "",
                        };
                        ui.add_enabled(
                            problems[i].is_none(),
                            egui::Checkbox::without_text(&mut picks[i]),
                        );
                        ui.label(job.name());
                        ui.label(job.enabled_scenes().count().to_string());
                        ui.label(job.frames().to_string());
                        ui.weak(note);
                        ui.end_row();
                    }
                });
            ui.separator();
            ui.label(format!("{count} jobs · {scenes} scenes · {frames} frames"));
            ui.separator();
            ui.label(gpu_line);
            ui.horizontal(|ui| {
                ui.label("Max at once");
                if ui
                    .add(egui::DragValue::new(&mut at_once).range(1..=count.max(1)))
                    .changed()
                {
                    parallel = at_once;
                }
            });
            ui.weak(note);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                start = ui
                    .add_enabled(
                        count > 0 && (at_once == 1 || detected.is_some()),
                        egui::Button::new(format!("▶ Render {count} jobs")),
                    )
                    .clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });

        if start {
            self.render_all(&picks, at_once);
        } else if !cancel && !modal.should_close() {
            self.picker = Some(Picker { picks, parallel });
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
        let detecting = self.gpus.as_ref().is_some_and(|g| g.result.is_none());
        if self.states.iter().any(JobState::busy) || detecting {
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
        self.picker_ui(ctx);

        self.autosave();
    }
}
