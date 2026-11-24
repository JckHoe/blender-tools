use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Job {
    pub enabled: bool,
    pub scene: String,
    pub start: u32,
    pub end: u32,
    pub output: String,
}

impl Default for Job {
    fn default() -> Self {
        Self {
            enabled: true,
            scene: String::new(),
            start: 1,
            end: 1,
            output: String::new(),
        }
    }
}

#[derive(Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub blender: String,
    pub file: String,
    pub jobs_file: String,
    pub jobs: Vec<Job>,
}

// Windows "Copy as path" wraps paths in quotes
pub fn clean(s: &str) -> &str {
    s.trim().trim_matches('"')
}

fn config_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("render-queue.json")))
        .unwrap_or_else(|| "render-queue.json".into())
}

impl Config {
    pub fn load() -> Self {
        std::fs::read_to_string(config_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        std::fs::write(config_path(), serde_json::to_string_pretty(self)?)
    }

    pub fn enabled_jobs(&self) -> impl Iterator<Item = &Job> {
        self.jobs.iter().filter(|j| j.enabled)
    }

    pub fn validate_paths(&self) -> Result<(), String> {
        if !Path::new(clean(&self.blender)).is_file() {
            return Err(format!("Blender not found: {}", self.blender));
        }
        if !Path::new(clean(&self.file)).is_file() {
            return Err(format!(".blend file not found: {}", self.file));
        }
        Ok(())
    }

    pub fn has_new_blend(&self) -> bool {
        let file = clean(&self.file);
        file != self.jobs_file
            && file.to_lowercase().ends_with(".blend")
            && Path::new(file).is_file()
    }

    pub fn apply_scenes(&mut self, file: &str, jobs: Vec<Job>) -> (bool, usize) {
        let file = clean(file);
        let replaced = !self.jobs_file.is_empty() && self.jobs_file != file;
        if replaced {
            self.jobs.clear();
        }
        self.jobs_file = file.to_string();
        (replaced, self.merge_jobs(jobs))
    }

    fn merge_jobs(&mut self, jobs: Vec<Job>) -> usize {
        let before = self.jobs.len();
        for job in jobs {
            match self.jobs.iter_mut().find(|j| j.scene == job.scene) {
                Some(j) => (j.start, j.end) = (job.start, job.end),
                None => self.jobs.push(job),
            }
        }
        self.jobs.len() - before
    }

    pub fn validate(&self) -> Result<(), String> {
        self.validate_paths()?;
        if self.enabled_jobs().next().is_none() {
            return Err("No jobs enabled".into());
        }
        for job in self.enabled_jobs() {
            if job.scene.trim().is_empty() {
                return Err("A job has no scene name".into());
            }
            if job.start > job.end {
                return Err(format!("{}: start frame is after end frame", job.scene));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(scene: &str, end: u32) -> Job {
        Job {
            scene: scene.into(),
            end,
            ..Job::default()
        }
    }

    fn config(jobs_file: &str, jobs: Vec<Job>) -> Config {
        Config {
            jobs_file: jobs_file.into(),
            jobs,
            ..Config::default()
        }
    }

    #[test]
    fn same_file_refreshes_frames_keeps_settings_and_adds_new_scenes() {
        let mut c = config(
            "a.blend",
            vec![Job {
                enabled: false,
                output: "D:\\out\\".into(),
                ..job("BG", 99)
            }],
        );
        assert_eq!(
            c.apply_scenes("\"a.blend\"", vec![job("BG", 1), job("CH", 2)]),
            (false, 1)
        );
        assert_eq!(c.jobs[0].end, 1);
        assert!(!c.jobs[0].enabled);
        assert_eq!(c.jobs[0].output, "D:\\out\\");
        assert_eq!(c.jobs[1].scene, "CH");
    }

    #[test]
    fn different_file_replaces_jobs() {
        let mut c = config("a.blend", vec![job("Old", 5)]);
        assert_eq!(c.apply_scenes("b.blend", vec![job("New", 1)]), (true, 1));
        assert_eq!(c.jobs.len(), 1);
        assert_eq!(c.jobs[0].scene, "New");
        assert_eq!(c.jobs_file, "b.blend");
    }

    #[test]
    fn manual_jobs_without_source_file_are_merged() {
        let mut c = config("", vec![job("BG", 99)]);
        assert_eq!(c.apply_scenes("a.blend", vec![job("BG", 1)]), (false, 0));
        assert_eq!(c.jobs.len(), 1);
        assert_eq!(c.jobs[0].end, 1);
    }
}
