//! Read-only LaunchAgent preflight. Registration and live ownership handoff are
//! intentionally separate: producing a plan must never load or stop a service.
use crate::store::{err, Result};
use serde::Serialize;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

pub const LABEL: &str = "app.clamav.desktop.worker";
const EXECUTABLE: &str = "clamav-desktop";
const BUNDLE: &str = "ClamAV Desktop.app";
const IDENTIFIER: &str = "app.clamav.desktop";

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Registration {
    Absent,
    Matches,
    Conflict,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub label: &'static str,
    pub executable: PathBuf,
    pub data_directory: PathBuf,
    pub registration_file: PathBuf,
    pub plist: String,
    pub registration: Registration,
    // A matching file does not establish launchd state or user permission.
    pub permission: &'static str,
    pub loaded: Option<bool>,
}

fn clean_absolute(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err("login_service_absolute_path_required".into());
    }
    Ok(())
}

/// Check every existing component without following links. Root-owned system
/// ancestors are accepted, including the standard root:admin /Applications
/// directory. Other group/world-writable components are rejected.
fn safe_path(path: &Path, uid: u32, boundary: &Path) -> Result<()> {
    clean_absolute(path)?;
    let mut current = boundary.to_owned();
    let relative = path
        .strip_prefix(boundary)
        .map_err(|_| "login_service_unsafe_path")?;
    let components = std::iter::once(None).chain(relative.components().map(Some));
    for component in components {
        if let Some(component) = component {
            current.push(component);
        }
        let meta = fs::symlink_metadata(&current).map_err(err)?;
        let system_applications = current == Path::new("/Applications")
            && meta.is_dir()
            && meta.uid() == 0
            && meta.gid() == 80;
        if meta.file_type().is_symlink()
            || (meta.uid() != uid && meta.uid() != 0)
            || meta.mode() & 0o002 != 0
            || (meta.mode() & 0o020 != 0 && !system_applications)
        {
            return Err("login_service_unsafe_path".into());
        }
    }
    Ok(())
}

fn xml(value: &str) -> Result<String> {
    // Reject controls in paths, including XML newline normalization cases.
    if value.chars().any(|c| c < ' ') {
        return Err("login_service_invalid_xml_path".into());
    }
    Ok(value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;"))
}

fn path_xml(path: &Path) -> Result<String> {
    xml(path.to_str().ok_or("login_service_non_utf8_path")?)
}

fn plist(executable: &Path, data: &Path) -> Result<String> {
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{LABEL}</string>
<key>ProgramArguments</key><array><string>{}</string><string>--worker</string><string>serve</string><string>--data-dir</string><string>{}</string></array>
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
<key>ThrottleInterval</key><integer>60</integer>
<key>ProcessType</key><string>Background</string>
<key>LimitLoadToSessionType</key><string>Aqua</string>
<key>Umask</key><integer>63</integer>
</dict></plist>
"#,
        path_xml(executable)?,
        path_xml(data)?
    ))
}

