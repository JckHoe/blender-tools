use crate::render::blender_command;
use serde::de::DeserializeOwned;

pub fn run<T: DeserializeOwned>(
    blender: &str,
    args: &[&str],
    marker: &str,
    what: &str,
) -> Result<T, String> {
    let out = blender_command(blender)
        .args(["-b", "--factory-startup"])
        .args(args)
        .output()
        .map_err(|e| format!("Could not start Blender: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    parse(&stdout, marker, what).map_err(|e| {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let detail = if stderr.trim().is_empty() {
            &stdout
        } else {
            &stderr
        };
        format!("{e} ({})\n{}", out.status, tail(detail, 5))
    })
}

pub fn parse<T: DeserializeOwned>(stdout: &str, marker: &str, what: &str) -> Result<T, String> {
    let json = stdout
        .lines()
        .find_map(|line| line.strip_prefix(marker))
        .ok_or_else(|| format!("Blender did not return {what}"))?;
    serde_json::from_str(json).map_err(|e| e.to_string())
}

fn tail(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_when_marker_missing() {
        assert!(parse::<Vec<u32>>("Error: cannot read file\n", "X:", "x").is_err());
    }

    #[test]
    fn tail_keeps_last_non_empty_lines() {
        assert_eq!(tail("a\n\nb\nc\n\n", 2), "b\nc");
        assert_eq!(tail("only\n", 5), "only");
    }
}
