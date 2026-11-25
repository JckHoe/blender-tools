use crate::config::{Scene, clean};
use crate::probe;
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
    let scenes: Vec<BlendScene> = probe::run(
        blender,
        &[clean(file), "--python-expr", SCRIPT],
        MARKER,
        "a scene list",
    )?;
    Ok(scenes.into_iter().map(Scene::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scene_line_from_blender_output() {
        let out = "Blender 5.2\nRead blend: x.blend\nSCENES_JSON:[{\"name\": \"BG_Cycles\", \"start\": 1, \"end\": 250}, {\"name\": \"CH\", \"start\": -5, \"end\": 10}]\nBlender quit\n";
        let scenes: Vec<Scene> = probe::parse::<Vec<BlendScene>>(out, MARKER, "x")
            .unwrap()
            .into_iter()
            .map(Scene::from)
            .collect();
        assert_eq!(scenes.len(), 2);
        assert_eq!(
            (scenes[0].name.as_str(), scenes[0].start, scenes[0].end),
            ("BG_Cycles", 1, 250)
        );
        assert_eq!((scenes[1].start, scenes[1].end), (0, 10));
    }

    #[test]
    fn script_prints_marker() {
        assert!(SCRIPT.contains(MARKER));
    }
}
