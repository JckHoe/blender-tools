use crate::config::{Config, Job, clean};
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub fn args(file: &str, job: &Job) -> Vec<String> {
    let mut args = vec![
        "-b".into(),
        clean(file).into(),
        "-S".into(),
        job.scene.trim().into(),
        "-s".into(),
        job.start.to_string(),
        "-e".into(),
        job.end.to_string(),
    ];
    let output = clean(&job.output);
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

pub fn spawn(config: Config, cancel: Arc<AtomicBool>, log: Sender<String>) {
    thread::spawn(move || {
        let mut failed = Vec::new();
        for job in config.enabled_jobs() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let _ = log.send(format!(
                "==== {}  frames {}-{} ====",
                job.scene, job.start, job.end
            ));
            match run_job(&config, job, &cancel, &log) {
                Ok(status) if status.success() => {}
                Ok(status) => {
                    let _ = log.send(format!("[FAILED] {} ({status})", job.scene));
                    failed.push(job.scene.clone());
                }
                Err(e) => {
                    let _ = log.send(format!("[ERROR] {}: {e}", job.scene));
                    failed.push(job.scene.clone());
                }
            }
        }
        let summary = if cancel.load(Ordering::Relaxed) {
            "Cancelled.".to_string()
        } else if failed.is_empty() {
            "All renders done.".to_string()
        } else {
            format!("Done with failures: {}", failed.join(", "))
        };
        let _ = log.send(summary);
    });
}

fn run_job(
    config: &Config,
    job: &Job,
    cancel: &AtomicBool,
    log: &Sender<String>,
) -> std::io::Result<ExitStatus> {
    let mut child = blender_command(&config.blender)
        .args(args(&config.file, job))
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

    fn job(output: &str) -> Job {
        Job {
            scene: "BG_Cycles".into(),
            start: 1,
            end: 2,
            output: output.into(),
            ..Job::default()
        }
    }

    #[test]
    fn builds_args_in_blender_order() {
        assert_eq!(
            args("\"D:\\a.blend\"", &job("")),
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
            args("a.blend", &job("D:\\out\\")),
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