fn registration(path: &Path, expected: &[u8], uid: u32) -> Result<Registration> {
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // A dangling link is a conflict, never an available destination.
            return Ok(if fs::symlink_metadata(path).is_ok() {
                Registration::Conflict
            } else {
                Registration::Absent
            });
        }
        Err(_) => return Ok(Registration::Conflict),
    };
    let meta = file.metadata().map_err(err)?;
    if !meta.is_file()
        || meta.uid() != uid
        || meta.nlink() != 1
        || meta.mode() & 0o077 != 0
        || meta.len() > 16 * 1024
    {
        return Ok(Registration::Conflict);
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(16 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    let named = fs::symlink_metadata(path).map_err(err)?;
    Ok(
        if named.ino() == meta.ino() && named.dev() == meta.dev() && bytes == expected {
            Registration::Matches
        } else {
            Registration::Conflict
        },
    )
}

fn build(
    executable: &Path,
    home: &Path,
    applications: &Path,
    uid: u32,
    boundary: &Path,
) -> Result<Plan> {
    clean_absolute(executable)?;
    safe_path(home, uid, boundary)?;
    let system = applications
        .join(BUNDLE)
        .join("Contents/MacOS")
        .join(EXECUTABLE);
    let personal = home
        .join("Applications")
        .join(BUNDLE)
        .join("Contents/MacOS")
        .join(EXECUTABLE);
    if executable != system && executable != personal {
        return Err("login_service_install_app_required".into());
    }
    safe_path(executable, uid, boundary)?;
    let meta = fs::symlink_metadata(executable).map_err(err)?;
    if !meta.is_file() || meta.nlink() != 1 || meta.mode() & 0o111 == 0 {
        return Err("login_service_invalid_executable".into());
    }
    let data = home.join("Library/Application Support/app.clamav.desktop");
    safe_path(&data, uid, boundary)?;
    let meta = fs::symlink_metadata(&data).map_err(err)?;
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err("login_service_invalid_data_directory".into());
    }
    let library = home.join("Library");
    safe_path(&library, uid, boundary)?;
    let agents = library.join("LaunchAgents");
    match fs::symlink_metadata(&agents) {
        Ok(meta) => {
            safe_path(&agents, uid, boundary)?;
            if !meta.is_dir() || meta.uid() != uid {
                return Err("login_service_unsafe_path".into());
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(err(e)),
    }
    let registration_file = agents.join(format!("{LABEL}.plist"));
    let plist = plist(executable, &data)?;
    let registration = registration(&registration_file, plist.as_bytes(), uid)?;
    Ok(Plan {
        label: LABEL,
        executable: executable.to_owned(),
        data_directory: data,
        registration_file,
        plist,
        registration,
        permission: "unknown",
        loaded: None,
    })
}

/// Production preflight accepts only the installed application and the normal
/// per-user data root. No CLI/environment override can register a development binary.
#[cfg(target_os = "macos")]
pub fn preflight(executable: &Path, home: &Path) -> Result<Plan> {
    let plan = build(
        executable,
        home,
        Path::new("/Applications"),
        unsafe { libc::geteuid() },
        Path::new("/"),
    )?;
    let contents = executable
        .parent()
        .and_then(Path::parent)
        .ok_or("login_service_invalid_bundle")?;
    let info = contents.join("Info.plist");
    safe_path(&info, unsafe { libc::geteuid() }, Path::new("/"))?;
    for (key, expected) in [
        ("CFBundleIdentifier", IDENTIFIER),
        ("CFBundleExecutable", EXECUTABLE),
        ("CFBundlePackageType", "APPL"),
    ] {
        let output = std::process::Command::new("/usr/bin/plutil")
            .args(["-extract", key, "raw", "-o", "-"])
            .arg(&info)
            .output()
            .map_err(err)?;
        if !output.status.success() || output.stdout != format!("{expected}\n").as_bytes() {
            return Err("login_service_invalid_bundle".into());
        }
    }
    let status = std::process::Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict", "-R",
            "anchor apple generic and identifier \"app.clamav.desktop\" and certificate leaf[field.1.2.840.113635.100.6.1.13] exists"])
        .arg(contents.parent().ok_or("login_service_invalid_bundle")?)
        .output()
        .map_err(err)?;
    if !status.status.success() {
        return Err("login_service_invalid_signature".into());
    }
    Ok(plan)
}

