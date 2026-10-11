use crate::{model::*, process, scanner, store::*};
use futures_util::StreamExt;
use pgp::composed::{Deserializable, DetachedSignature, SignedPublicKey};
use pgp::types::KeyDetails;
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tauri::AppHandle;
use tokio::io::AsyncWriteExt;
// Reviewed release. Updating the engine is an explicit app release change, never remote code selection.
pub const VERSION: &str = "1.5.4";
pub const PACKAGE: &str = "clamav-1.5.4.macos.universal.pkg";
const TALOS_KEY: &str = include_str!("../resources/cisco-talos.gpg");
fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let url = attempt.url();
            if attempt.previous().len() <= 3
                && url.scheme() == "https"
                && matches!(
                    url.host_str(),
                    Some("www.clamav.net" | "clamav-site.s3.amazonaws.com")
                )
            {
                attempt.follow()
            } else {
                attempt.error("untrusted_download_redirect")
            }
        }))
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(1200))
        .build()
        .map_err(err)
}
async fn download(
    app: &AppHandle,
    url: &str,
    path: &Path,
    limit: u64,
    progress: bool,
) -> Result<()> {
    let response = client()?
        .get(url)
        .send()
        .await
        .map_err(err)?
        .error_for_status()
        .map_err(err)?;
    if response.content_length().is_some_and(|n| n > limit) {
        return Err("download_too_large".into());
    }
    let total = response.content_length();
    let mut stream = response.bytes_stream();
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
        .map_err(err)?;
    let c = core(app);
    let mut size = 0;
    let mut last = Instant::now();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(err)?;
        size += chunk.len() as u64;
        if size > limit {
            return Err("download_too_large".into());
        }
        file.write_all(&chunk).await.map_err(err)?;
        if progress && last.elapsed() > Duration::from_millis(200) {
            last = Instant::now();
            if let Some(op) = &mut c.document.lock().unwrap().operation {
                op.downloaded = size;
                op.total = total;
            }
            c.publish(app);
        }
    }
    file.sync_all().await.map_err(err)?;
    if size == 0 || total.is_some_and(|n| n != size) {
        return Err("download_incomplete".into());
    }
    Ok(())
}
fn trusted_key() -> Result<SignedPublicKey> {
    let (mut key, _) = SignedPublicKey::from_armor_single(TALOS_KEY.as_bytes()).map_err(err)?;
    let id = key.key_id();
    // The official certificate includes an old key's third-party certification.
    // Only self-certifications are verified with the pinned primary key.
    for user in &mut key.details.users {
        user.signatures
            .retain(|sig| sig.issuer().iter().any(|issuer| **issuer == id));
    }
    key.verify().map_err(err)?;
    if !key.details.revocation_signatures.is_empty() {
        return Err("signing_key_revoked".into());
    }
    if key.expires_at().is_some_and(|at| at < chrono::Utc::now()) {
        return Err("signing_key_expired".into());
    }
    Ok(key)
}
pub fn verify_pgp(package: &Path, signature: &Path) -> Result<()> {
    let key = trusted_key()?;
    let content = std::fs::read(package).map_err(err)?;
    let raw = std::fs::read(signature).map_err(err)?;
    let sig = if raw.starts_with(b"-----BEGIN") {
        DetachedSignature::from_armor_single(&raw[..])
            .map_err(err)?
            .0
    } else {
        DetachedSignature::from_bytes(&raw[..]).map_err(err)?
    };
    sig.verify(&key, &content)
        .map_err(|e| format!("pgp_verification_failed: {e}"))
}
async fn verify_apple(package: &Path) -> Result<()> {
    let path = package.to_string_lossy().to_string();
    let mut output = String::new();
    let result = process::run(
        Path::new("/usr/sbin/pkgutil"),
        &["--check-signature".into(), path.clone()],
        None,
        Duration::from_secs(60),
        |s| output.push_str(s),
    )
    .await?;
    if result.code != Some(0)
        || !(output.contains("signed by a certificate trusted by")
            || output
                .contains("signed by a developer certificate issued by Apple for distribution"))
        || !output.contains("Developer ID Installer: Cisco (DE8Y96K9QP)")
    {
        return Err(format!("pkg_signature_failed: {output}"));
    }
    let result = process::run(
        Path::new("/usr/sbin/spctl"),
        &["--assess".into(), "--type".into(), "install".into(), path],
        None,
        Duration::from_secs(120),
        |_| {},
    )
    .await?;
    if result.code != Some(0) {
        return Err("gatekeeper_rejected_package".into());
    }
    Ok(())
}
pub async fn prepare(app: AppHandle) -> Result<()> {
    let c = core(&app);
    {
        let mut d = c.document.lock().unwrap();
        if d.operation.is_some() || d.scan.as_ref().is_some_and(|s| s.status == "running") {
            return Err("busy".into());
        }
        d.operation = Some(Operation {
            kind: "installer".into(),
            stage: "downloading".into(),
            ..Default::default()
        });
        d.installer_ready = false;
    }
    c.publish(&app);
    let result = prepare_inner(&app).await;
    c.document.lock().unwrap().operation = None;
    match &result {
        Ok(_) => c.log(&app, "installer_ready", VERSION),
        Err(e) => c.log(&app, "error", e),
    }
    result
}
async fn prepare_inner(app: &AppHandle) -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Err("macos_only".into());
    }
    let c = core(app);
    crate::downloads::ensure_download_budget(&c)?;
    let dir = c
        .root
        .join("downloads")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir(&dir).map_err(err)?;
    private_dir(&dir)?;
    let pkg = dir.join(PACKAGE);
    let sig = dir.join(format!("{PACKAGE}.sig"));
    download(
        app,
        &format!("https://www.clamav.net/downloads/production/{PACKAGE}"),
        &pkg,
        256 * 1024 * 1024,
        true,
    )
    .await?;
    download(
        app,
        &format!("https://www.clamav.net/downloads/production/{PACKAGE}.sig"),
        &sig,
        16384,
        false,
    )
    .await?;
    if let Some(op) = &mut c.document.lock().unwrap().operation {
        op.stage = "verifying".into();
    }
    c.publish(app);
    let p = pkg.clone();
    tauri::async_runtime::spawn_blocking(move || verify_pgp(&p, &sig))
        .await
        .map_err(err)??;
    verify_apple(&pkg).await?;
    let hash = scanner::digest(&pkg)?;
    *c.prepared.lock().unwrap() = Some((pkg, hash));
    c.document.lock().unwrap().installer_ready = true;
    Ok(())
}
pub async fn open(app: &AppHandle) -> Result<()> {
    let c = core(app);
    let (pkg, hash) = c
        .prepared
        .lock()
        .unwrap()
        .clone()
        .ok_or("installer_not_prepared")?;
    if scanner::digest(&pkg)? != hash {
        return Err("package_changed".into());
    }
    verify_apple(&pkg).await?;
    let result = process::run(
        Path::new("/usr/bin/open"),
        &[
            "-a".into(),
            "Installer".into(),
            pkg.to_string_lossy().into(),
        ],
        None,
        Duration::from_secs(15),
        |_| {},
    )
    .await?;
    if result.code != Some(0) {
        return Err("installer_open_failed".into());
    }
    c.log(app, "installer_opened", VERSION);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_key_is_valid() {
        trusted_key().unwrap();
    }
    #[tokio::test]
    #[ignore = "requires official PKG fixture; never installs it"]
    async fn verifies_official_release() {
        let path = std::env::var("CLAMAV_TEST_PKG").expect("CLAMAV_TEST_PKG required");
        verify_pgp(Path::new(&path), Path::new(&format!("{path}.sig"))).unwrap();
        verify_apple(Path::new(&path)).await.unwrap();
    }
    #[test]
    fn tampered_package_is_rejected() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&root).unwrap();
        let pkg = root.join("pkg");
        let sig = root.join("sig");
        std::fs::write(&pkg, b"tampered").unwrap();
        std::fs::write(&sig, b"invalid").unwrap();
        assert!(verify_pgp(&pkg, &sig).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
