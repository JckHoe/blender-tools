use crate::probe;
use serde::Deserialize;

const MARKER: &str = "GPUS_JSON:";
const SCRIPT: &str = "\
import bpy, json
p = bpy.context.preferences.addons['cycles'].preferences
found = []
for t in ('OPTIX', 'CUDA', 'HIP', 'ONEAPI', 'METAL'):
    try:
        p.compute_device_type = t
    except TypeError:
        continue
    p.refresh_devices()
    found = [{'backend': t, 'id': d.id, 'name': d.name} for d in p.devices if d.type == t]
    if found:
        break
print('GPUS_JSON:' + json.dumps(found))
";

#[derive(Clone, Deserialize)]
pub struct Gpu {
    pub backend: String,
    pub id: String,
    pub name: String,
}

pub fn list(blender: &str) -> Result<Vec<Gpu>, String> {
    probe::run(blender, &["--python-expr", SCRIPT], MARKER, "a GPU list")
}

// Disables preference autosave first so pinning never changes the user's saved Cycles devices
pub fn pin_script(gpu: &Gpu) -> String {
    let quote = |s: &str| serde_json::to_string(s).unwrap_or_default();
    format!(
        "import bpy\n\
         bpy.context.preferences.use_preferences_save = False\n\
         p = bpy.context.preferences.addons['cycles'].preferences\n\
         p.compute_device_type = {}\n\
         p.refresh_devices()\n\
         for d in p.devices:\n    d.use = d.id == {}\n",
        quote(&gpu.backend),
        quote(&gpu.id),
    )
}

pub fn lanes(parallel: usize, gpus: &[Gpu]) -> Vec<Option<Gpu>> {
    let pin = parallel >= 2 && gpus.len() >= 2;
    (0..parallel.max(1))
        .map(|k| pin.then(|| gpus[k % gpus.len()].clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(id: &str) -> Gpu {
        Gpu {
            backend: "OPTIX".into(),
            id: id.into(),
            name: "RTX".into(),
        }
    }

    #[test]
    fn parses_gpu_line() {
        let out = "Blender 5.2\nGPUS_JSON:[{\"backend\": \"OPTIX\", \"id\": \"OPTIX_0\", \"name\": \"RTX\"}]\n";
        let gpus: Vec<Gpu> = probe::parse(out, MARKER, "x").unwrap();
        assert_eq!(gpus[0].id, "OPTIX_0");
        assert!(SCRIPT.contains(MARKER));
    }

    #[test]
    fn pin_script_quotes_backend_and_id() {
        let s = pin_script(&gpu("OPTIX_NVIDIA \"A\"_0000:01:00"));
        assert!(s.contains("p.compute_device_type = \"OPTIX\""));
        assert!(s.contains("d.use = d.id == \"OPTIX_NVIDIA \\\"A\\\"_0000:01:00\""));
    }

    #[test]
    fn lanes_round_robin_gpus() {
        let gpus = [gpu("a"), gpu("b")];
        let ids: Vec<_> = lanes(3, &gpus).into_iter().map(|g| g.unwrap().id).collect();
        assert_eq!(ids, ["a", "b", "a"]);
    }

    #[test]
    fn no_pinning_with_one_lane_or_one_gpu() {
        assert!(lanes(1, &[gpu("a"), gpu("b")]).iter().all(Option::is_none));
        assert_eq!(lanes(2, &[gpu("a")]).len(), 2);
        assert!(lanes(2, &[gpu("a")]).iter().all(Option::is_none));
        assert_eq!(lanes(0, &[]).len(), 1);
    }
}
