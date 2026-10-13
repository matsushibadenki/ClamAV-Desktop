use crate::{engine, model::*, process, store::*};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::File,
    io::Read,
    path::Path,
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
pub fn digest(path: &Path) -> Result<String> {
    let mut f = File::open(path).map_err(err)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf).map_err(err)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}
#[derive(Debug, PartialEq)]
pub enum Parsed<'a> {
    Clean(&'a str),
    Found(&'a str, &'a str),
    Error(&'a str),
    Other,
}
pub fn parse(line: &str) -> Parsed<'_> {
    if let Some(p) = line.strip_suffix(": OK") {
        return Parsed::Clean(p);
    }
    if let Some(p) = line.strip_suffix(" FOUND") {
        if let Some((path, sig)) = p.rsplit_once(": ") {
            return Parsed::Found(path, sig);
        }
    }
    if line.ends_with(" ERROR") || line.starts_with("ERROR:") {
        return Parsed::Error(line);
    }
    Parsed::Other
}
pub async fn start(c: Arc<Core>, kind: String, target: Option<String>) -> Result<String> {
    let binary = engine::discover().ok_or("engine_missing")?;
    let home = std::env::var("HOME").map_err(err)?;
    let target = match kind.as_str() {
        "quick" => format!("{home}/Downloads"),
        "full" => home,
        "custom" => target.ok_or("target_missing")?,
        _ => return Err("invalid_scan_kind".into()),
    };
    let input = Path::new(&target);
    if std::fs::symlink_metadata(input)
        .map_err(err)?
        .file_type()
        .is_symlink()
    {
        return Err("symlink_not_allowed".into());
    }
    let path = std::fs::canonicalize(input).map_err(err)?;
    if path.starts_with(&c.root) || c.root.starts_with(&path) && kind == "custom" {
        return Err("app_data_target".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let scan = Scan {
        id: id.clone(),
        kind,
        target: path.to_string_lossy().into(),
        started_at: now(),
        finished_at: None,
        status: "running".into(),
        scanned: 0,
        errors: 0,
        current_file: String::new(),
        findings: vec![],
        message: None,
    };
    let max;
    {
        let mut d = c.document.lock().unwrap();
        if d.operation.is_some() || d.scan.as_ref().is_some_and(|s| s.status == "running") {
            return Err("busy".into());
        }
        if !d.engine.definitions || d.engine.version.is_none() {
            return Err("engine_not_ready".into());
        }
        max = d.settings.max_file_mb;
        c.save_scan(&scan)?;
        d.scan = Some(scan.clone());
        c.cancel.store(false, Ordering::Relaxed);
    }
    c.log("scan_started", scan.target.clone());
    tokio::spawn(async move {
        let mut scan = scan;
        let mut last = Instant::now();
        let mut file_errors = HashSet::new();
        let mut engine_errors = HashSet::new();
        let args = vec![
            "--recursive".into(),
            "--verbose".into(),
            "--stdout".into(),
            "--follow-dir-symlinks=0".into(),
            "--follow-file-symlinks=0".into(),
            format!(
                "--database={}",
                c.snapshot().engine.database_path.as_deref().unwrap_or("")
            ),
            format!("--max-filesize={max}M"),
            format!("--max-scansize={}M", max.saturating_mul(4)),
            "--alert-exceeds-max=yes".into(),
            format!("--exclude-dir={}", regex_escape(&c.root.to_string_lossy())),
            "--".into(),
            scan.target.clone(),
        ];
        let result = process::run(
            &binary,
            &args,
            Some(&c.cancel),
            Duration::from_secs(86400),
            |line| {
                match parse(line) {
                    Parsed::Clean(p) => {
                        scan.scanned += 1;
                        scan.current_file = p.into()
                    }
                    Parsed::Found(p, sig) => {
                        scan.current_file = p.into();
                        if sig.starts_with("Heuristics.Limits.Exceeded") {
                            file_errors.insert(p.to_string());
                        } else {
                            scan.scanned += 1;
                            let p = Path::new(p);
                            if !p.starts_with(Path::new(&scan.target)) {
                                engine_errors.insert("unexpected_result_path".to_string());
                                scan.errors = (file_errors.len() + engine_errors.len()) as u64;
                                return;
                            }
                            let hash = std::fs::symlink_metadata(p)
                                .ok()
                                .filter(|m| m.is_file() && !m.file_type().is_symlink())
                                .and_then(|_| digest(p).ok());
                            scan.findings.push(Finding {
                                id: uuid::Uuid::new_v4().to_string(),
                                path: p.to_string_lossy().into(),
                                signature: sig.into(),
                                digest: hash,
                                quarantined: false,
                            });
                        }
                    }
                    Parsed::Error(e) => {
                        track_error(e, &scan.target, &mut file_errors, &mut engine_errors);
                        scan.message = Some(e.chars().take(2048).collect());
                    }
                    Parsed::Other => {
                        if let Some(n) = line
                            .strip_prefix("Scanned files: ")
                            .and_then(|s| s.trim().parse().ok())
                        {
                            scan.scanned = n;
                        }
                    }
                }
                scan.errors = (file_errors.len() + engine_errors.len()) as u64;
                if last.elapsed() > Duration::from_millis(200) {
                    last = Instant::now();
                    c.document.lock().unwrap().scan = Some(scan.clone());
                    c.publish();
                }
            },
        )
        .await;
        scan.finished_at = Some(now());
        match result {
            Ok(o) if o.cancelled => scan.status = "cancelled".into(),
            Ok(o) if o.code == Some(0) || o.code == Some(1) => {
                scan.status = if scan.errors > 0 {
                    "incomplete"
                } else {
                    "completed"
                }
                .into();
            }
            Ok(o) if o.code == Some(2) && !file_errors.is_empty() && engine_errors.is_empty() => {
                scan.status = "incomplete".into();
            }
            Ok(o) => {
                scan.status = "failed".into();
                scan.message = Some(format!("clamscan_exit: {:?}", o.code));
            }
            Err(e) => {
                scan.status = "failed".into();
                scan.message = Some(e);
            }
        }
        if let Err(e) = c.save_scan(&scan) {
            scan.status = "failed".into();
            scan.message = Some(format!("history_save_failed: {e}"));
        }
        {
            let mut d = c.document.lock().unwrap();
            d.history.insert(0, scan.clone());
            d.scan = Some(scan);
        }
        c.log("scan_finished", "");
    });
    Ok(id)
}
fn track_error(e: &str, target: &str, files: &mut HashSet<String>, engine: &mut HashSet<String>) {
    if let Some((path, _)) = e.rsplit_once(": ") {
        if Path::new(path).starts_with(Path::new(target)) {
            files.insert(path.to_string());
            return;
        }
    }
    engine.insert(e.to_string());
}
fn regex_escape(s: &str) -> String {
    let mut out = String::from("^");
    for ch in s.chars() {
        if ".+*?()|[]{}^$\\".contains(ch) {
            out.push('\\')
        }
        out.push(ch)
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_limit_and_file_error_once() {
        let mut files = HashSet::from(["/tmp/scan/large.txt".to_string()]);
        let mut engine = HashSet::new();
        track_error(
            "/tmp/scan/large.txt: Virus(es) detected ERROR",
            "/tmp/scan",
            &mut files,
            &mut engine,
        );
        assert_eq!(files.len(), 1);
        assert!(engine.is_empty());
        track_error(
            "ERROR: Can't load database",
            "/tmp/scan",
            &mut files,
            &mut engine,
        );
        assert_eq!(engine.len(), 1);
    }
    #[test]
    fn parses_colons_in_paths() {
        assert_eq!(
            parse("/tmp/a: b: Eicar-Signature FOUND"),
            Parsed::Found("/tmp/a: b", "Eicar-Signature")
        );
        assert_eq!(parse("/tmp/FOUND: OK"), Parsed::Clean("/tmp/FOUND"));
        assert_eq!(
            parse("/tmp/no: Access denied ERROR"),
            Parsed::Error("/tmp/no: Access denied ERROR")
        );
        assert_eq!(parse("Infected files: 0"), Parsed::Other);
    }
}