/// Resolve the account home from the OS account database, never from HOME.
#[cfg(target_os = "macos")]
pub fn current_user_preflight() -> Result<Plan> {
    use std::{
        ffi::{CStr, OsStr},
        os::unix::ffi::OsStrExt,
    };
    let uid = unsafe { libc::geteuid() };
    let mut buffer = vec![0u8; 64 * 1024];
    let mut record: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result = std::ptr::null_mut();
    let code = unsafe {
        libc::getpwuid_r(
            uid,
            &mut record,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    if code != 0 || result.is_null() || record.pw_dir.is_null() {
        return Err("login_service_account_unavailable".into());
    }
    let home = Path::new(OsStr::from_bytes(
        unsafe { CStr::from_ptr(record.pw_dir) }.to_bytes(),
    ));
    preflight(&std::env::current_exe().map_err(err)?, home)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};
    struct Fixture {
        root: PathBuf,
        home: PathBuf,
        apps: PathBuf,
        exe: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            // Use the canonical temp directory: /tmp is a symlink on macOS.
            let root = std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!("clamav-login-{}", uuid::Uuid::new_v4()));
            let home = root.join("home");
            let apps = root.join("Applications");
            let exe = apps.join(BUNDLE).join("Contents/MacOS").join(EXECUTABLE);
            fs::create_dir_all(exe.parent().unwrap()).unwrap();
            fs::create_dir_all(home.join("Library/Application Support/app.clamav.desktop"))
                .unwrap();
            fs::write(&exe, b"fixture").unwrap();
            fs::set_permissions(&exe, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(
                home.join("Library/Application Support/app.clamav.desktop"),
                fs::Permissions::from_mode(0o700),
            )
            .unwrap();
            Self {
                root,
                home,
                apps,
                exe,
            }
        }
        fn plan(&self) -> Result<Plan> {
            build(
                &self.exe,
                &self.home,
                &self.apps,
                unsafe { libc::geteuid() },
                &self.root,
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn escapes_paths_without_shell_and_throttles_failed_restarts() {
        let xml = plist(
            Path::new("/Applications/A & <B>.app/a"),
            Path::new("/Users/a'b/数据"),
        )
        .unwrap();
        assert!(xml.contains("A &amp; &lt;B&gt;.app"));
        assert!(xml.contains("a&apos;b/数据"));
        assert!(xml.contains("<key>SuccessfulExit</key><false/>"));
        assert!(xml.contains("<key>ThrottleInterval</key><integer>60</integer>"));
        assert!(!xml.contains("StartCalendarInterval"));
        assert!(plist(Path::new("/bad\u{1}path"), Path::new("/data")).is_err());
        assert!(plist(Path::new("/bad\rpath"), Path::new("/data")).is_err());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_accepts_generated_plist_and_argument_boundaries() {
        use std::{
            io::Write,
            process::{Command, Stdio},
        };
        let value = plist(
            Path::new("/Applications/A & B.app/Contents/MacOS/app"),
            Path::new("/Users/名字/Library/Application Support/app"),
        )
        .unwrap();
        let mut child = Command::new("/usr/bin/plutil")
            .args(["-convert", "json", "-o", "-", "--", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(value.as_bytes())
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(result.status.success());
        let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(
            json["ProgramArguments"],
            serde_json::json!([
                "/Applications/A & B.app/Contents/MacOS/app",
                "--worker",
                "serve",
                "--data-dir",
                "/Users/名字/Library/Application Support/app"
            ])
        );
        assert_eq!(json["Umask"], 63);
    }
    #[test]
    fn readonly_plan_does_not_create_registration_and_never_claims_permission() {
        let f = Fixture::new();
        let plan = f.plan().unwrap();
        assert_eq!(plan.registration, Registration::Absent);
        assert_eq!(plan.loaded, None);
        assert_eq!(plan.permission, "unknown");
        assert!(!f.home.join("Library/LaunchAgents").exists());
    }
    #[test]
    fn rejects_development_and_symlinked_or_writable_executable() {
        let f = Fixture::new();
        assert!(build(
            &f.root.join("target/debug/clamav-desktop"),
            &f.home,
            &f.apps,
            unsafe { libc::geteuid() },
            &f.root
        )
        .is_err());
        fs::set_permissions(&f.exe, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(f.plan().is_err());
        fs::remove_file(&f.exe).unwrap();
        symlink("/bin/echo", &f.exe).unwrap();
        assert!(f.plan().is_err());
    }
    #[test]
    fn accepts_personal_applications_but_detects_old_registered_path() {
        let f = Fixture::new();
        let system_plan = f.plan().unwrap();
        fs::create_dir(system_plan.registration_file.parent().unwrap()).unwrap();
        fs::write(&system_plan.registration_file, &system_plan.plist).unwrap();
        fs::set_permissions(
            &system_plan.registration_file,
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let personal = f
            .home
            .join("Applications")
            .join(BUNDLE)
            .join("Contents/MacOS")
            .join(EXECUTABLE);
        fs::create_dir_all(personal.parent().unwrap()).unwrap();
        fs::rename(&f.exe, &personal).unwrap();
        let moved = build(
            &personal,
            &f.home,
            &f.apps,
            unsafe { libc::geteuid() },
            &f.root,
        )
        .unwrap();
        assert_eq!(moved.registration, Registration::Conflict);
        assert_eq!(
            fs::read_to_string(&moved.registration_file).unwrap(),
            system_plan.plist
        );
    }
    #[test]
    fn rejects_shared_data_and_linked_registration_directory() {
        let f = Fixture::new();
        let data = f
            .home
            .join("Library/Application Support/app.clamav.desktop");
        fs::set_permissions(&data, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(f.plan().is_err());
        fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
        symlink(&f.apps, f.home.join("Library/LaunchAgents")).unwrap();
        assert!(f.plan().is_err());
    }
    #[test]
    fn only_exact_private_owned_file_matches_and_unknown_files_are_untouched() {
        let f = Fixture::new();
        let plan = f.plan().unwrap();
        fs::create_dir(plan.registration_file.parent().unwrap()).unwrap();
        fs::write(&plan.registration_file, &plan.plist).unwrap();
        fs::set_permissions(&plan.registration_file, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(f.plan().unwrap().registration, Registration::Matches);
        fs::write(&plan.registration_file, b"another job").unwrap();
        assert_eq!(f.plan().unwrap().registration, Registration::Conflict);
        assert_eq!(fs::read(&plan.registration_file).unwrap(), b"another job");
        fs::remove_file(&plan.registration_file).unwrap();
        symlink(f.root.join("missing"), &plan.registration_file).unwrap();
        assert_eq!(f.plan().unwrap().registration, Registration::Conflict);
    }
    #[test]
    fn rejects_linked_and_public_registration_files() {
        let f = Fixture::new();
        let plan = f.plan().unwrap();
        fs::create_dir(plan.registration_file.parent().unwrap()).unwrap();
        fs::write(&plan.registration_file, &plan.plist).unwrap();
        fs::set_permissions(&plan.registration_file, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(f.plan().unwrap().registration, Registration::Conflict);
        fs::set_permissions(&plan.registration_file, fs::Permissions::from_mode(0o600)).unwrap();
        fs::hard_link(&plan.registration_file, f.root.join("alias")).unwrap();
        assert_eq!(f.plan().unwrap().registration, Registration::Conflict);
    }
}
