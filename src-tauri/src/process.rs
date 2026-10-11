use crate::store::{err, Result};
use std::{
    path::Path,
    process::Stdio,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};
#[derive(Debug)]
pub struct Outcome {
    pub code: Option<i32>,
    pub cancelled: bool,
}
pub async fn run<F: FnMut(&str)>(
    binary: &Path,
    args: &[String],
    cancel: Option<&AtomicBool>,
    timeout: Duration,
    mut line: F,
) -> Result<Outcome> {
    let mut child = Command::new(binary)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(err)?;
    let mut stdout = BufReader::new(child.stdout.take().ok_or("stdout_missing")?).lines();
    let mut stderr = BufReader::new(child.stderr.take().ok_or("stderr_missing")?).lines();
    let (mut out_open, mut err_open) = (true, true);
    let mut status = None;
    let start = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let mut cancelled = false;
    while out_open || err_open || status.is_none() {
        tokio::select! {
         s=stdout.next_line(), if out_open => { match s.map_err(err)? { Some(s)=>line(&s),None=>out_open=false } },
         s=stderr.next_line(), if err_open => { match s.map_err(err)? { Some(s)=>line(&s),None=>err_open=false } },
         _=tick.tick()=> {
          if status.is_none() {
           if cancel.is_some_and(|c|c.load(Ordering::Relaxed)) { cancelled=true; child.kill().await.map_err(err)?; status=Some(child.wait().await.map_err(err)?); }
           else if start.elapsed()>timeout { let _=child.kill().await; let _=child.wait().await; return Err("process_timeout".into()); }
           else { status=child.try_wait().map_err(err)?; }
          }
         }
        }
    }
    Ok(Outcome {
        code: status.and_then(|s| s.code()),
        cancelled,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn timeout_kills_process() {
        let result = run(
            Path::new("/bin/sleep"),
            &["10".into()],
            None,
            Duration::from_millis(200),
            |_| {},
        )
        .await;
        assert_eq!(result.unwrap_err(), "process_timeout");
    }
    #[tokio::test]
    async fn cancellation_is_not_success() {
        let cancel = AtomicBool::new(true);
        let result = run(
            Path::new("/bin/sleep"),
            &["10".into()],
            Some(&cancel),
            Duration::from_secs(5),
            |_| {},
        )
        .await
        .unwrap();
        assert!(result.cancelled);
        assert_ne!(result.code, Some(0));
    }
    #[tokio::test]
    #[ignore = "requires installed ClamAV; uses only harmless local custom signature"]
    async fn actual_clamav_streams_clean_and_detected_files() {
        let binary = crate::engine::discover().expect("ClamAV required");
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&root).unwrap();
        let db = root.join("test.ndb");
        std::fs::write(
            &db,
            format!(
                "ClamAV-Desktop-Test:0:*:{}\n",
                hex::encode(b"CLAMAV_DESKTOP_TEST_DETECTION_PAYLOAD")
            ),
        )
        .unwrap();
        let files = root.join("files");
        std::fs::create_dir(&files).unwrap();
        std::fs::write(files.join("clean: file.txt"), b"ordinary harmless document").unwrap();
        std::fs::write(
            files.join("detected.txt"),
            b"CLAMAV_DESKTOP_TEST_DETECTION_PAYLOAD",
        )
        .unwrap();
        let mut clean = 0;
        let mut found = 0;
        let result = run(
            &binary,
            &[
                "--recursive".into(),
                "--verbose".into(),
                "--stdout".into(),
                format!("--database={}", db.display()),
                "--".into(),
                files.to_string_lossy().into(),
            ],
            None,
            Duration::from_secs(30),
            |line| match crate::scanner::parse(line) {
                crate::scanner::Parsed::Clean(_) => clean += 1,
                crate::scanner::Parsed::Found(_, _) => found += 1,
                _ => {}
            },
        )
        .await
        .unwrap();
        assert_eq!(result.code, Some(1));
        assert_eq!(clean, 1);
        assert_eq!(found, 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
