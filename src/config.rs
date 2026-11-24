use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scene {
    pub enabled: bool,
    #[serde(alias = "scene")]
    pub name: String,
    pub start: u32,
    pub end: u32,
    pub output: String,
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            enabled: true,
            name: String::new(),
            start: 1,
            end: 1,
            output: String::new(),
        }
    }
}

#[derive(Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Job {
    pub file: String,
    pub scenes_file: String,
    pub scenes: Vec<Scene>,
}

#[derive(Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub blender: String,
    pub jobs: Vec<Job>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct LegacyConfig {
    blender: String,
    file: String,
    jobs_file: String,
    jobs: Vec<Scene>,
}

impl From<LegacyConfig> for Config {
    fn from(c: LegacyConfig) -> Self {
        Config {
            blender: c.blender,
            jobs: vec![Job {
                file: c.file,
                scenes_file: c.jobs_file,
                scenes: c.jobs,
            }],
        }
    }
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
            .and_then(|s| Self::parse(&s))
            .unwrap_or_default()
    }

    fn parse(s: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(s).ok()?;
        if value.get("file").is_some() {
            serde_json::from_value::<LegacyConfig>(value)
                .ok()
                .map(Into::into)
        } else {
            serde_json::from_value(value).ok()
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        std::fs::write(config_path(), serde_json::to_string_pretty(self)?)
    }
}

impl Job {
    pub fn name(&self) -> String {
        Path::new(clean(&self.file))
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into())
    }

    pub fn enabled_scenes(&self) -> impl Iterator<Item = &Scene> {
        self.scenes.iter().filter(|s| s.enabled)
    }

    pub fn validate_paths(&self, blender: &str) -> Result<(), String> {
        if !Path::new(clean(blender)).is_file() {
            return Err(format!("Blender not found: {blender}"));
        }
        if !Path::new(clean(&self.file)).is_file() {
            return Err(format!(".blend file not found: {}", self.file));
        }
        Ok(())
    }

    pub fn has_new_blend(&self) -> bool {
        let file = clean(&self.file);
        file != self.scenes_file
            && file.to_lowercase().ends_with(".blend")
            && Path::new(file).is_file()
    }

    pub fn apply_scenes(&mut self, file: &str, scenes: Vec<Scene>) -> (bool, usize) {
        let file = clean(file);
        let replaced = !self.scenes_file.is_empty() && self.scenes_file != file;
        if replaced {
            self.scenes.clear();
        }
        self.scenes_file = file.to_string();
        (replaced, self.merge_scenes(scenes))
    }

    fn merge_scenes(&mut self, scenes: Vec<Scene>) -> usize {
        let before = self.scenes.len();
        for scene in scenes {
            match self.scenes.iter_mut().find(|s| s.name == scene.name) {
                Some(s) => (s.start, s.end) = (scene.start, scene.end),
                None => self.scenes.push(scene),
            }
        }
        self.scenes.len() - before
    }

    pub fn validate(&self, blender: &str) -> Result<(), String> {
        self.validate_paths(blender)?;
        if self.enabled_scenes().next().is_none() {
            return Err("No scenes enabled".into());
        }
        for scene in self.enabled_scenes() {
            if scene.name.trim().is_empty() {
                return Err("A scene row has no name".into());
            }
            if scene.start > scene.end {
                return Err(format!("{}: start frame is after end frame", scene.name));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(name: &str, end: u32) -> Scene {
        Scene {
            name: name.into(),
            end,
            ..Scene::default()
        }
    }

    fn job(scenes_file: &str, scenes: Vec<Scene>) -> Job {
        Job {
            scenes_file: scenes_file.into(),
            scenes,
            ..Job::default()
        }
    }

    #[test]
    fn same_file_refreshes_frames_keeps_settings_and_adds_new_scenes() {
        let mut j = job(
            "a.blend",
            vec![Scene {
                enabled: false,
                output: "D:\\out\\".into(),
                ..scene("BG", 99)
            }],
        );
        assert_eq!(
            j.apply_scenes("\"a.blend\"", vec![scene("BG", 1), scene("CH", 2)]),
            (false, 1)
        );
        assert_eq!(j.scenes[0].end, 1);
        assert!(!j.scenes[0].enabled);
        assert_eq!(j.scenes[0].output, "D:\\out\\");
        assert_eq!(j.scenes[1].name, "CH");
    }

    #[test]
    fn different_file_replaces_scenes() {
        let mut j = job("a.blend", vec![scene("Old", 5)]);
        assert_eq!(j.apply_scenes("b.blend", vec![scene("New", 1)]), (true, 1));
        assert_eq!(j.scenes.len(), 1);
        assert_eq!(j.scenes[0].name, "New");
        assert_eq!(j.scenes_file, "b.blend");
    }

    #[test]
    fn manual_scenes_without_source_file_are_merged() {
        let mut j = job("", vec![scene("BG", 99)]);
        assert_eq!(j.apply_scenes("a.blend", vec![scene("BG", 1)]), (false, 0));
        assert_eq!(j.scenes.len(), 1);
        assert_eq!(j.scenes[0].end, 1);
    }

    #[test]
    fn name_is_file_stem() {
        let j = Job {
            file: "\"D:/shots/shot_A.blend\"".into(),
            ..Job::default()
        };
        assert_eq!(j.name(), "shot_A");
        assert_eq!(Job::default().name(), "Untitled");
    }

    #[test]
    fn migrates_single_file_config() {
        let old = r#"{"blender":"b.exe","file":"a.blend","jobs_file":"a.blend",
            "jobs":[{"enabled":false,"scene":"BG","start":1,"end":9,"output":""}]}"#;
        let c = Config::parse(old).unwrap();
        assert_eq!(c.blender, "b.exe");
        assert_eq!(c.jobs.len(), 1);
        assert_eq!(c.jobs[0].file, "a.blend");
        assert_eq!(c.jobs[0].scenes_file, "a.blend");
        assert_eq!(c.jobs[0].scenes[0].name, "BG");
        assert!(!c.jobs[0].scenes[0].enabled);
    }

    #[test]
    fn current_config_round_trips() {
        let c = Config {
            blender: "b.exe".into(),
            jobs: vec![job("a.blend", vec![scene("BG", 3)]), Job::default()],
        };
        assert!(Config::parse(&serde_json::to_string(&c).unwrap()) == Some(c));
    }
}
