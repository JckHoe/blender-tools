use crate::config::{Job, Scene, clean};
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub enum Outcome {
    Done,
    Failed,
    Cancelled,
}

pub fn args(file: &str, scene: &Scene) -> Vec<String> {
    let mut args = vec![
        "-b".into(),
        clean(file).into(),
        "-S".into(),
        scene.name.trim().into(),
        "-s".into(),
        scene.start.to_string(),
        "-e".into(),
        scene.end.to_string(),
    ];
    let output = clean(&scene.output);
    if !output.is_empty() {
        args.extend(["-o".into(), output.into()]);
    }
    args.push("-a".into());
    args
}

pub fn blender_command(blender: &str) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut cmd = Command::new(clean(blender));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

pub struct Task {
    pub job: Job,
    pub cancel: Arc<AtomicBool>,
    pub log: Sender<String>,
    pub done: Sender<Outcome>,
}

pub fn spawn(blender: String, tasks: Vec<Task>) {
    thread::spawn(move || {
        for task in tasks {
            let outcome = render_job(&blender, &task.job, &task.cancel, &task.log);
            let _ = task.done.send(outcome);
        }
    });
}

fn render_job(blender: &str, job: &Job, cancel: &AtomicBool, log: &Sender<String>) -> Outcome {
    let mut failed = Vec::new();
    for scene in job.enabled_scenes() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let _ = log.send(format!(
            "==== {}  frames {}-{} ====",
            scene.name, scene.start, scene.end
        ));
        match run_scene(blender, &job.file, scene, cancel, log) {
            Ok(status) if status.success() => {}
            Ok(status) => {
                let _ = log.send(format!("[FAILED] {} ({status})", scene.name));
                failed.push(scene.name.clone());
            }
            Err(e) => {
                let _ = log.send(format!("[ERROR] {}: {e}", scene.name));
                failed.push(scene.name.clone());
            }
        }
    }
    let (summary, outcome) = if cancel.load(Ordering::Relaxed) {
        ("Cancelled.".to_string(), Outcome::Cancelled)
    } else if failed.is_empty() {
        ("All renders done.".to_string(), Outcome::Done)
    } else {
        (
            format!("Done with failures: {}", failed.join(", ")),
            Outcome::Failed,
        )
    };
    let _ = log.send(summary);
    outcome
}

fn run_scene(
    blender: &str,
    file: &str,
    scene: &Scene,
    cancel: &AtomicBool,
    log: &Sender<String>,
) -> std::io::Result<ExitStatus> {
    let mut child = blender_command(blender)
        .args(args(file, scene))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let readers = [
        forward(child.stdout.take(), log.clone()),
        forward(child.stderr.take(), log.clone()),
    ];
    let status = wait(&mut child, cancel)?;
    for reader in readers {
        let _ = reader.join();
    }
    Ok(status)
}

fn wait(child: &mut Child, cancel: &AtomicBool) -> std::io::Result<ExitStatus> {
    loop {
        if cancel.load(Ordering::Relaxed) {
            child.kill()?;
            return child.wait();
        }
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn forward(src: Option<impl Read + Send + 'static>, log: Sender<String>) -> JoinHandle<()> {
    thread::spawn(move || {
        if let Some(src) = src {
            for line in BufReader::new(src).lines().map_while(Result::ok) {
                let _ = log.send(line);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(output: &str) -> Scene {
        Scene {
            name: "BG_Cycles".into(),
            start: 1,
            end: 2,
            output: output.into(),
            ..Scene::default()
        }
    }

    #[test]
    fn builds_args_in_blender_order() {
        assert_eq!(
            args("\"D:\\a.blend\"", &scene("")),
            [
                "-b",
                "D:\\a.blend",
                "-S",
                "BG_Cycles",
                "-s",
                "1",
                "-e",
                "2",
                "-a"
            ]
        );
    }

    #[test]
    fn output_goes_before_animation_flag() {
        assert_eq!(
            args("a.blend", &scene("D:\\out\\")),
            [
                "-b",
                "a.blend",
                "-S",
                "BG_Cycles",
                "-s",
                "1",
                "-e",
                "2",
                "-o",
                "D:\\out\\",
                "-a"
            ]
        );
    }
}
