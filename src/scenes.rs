use crate::config::{Scene, clean};
use crate::render::blender_command;
use serde::Deserialize;

const MARKER: &str = "SCENES_JSON:";
const SCRIPT: &str = "import bpy, json; print('SCENES_JSON:' + json.dumps([{'name': s.name, 'start': s.frame_start, 'end': s.frame_end} for s in bpy.data.scenes]))";

#[derive(Deserialize)]
struct BlendScene {
    name: String,
    start: i32,
    end: i32,
}

impl From<BlendScene> for Scene {
    fn from(s: BlendScene) -> Self {
        Scene {
            name: s.name,
            start: s.start.max(0) as u32,
            end: s.end.max(0) as u32,
            ..Scene::default()
        }
    }
}

pub fn list(blender: &str, file: &str) -> Result<Vec<Scene>, String> {
    let out = blender_command(blender)
        .args([
            "-b",
            "--factory-startup",
            clean(file),
            "--python-expr",
            SCRIPT,
        ])
        .output()
        .map_err(|e| format!("Could not start Blender: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    parse(&stdout).map_err(|e| {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let detail = if stderr.trim().is_empty() {
            &stdout
        } else {
            &stderr
        };
        format!("{e} ({})\n{}", out.status, tail(detail, 5))
    })
}

fn tail(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

fn parse(stdout: &str) -> Result<Vec<Scene>, String> {
    let json = stdout
        .lines()
        .find_map(|line| line.strip_prefix(MARKER))
        .ok_or("Blender did not return a scene list")?;
    let scenes: Vec<BlendScene> = serde_json::from_str(json).map_err(|e| e.to_string())?;
    Ok(scenes.into_iter().map(Scene::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scene_line_from_blender_output() {
        let out = "Blender 5.2\nRead blend: x.blend\nSCENES_JSON:[{\"name\": \"BG_Cycles\", \"start\": 1, \"end\": 250}, {\"name\": \"CH\", \"start\": -5, \"end\": 10}]\nBlender quit\n";
        let scenes = parse(out).unwrap();
        assert_eq!(scenes.len(), 2);
        assert_eq!(
            (scenes[0].name.as_str(), scenes[0].start, scenes[0].end),
            ("BG_Cycles", 1, 250)
        );
        assert_eq!((scenes[1].start, scenes[1].end), (0, 10));
    }

    #[test]
    fn errors_when_marker_missing() {
        assert!(parse("Error: cannot read file\n").is_err());
    }

    #[test]
    fn tail_keeps_last_non_empty_lines() {
        assert_eq!(tail("a\n\nb\nc\n\n", 2), "b\nc");
        assert_eq!(tail("only\n", 5), "only");
    }

    #[test]
    fn script_prints_marker() {
        assert!(SCRIPT.contains(MARKER));
    }
}
