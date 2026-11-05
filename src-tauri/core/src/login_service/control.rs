//! The OS adapter only targets this user's fixed label. File publication is
//! atomic and no failed OS operation rolls back by deleting an existing job.
use super::*;
use crate::{
    ipc::{Client, Command, Payload},
    ownership::Ownership,
};
use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    io::Write,
    os::unix::fs::DirBuilderExt,
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Enable,
    Disable,
    Repair,
    Recover,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Enable,
    Bootstrap,
    Bootout,
    PrintService,
    PrintDomain,
}

trait Control {
    fn permission(&mut self, _plan: &Plan) -> Permission {
        Permission::Unknown
    }
    async fn connected(&mut self, _root: &Path) -> bool {
        false
    }
    async fn run(&mut self, action: Action, plan: &Plan) -> Result<i32>;
    async fn stop_if_idle(&mut self, root: &Path) -> Result<()>;
    async fn wait_ready(&mut self, root: &Path) -> Result<()>;
}

fn arguments(action: Action, plan: &Plan) -> Vec<OsString> {
    let domain = format!("gui/{}", plan.uid);
    let target = format!("{domain}/{LABEL}");
    match action {
        Action::Enable => vec!["enable".into(), target.into()],
        Action::Bootstrap => vec![
            "bootstrap".into(),
            domain.into(),
            plan.registration_file.as_os_str().into(),
        ],
        Action::Bootout => vec!["bootout".into(), target.into()],
        Action::PrintService => vec!["print".into(), target.into()],
        Action::PrintDomain => vec!["print".into(), domain.into()],
    }
}

#[cfg(target_os = "macos")]
struct System;
#[cfg(target_os = "macos")]
impl Control for System {
    fn permission(&mut self, plan: &Plan) -> Permission {
        super::os_status::authorization(plan).unwrap_or(Permission::Unknown)
    }
    async fn connected(&mut self, root: &Path) -> bool {
        let Ok(client) = Client::new(root.to_owned()) else {
            return false;
        };
        matches!(
            tokio::time::timeout(
                Duration::from_millis(500),
                client.request(Command::GetSnapshot)
            )
            .await,
            Ok(Ok(Payload::Snapshot(_)))
        )
    }
    async fn run(&mut self, action: Action, plan: &Plan) -> Result<i32> {
        use std::process::Stdio;
        let mut child = tokio::process::Command::new("/bin/launchctl")
            .args(arguments(action, plan))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| "login_service_launchctl_failed")?;
        match tokio::time::timeout(Duration::from_secs(20), child.wait()).await {
            Ok(Ok(status)) => Ok(status.code().unwrap_or(-1)),
            Ok(Err(_)) => Err("login_service_launchctl_failed".into()),
            Err(_) => {
                let _ = tokio::time::timeout(Duration::from_secs(5), child.kill()).await;
                Err("login_service_launchctl_timeout".into())
            }
        }
    }
    async fn wait_ready(&mut self, root: &Path) -> Result<()> {
        let client = Client::new(root.to_owned())?;
        let result = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if let Ok(Ok(Payload::Snapshot(_))) = tokio::time::timeout(
                    Duration::from_millis(500),
                    client.request(Command::GetSnapshot),
                )
                .await
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
        result.map_err(|_| "login_service_worker_start_timeout".into())
    }
    async fn stop_if_idle(&mut self, root: &Path) -> Result<()> {
        match Client::new(root.to_owned())?
            .request(Command::ShutdownIfIdle)
            .await?
        {
            Payload::Ack => Ok(()),
            _ => Err("ipc_invalid_response".into()),
        }
    }
}

fn checked_registration(plan: &Plan) -> Result<Registration> {
    safe_path(&plan.data_directory, plan.uid, &plan.boundary)?;
    let parent = plan
        .registration_file
        .parent()
        .ok_or("login_service_unsafe_path")?;
    safe_path(parent, plan.uid, &plan.boundary)?;
    if fs::symlink_metadata(parent).map_err(err)?.uid() != plan.uid {
        return Err("login_service_unsafe_path".into());
    }
    classify_registration(
        &plan.registration_file,
        &plan.executable,
        &plan.data_directory,
        &plan.alternatives,
        plan.uid,
        &plan.boundary,
    )
}

fn ensure_parent(plan: &Plan) -> Result<()> {
    let parent = plan
        .registration_file
        .parent()
        .ok_or("login_service_unsafe_path")?;
    safe_path(
        parent.parent().ok_or("login_service_unsafe_path")?,
        plan.uid,
        &plan.boundary,
    )?;
    match fs::DirBuilder::new().mode(0o700).create(parent) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(err(e)),
    }
    checked_registration(plan).map(|_| ())
}

fn sync_parent(plan: &Plan) -> Result<()> {
    File::open(
        plan.registration_file
            .parent()
            .ok_or("login_service_unsafe_path")?,
    )
    .and_then(|file| file.sync_all())
    .map_err(err)
}

fn publish(plan: &Plan) -> Result<()> {
    match checked_registration(plan)? {
        Registration::Matches => return Ok(()),
        Registration::Conflict | Registration::Repairable => {
            return Err("login_service_registration_conflict".into())
        }
        Registration::Absent => (),
    }
    let temporary = plan
        .registration_file
        .with_file_name(format!(".{LABEL}-{}.tmp", uuid::Uuid::new_v4()));
    let result: Result<()> = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temporary)
            .map_err(err)?;
        file.write_all(plan.plist.as_bytes()).map_err(err)?;
        file.sync_all().map_err(err)?;
        // Hard-link publication cannot replace an existing file or follow a link.
        fs::hard_link(&temporary, &plan.registration_file)
            .map_err(|_| "login_service_registration_conflict")?;
        Ok(())
    })();
    let cleanup = fs::remove_file(&temporary);
    result?;
    cleanup.map_err(err)?;
    sync_parent(plan)?;
    if checked_registration(plan)? != Registration::Matches {
        return Err("login_service_registration_conflict".into());
    }
    Ok(())
}

async fn idle_owner(plan: &Plan, control: &mut impl Control, wait: Duration) -> Result<Ownership> {
    match Ownership::acquire(&plan.data_directory) {
        Ok(guard) => return Ok(guard),
        Err(reason) if reason == "data_in_use" => (),
        Err(reason) => return Err(reason),
    }
    // Never send unconditional Shutdown: an active scan/transfer must win.
    control.stop_if_idle(&plan.data_directory).await?;
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        match Ownership::acquire(&plan.data_directory) {
            Ok(guard) => return Ok(guard),
            Err(reason) if reason == "data_in_use" && tokio::time::Instant::now() < deadline => (),
            Err(_) => return Err("login_service_worker_stop_timeout".into()),
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn bootout(plan: &Plan, control: &mut impl Control) -> Result<()> {
    let code = control.run(Action::Bootout, plan).await?;
    if code == 0
        // launchctl bootout reports ESRCH (3) for an absent job, whereas
        // launchctl print reports 113. Require independent absence proof.
        || (matches!(code, 3 | 113)
            && control.run(Action::PrintDomain, plan).await? == 0
            && control.run(Action::PrintService, plan).await? == 113)
    {
        return Ok(());
    }
    Err("login_service_bootout_failed".into())
}
async fn loaded_with(plan: &Plan, control: &mut impl Control) -> (Option<bool>, bool) {
    match control.run(Action::PrintService, plan).await {
        Ok(0) => (Some(true), false),
        Ok(113) if matches!(control.run(Action::PrintDomain, plan).await, Ok(0)) => {
            (Some(false), false)
        }
        _ => (None, true),
    }
}
#[cfg(target_os = "macos")]
pub(super) async fn loaded(plan: &Plan) -> (Option<bool>, bool) {
    loaded_with(plan, &mut System).await
}

fn replace_moved_registration(plan: &Plan) -> Result<PathBuf> {
    if checked_registration(plan)? != Registration::Repairable {
        return Err("login_service_registration_conflict".into());
    }
    let old = private_registration(&plan.registration_file, plan.uid)?
        .ok_or("login_service_registration_conflict")?;
    let backup = plan
        .registration_file
        .with_file_name(format!(".{LABEL}-{}.backup", uuid::Uuid::new_v4()));
    let temporary = plan
        .registration_file
        .with_file_name(format!(".{LABEL}-{}.tmp", uuid::Uuid::new_v4()));
    let result: Result<()> = (|| {
        let mut copy = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&backup)
            .map_err(err)?;
        copy.write_all(&old).map_err(err)?;
        copy.sync_all().map_err(err)?;
        let mut next = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temporary)
            .map_err(err)?;
        next.write_all(plan.plist.as_bytes()).map_err(err)?;
        next.sync_all().map_err(err)?;
        if checked_registration(plan)? != Registration::Repairable
            || private_registration(&plan.registration_file, plan.uid)?.as_deref()
                != Some(old.as_slice())
        {
            return Err("login_service_registration_conflict".into());
        }
        // Same-directory rename atomically replaces only the rechecked known
        // template. The old bytes remain in a private backup on failure.
        fs::rename(&temporary, &plan.registration_file).map_err(err)?;
        sync_parent(plan)
    })();
    let _ = fs::remove_file(&temporary);
    result?;
    Ok(backup)
}

async fn manage(
    plan: &Plan,
    operation: Operation,
    control: &mut impl Control,
    wait: Duration,
) -> Result<()> {
    // Separate from owner.lock: serialize through the release/bootstrap gap too.
    let _manager = Ownership::acquire_service(&plan.data_directory)
        .map_err(|_| "login_service_management_busy")?;
    match operation {
        Operation::Enable => {
            // Enabling does not stop a GUI/manual worker or silently adopt it.
            let owner = Ownership::acquire(&plan.data_directory)?;
            if matches!(
                plan.registration,
                Registration::Conflict | Registration::Repairable
            ) {
                return Err("login_service_registration_conflict".into());
            }
            ensure_parent(plan)?;
            publish(plan)?;
            if control.run(Action::Enable, plan).await? != 0 {
                return Err("login_service_enable_failed".into());
            }
            // launchd's worker must acquire the same lock before opening SQLite.
            drop(owner);
            if checked_registration(plan)? != Registration::Matches {
                return Err("login_service_registration_conflict".into());
            }
            if control.run(Action::Bootstrap, plan).await? != 0 {
                // Keep the validated file for explicit retry/removal. In particular,
                // never bootout a possibly pre-existing service as rollback.
                return Err("login_service_bootstrap_failed".into());
            }
            control.wait_ready(&plan.data_directory).await?;
        }
        Operation::Disable => {
            // Refuse unknown/moved/absent registrations before sending worker IPC.
            if checked_registration(plan)? != Registration::Matches {
                return Err("login_service_registration_conflict".into());
            }
            let _owner = idle_owner(plan, control, wait).await?;
            if checked_registration(plan)? != Registration::Matches {
                return Err("login_service_registration_conflict".into());
            }
            bootout(plan, control).await?;
            if checked_registration(plan)? != Registration::Matches {
                return Err("login_service_registration_conflict".into());
            }
            fs::remove_file(&plan.registration_file).map_err(err)?;
            sync_parent(plan)?;
            // _owner prevents a restarting process from acquiring Core during
            // bootout/unlink. No database, definitions or quarantine are removed.
        }
        Operation::Recover => {
            if checked_registration(plan)? != Registration::Matches {
                return Err("login_service_registration_conflict".into());
            }
            // Reconnection never stops a healthy worker, including active jobs.
            if control.connected(&plan.data_directory).await {
                return Ok(());
            }
            if control.permission(plan) == Permission::RequiresApproval {
                return Err("login_service_approval_required".into());
            }
            let owner = idle_owner(plan, control, wait).await?;
            if checked_registration(plan)? != Registration::Matches {
                return Err("login_service_registration_conflict".into());
            }
            match loaded_with(plan, control).await {
                (Some(true), _) => bootout(plan, control).await?,
                (Some(false), _) => (),
                _ => return Err("login_service_query_failed".into()),
            }
            // No enable override or forced kickstart: user OS approval remains
            // authoritative. Both paths reload the known on-disk definition.
            drop(owner);
            if checked_registration(plan)? != Registration::Matches {
                return Err("login_service_registration_conflict".into());
            }
            if control.run(Action::Bootstrap, plan).await? != 0 {
                return Err("login_service_bootstrap_failed".into());
            }
            control.wait_ready(&plan.data_directory).await?;
        }
        Operation::Repair => {
            if checked_registration(plan)? != Registration::Repairable {
                return Err("login_service_registration_conflict".into());
            }
            let owner = idle_owner(plan, control, wait).await?;
            if checked_registration(plan)? != Registration::Repairable {
                return Err("login_service_registration_conflict".into());
            }
            bootout(plan, control).await?;
            let backup = replace_moved_registration(plan)?;
            if control.run(Action::Enable, plan).await? != 0 {
                return Err("login_service_enable_failed".into());
            }
            drop(owner);
            if checked_registration(plan)? != Registration::Matches {
                return Err("login_service_registration_conflict".into());
            }
            if control.run(Action::Bootstrap, plan).await? != 0 {
                return Err("login_service_bootstrap_failed".into());
            }
            control.wait_ready(&plan.data_directory).await?;
            fs::remove_file(backup).map_err(err)?;
            sync_parent(plan)?;
        }
    }
    Ok(())
}

/// Only an explicit local command reaches this adapter. Never accepts a plan,
/// executable path, label, UID or data directory from an untrusted caller.
#[cfg(target_os = "macos")]
pub async fn manage_current(operation: Operation) -> Result<()> {
    let plan = current_user_preflight()?;
    manage(&plan, operation, &mut System, Duration::from_secs(15)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};
    struct Fixture {
        root: PathBuf,
        plan: Plan,
    }
    impl Fixture {
        fn new() -> Self {
            // Keep fixture IPC below Darwin's Unix socket path limit.
            let root = PathBuf::from("/private/tmp").join(format!(
                "ca-{}",
                &uuid::Uuid::new_v4().simple().to_string()[..12]
            ));
            let home = root.join("home");
            let apps = root.join("Applications");
            let exe = apps.join(BUNDLE).join("Contents/MacOS").join(EXECUTABLE);
            fs::create_dir_all(exe.parent().unwrap()).unwrap();
            let data = home.join("Library/Application Support/app.clamav.desktop");
            fs::create_dir_all(&data).unwrap();
            fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
            fs::write(&exe, b"fixture").unwrap();
            fs::set_permissions(&exe, fs::Permissions::from_mode(0o700)).unwrap();
            let plan = build(&exe, &home, &apps, unsafe { libc::geteuid() }, &root).unwrap();
            Self { root, plan }
        }
        fn publish(&self) {
            ensure_parent(&self.plan).unwrap();
            publish(&self.plan).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    struct Mock {
        calls: Vec<Action>,
        codes: Vec<i32>,
        owner: Option<Ownership>,
        busy: bool,
        retain_owner: bool,
        ready_error: bool,
        stopped: bool,
        ready: bool,
        connected: bool,
        permission: Permission,
    }
    impl Mock {
        fn new(codes: Vec<i32>) -> Self {
            Self {
                calls: vec![],
                codes,
                owner: None,
                busy: false,
                retain_owner: false,
                ready_error: false,
                stopped: false,
                ready: false,
                connected: false,
                permission: Permission::Unknown,
            }
        }
    }
    impl Control for Mock {
        fn permission(&mut self, _: &Plan) -> Permission {
            self.permission
        }
        async fn connected(&mut self, _: &Path) -> bool {
            self.connected
        }
        async fn run(&mut self, action: Action, plan: &Plan) -> Result<i32> {
            self.calls.push(action);
            // On enable the data owner is held until launchd is about to start.
            // During bootout the lock remains held so no replacement can enter.
            let owner = Ownership::acquire(&plan.data_directory);
            match action {
                Action::Bootstrap => assert!(owner.is_ok()),
                _ => assert_eq!(owner.err().unwrap(), "data_in_use"),
            }
            Ok(self.codes.remove(0))
        }
        async fn stop_if_idle(&mut self, _: &Path) -> Result<()> {
            self.stopped = true;
            if self.busy {
                return Err("busy".into());
            }
            if !self.retain_owner {
                self.owner.take();
            }
            Ok(())
        }
        async fn wait_ready(&mut self, root: &Path) -> Result<()> {
            self.ready = true;
            // The handoff released owner.lock but still serializes service management.
            assert!(Ownership::acquire(root).is_ok());
            assert!(Ownership::acquire_service(root).is_err());
            if self.ready_error {
                Err("login_service_worker_start_timeout".into())
            } else {
                Ok(())
            }
        }
    }
    async fn run(f: &Fixture, op: Operation, m: &mut Mock) -> Result<()> {
        manage(&f.plan, op, m, Duration::from_millis(5)).await
    }
    #[tokio::test]
    async fn publishes_private_plist_and_releases_owner_before_bootstrap() {
        let f = Fixture::new();
        let mut m = Mock::new(vec![0, 0]);
        run(&f, Operation::Enable, &mut m).await.unwrap();
        assert_eq!(m.calls, [Action::Enable, Action::Bootstrap]);
        assert!(m.ready);
        assert!(!m.stopped);
        assert_eq!(
            checked_registration(&f.plan).unwrap(),
            Registration::Matches
        );
        assert_eq!(
            fs::metadata(&f.plan.registration_file).unwrap().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::metadata(&f.plan.registration_file).unwrap().nlink(), 1);
        assert_eq!(
            fs::read_dir(f.plan.registration_file.parent().unwrap())
                .unwrap()
                .count(),
            1
        );
    }
    #[tokio::test]
    async fn registration_never_stops_or_replaces_existing_owner() {
        let f = Fixture::new();
        let _owner = Ownership::acquire(&f.plan.data_directory).unwrap();
        let mut m = Mock::new(vec![]);
        assert_eq!(
            run(&f, Operation::Enable, &mut m).await.unwrap_err(),
            "data_in_use"
        );
        assert!(m.calls.is_empty());
        assert!(!m.stopped);
        assert!(!f.plan.registration_file.exists());
    }
    #[tokio::test]
    async fn enable_failures_preserve_known_registration_without_destructive_rollback() {
        for (codes, reason) in [
            (vec![1], "login_service_enable_failed"),
            (vec![0, 5], "login_service_bootstrap_failed"),
        ] {
            let f = Fixture::new();
            let mut m = Mock::new(codes);
            assert_eq!(
                run(&f, Operation::Enable, &mut m).await.unwrap_err(),
                reason
            );
            assert_eq!(
                checked_registration(&f.plan).unwrap(),
                Registration::Matches
            );
            assert!(!m.calls.contains(&Action::Bootout));
        }
        let f = Fixture::new();
        let mut m = Mock::new(vec![0, 0]);
        m.ready_error = true;
        assert_eq!(
            run(&f, Operation::Enable, &mut m).await.unwrap_err(),
            "login_service_worker_start_timeout"
        );
        assert_eq!(
            checked_registration(&f.plan).unwrap(),
            Registration::Matches
        );
        assert!(Ownership::acquire_service(&f.plan.data_directory).is_ok());
    }
    #[tokio::test]
    async fn unknown_or_linked_registration_is_never_changed_or_sent_to_launchctl() {
        for linked in [false, true] {
            let f = Fixture::new();
            ensure_parent(&f.plan).unwrap();
            let other = f.root.join("other");
            fs::write(&other, b"unrelated").unwrap();
            if linked {
                symlink(&other, &f.plan.registration_file).unwrap();
            } else {
                fs::write(&f.plan.registration_file, b"unrelated").unwrap();
            }
            for op in [
                Operation::Enable,
                Operation::Disable,
                Operation::Repair,
                Operation::Recover,
            ] {
                let mut m = Mock::new(vec![]);
                assert_eq!(
                    run(&f, op, &mut m).await.unwrap_err(),
                    "login_service_registration_conflict"
                );
                assert!(m.calls.is_empty());
                assert!(!m.stopped);
            }
            assert_eq!(fs::read(&other).unwrap(), b"unrelated");
            assert_eq!(fs::read(&f.plan.registration_file).unwrap(), b"unrelated");
        }
    }
    #[tokio::test]
    async fn manager_lock_serializes_operations_independently_of_core_owner() {
        let f = Fixture::new();
        let _guard = Ownership::acquire_service(&f.plan.data_directory).unwrap();
        let mut m = Mock::new(vec![]);
        assert_eq!(
            run(&f, Operation::Enable, &mut m).await.unwrap_err(),
            "login_service_management_busy"
        );
        assert!(m.calls.is_empty());
        assert!(Ownership::acquire(&f.plan.data_directory).is_ok());
    }
    #[tokio::test]
    async fn removal_waits_for_idle_shutdown_and_preserves_user_data() {
        let f = Fixture::new();
        f.publish();
        fs::write(f.plan.data_directory.join("history-fixture"), b"retain").unwrap();
        let mut m = Mock::new(vec![0]);
        m.owner = Some(Ownership::acquire(&f.plan.data_directory).unwrap());
        run(&f, Operation::Disable, &mut m).await.unwrap();
        assert!(m.stopped);
        assert_eq!(m.calls, [Action::Bootout]);
        assert!(!f.plan.registration_file.exists());
        assert_eq!(
            fs::read(f.plan.data_directory.join("history-fixture")).unwrap(),
            b"retain"
        );
        assert!(Ownership::acquire(&f.plan.data_directory).is_ok());
    }
    #[tokio::test]
    async fn busy_or_unreleased_worker_prevents_bootout_and_deletion() {
        for busy in [true, false] {
            let f = Fixture::new();
            f.publish();
            let mut m = Mock::new(vec![]);
            m.owner = Some(Ownership::acquire(&f.plan.data_directory).unwrap());
            m.busy = busy;
            m.retain_owner = true;
            let reason = run(&f, Operation::Disable, &mut m).await.unwrap_err();
            assert_eq!(
                reason,
                if busy {
                    "busy"
                } else {
                    "login_service_worker_stop_timeout"
                }
            );
            assert!(m.calls.is_empty());
            assert_eq!(
                checked_registration(&f.plan).unwrap(),
                Registration::Matches
            );
        }
    }
    #[tokio::test]
    async fn uncertain_bootout_keeps_registration_but_confirmed_absence_can_be_removed() {
        for (codes, success) in [
            (vec![5], false),
            (vec![3, 5], false),
            (vec![3, 0, 0], false),
            (vec![3, 0, 5], false),
            (vec![3, 0, 113], true),
            (vec![113, 5], false),
            (vec![113, 0, 0], false),
            (vec![113, 0, 113], true),
        ] {
            let f = Fixture::new();
            f.publish();
            let mut m = Mock::new(codes);
            assert_eq!(run(&f, Operation::Disable, &mut m).await.is_ok(), success);
            assert_eq!(f.plan.registration_file.exists(), !success);
        }
    }
    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "requires an installed signed app, no running GUI/worker and an unloaded login service"]
    async fn native_absent_bootout_preserves_registration() {
        let executable = std::env::var_os("CLAMAV_SIGNED_APP_TEST_EXECUTABLE")
            .expect("installed signed executable required");
        let plan = preflight(Path::new(&executable), &account_home().unwrap()).unwrap();
        let _management = Ownership::acquire_service(&plan.data_directory).unwrap();
        let _owner = Ownership::acquire(&plan.data_directory).unwrap();
        let before = fs::read(&plan.registration_file).unwrap();
        assert_eq!(loaded_with(&plan, &mut System).await, (Some(false), false));
        bootout(&plan, &mut System).await.unwrap();
        assert_eq!(fs::read(&plan.registration_file).unwrap(), before);
    }
    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "mutates the account login service; requires an explicitly prepared unloaded moved registration"]
    async fn native_unloaded_moved_registration_repair() {
        let executable = std::env::var_os("CLAMAV_SIGNED_APP_TEST_EXECUTABLE")
            .expect("installed signed executable required");
        let plan = preflight(Path::new(&executable), &account_home().unwrap()).unwrap();
        assert_eq!(
            checked_registration(&plan).unwrap(),
            Registration::Repairable
        );
        assert_eq!(loaded_with(&plan, &mut System).await, (Some(false), false));
        manage(
            &plan,
            Operation::Repair,
            &mut System,
            Duration::from_secs(15),
        )
        .await
        .unwrap();
        assert_eq!(checked_registration(&plan).unwrap(), Registration::Matches);
        assert_eq!(loaded_with(&plan, &mut System).await, (Some(true), false));
    }
    #[test]
    fn command_arguments_target_only_this_users_fixed_service() {
        let f = Fixture::new();
        assert_eq!(
            arguments(Action::Bootstrap, &f.plan),
            vec![
                OsString::from("bootstrap"),
                format!("gui/{}", f.plan.uid).into(),
                f.plan.registration_file.as_os_str().into()
            ]
        );
        assert_eq!(
            arguments(Action::Bootout, &f.plan),
            vec![
                OsString::from("bootout"),
                format!("gui/{}/{LABEL}", f.plan.uid).into()
            ]
        );
    }
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn real_worker_idle_shutdown_releases_core_before_mock_bootout() {
        use crate::{ipc::Server, store::Core};
        use std::sync::Arc;
        struct Live(Mock);
        impl Control for Live {
            async fn run(&mut self, action: Action, plan: &Plan) -> Result<i32> {
                self.0.run(action, plan).await
            }
            async fn stop_if_idle(&mut self, root: &Path) -> Result<()> {
                System.stop_if_idle(root).await
            }
            async fn wait_ready(&mut self, root: &Path) -> Result<()> {
                System.wait_ready(root).await
            }
        }
        let f = Fixture::new();
        f.publish();
        let core = Arc::new(Core::new(f.plan.data_directory.clone()).unwrap());
        // A live job must reject removal, without unloading the service.
        core.document.lock().unwrap().operation = Some(Default::default());
        let server = Server::bind(core.clone()).unwrap();
        let task = tokio::spawn(server.run(std::future::pending()));
        let mut control = Live(Mock::new(vec![0]));
        assert_eq!(
            manage(
                &f.plan,
                Operation::Disable,
                &mut control,
                Duration::from_secs(2)
            )
            .await
            .unwrap_err(),
            "busy"
        );
        assert!(control.0.calls.is_empty());
        assert_eq!(
            checked_registration(&f.plan).unwrap(),
            Registration::Matches
        );
        core.document.lock().unwrap().operation = None;
        drop(core);
        manage(
            &f.plan,
            Operation::Disable,
            &mut control,
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(control.0.calls, [Action::Bootout]);
        assert!(!f.plan.registration_file.exists());
        assert!(!f.plan.data_directory.join("worker.sock").exists());
        assert!(Core::new(f.plan.data_directory.clone()).is_ok());
    }
    fn moved_fixture() -> Fixture {
        let mut f = Fixture::new();
        f.publish();
        let destination = f
            .plan
            .alternatives
            .iter()
            .find(|path| **path != f.plan.executable)
            .unwrap()
            .clone();
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::rename(&f.plan.executable, &destination).unwrap();
        f.plan = build(
            &destination,
            &f.root.join("home"),
            &f.root.join("Applications"),
            unsafe { libc::geteuid() },
            &f.root,
        )
        .unwrap();
        assert_eq!(f.plan.registration, Registration::Repairable);
        f
    }
    #[tokio::test]
    async fn repair_of_unloaded_moved_registration_requires_confirmed_absence() {
        for (codes, success) in [
            (vec![3, 0, 113, 0, 0], true),
            (vec![3, 0, 0], false),
            (vec![3, 5], false),
        ] {
            let f = moved_fixture();
            let before = fs::read(&f.plan.registration_file).unwrap();
            let mut control = Mock::new(codes);
            assert_eq!(
                run(&f, Operation::Repair, &mut control).await.is_ok(),
                success
            );
            if success {
                assert!(control.ready);
                assert_eq!(
                    checked_registration(&f.plan).unwrap(),
                    Registration::Matches
                );
            } else {
                assert_eq!(fs::read(&f.plan.registration_file).unwrap(), before);
                assert!(!control.calls.contains(&Action::Enable));
                assert!(!control.calls.contains(&Action::Bootstrap));
            }
        }
    }
    #[tokio::test]
    async fn repairs_only_known_moved_template_after_idle_shutdown_and_removes_backup_on_success() {
        let f = moved_fixture();
        let mut m = Mock::new(vec![0, 0, 0]);
        m.owner = Some(Ownership::acquire(&f.plan.data_directory).unwrap());
        run(&f, Operation::Repair, &mut m).await.unwrap();
        assert!(m.stopped);
        assert_eq!(
            m.calls,
            [Action::Bootout, Action::Enable, Action::Bootstrap]
        );
        assert_eq!(
            checked_registration(&f.plan).unwrap(),
            Registration::Matches
        );
        assert_eq!(
            fs::metadata(&f.plan.registration_file).unwrap().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::read_dir(f.plan.registration_file.parent().unwrap())
                .unwrap()
                .count(),
            1
        );
    }
    #[tokio::test]
    async fn failed_repair_preserves_old_registration_or_private_backup_without_destructive_rollback(
    ) {
        for codes in [vec![5], vec![0, 5], vec![0, 0, 5]] {
            let f = moved_fixture();
            let before = fs::read(&f.plan.registration_file).unwrap();
            let mut m = Mock::new(codes);
            assert!(run(&f, Operation::Repair, &mut m).await.is_err());
            let after = fs::read(&f.plan.registration_file).unwrap();
            if before != after {
                let backup = fs::read_dir(f.plan.registration_file.parent().unwrap())
                    .unwrap()
                    .map(|item| item.unwrap().path())
                    .find(|path| path.extension().is_some_and(|ext| ext == "backup"))
                    .unwrap();
                assert_eq!(fs::read(&backup).unwrap(), before);
                assert_eq!(fs::metadata(backup).unwrap().mode() & 0o777, 0o600);
                assert_eq!(
                    checked_registration(&f.plan).unwrap(),
                    Registration::Matches
                );
            } else {
                assert_eq!(
                    checked_registration(&f.plan).unwrap(),
                    Registration::Repairable
                );
            }
            assert_eq!(m.calls.iter().filter(|a| **a == Action::Bootout).count(), 1);
        }
    }
    #[tokio::test]
    async fn repair_refuses_active_work_and_recreated_old_app_or_changed_registration() {
        let f = moved_fixture();
        let old = fs::read(&f.plan.registration_file).unwrap();
        let mut m = Mock::new(vec![]);
        m.owner = Some(Ownership::acquire(&f.plan.data_directory).unwrap());
        m.busy = true;
        assert_eq!(
            run(&f, Operation::Repair, &mut m).await.unwrap_err(),
            "busy"
        );
        assert!(m.calls.is_empty());
        assert_eq!(fs::read(&f.plan.registration_file).unwrap(), old);
        m.owner.take();
        let previous = f
            .plan
            .alternatives
            .iter()
            .find(|path| **path != f.plan.executable)
            .unwrap();
        fs::write(previous, b"another copy").unwrap();
        assert_eq!(
            run(&f, Operation::Repair, &mut m).await.unwrap_err(),
            "login_service_registration_conflict"
        );
        fs::remove_file(previous).unwrap();
        fs::write(&f.plan.registration_file, b"unrelated").unwrap();
        assert_eq!(
            run(&f, Operation::Repair, &mut m).await.unwrap_err(),
            "login_service_registration_conflict"
        );
        assert!(m.calls.is_empty());
    }
    #[tokio::test]
    async fn loading_queries_never_parse_text_or_turn_query_failure_into_absence() {
        struct Query {
            codes: Vec<i32>,
            calls: Vec<Action>,
        }
        impl Control for Query {
            async fn run(&mut self, action: Action, _: &Plan) -> Result<i32> {
                self.calls.push(action);
                Ok(self.codes.remove(0))
            }
            async fn stop_if_idle(&mut self, _: &Path) -> Result<()> {
                panic!("read-only query")
            }
            async fn wait_ready(&mut self, _: &Path) -> Result<()> {
                panic!("read-only query")
            }
        }
        let f = Fixture::new();
        for (codes, expected) in [
            (vec![0], (Some(true), false)),
            (vec![113, 0], (Some(false), false)),
            (vec![113, 5], (None, true)),
            (vec![5], (None, true)),
        ] {
            let mut q = Query {
                codes,
                calls: vec![],
            };
            assert_eq!(loaded_with(&f.plan, &mut q).await, expected);
            assert!(q
                .calls
                .iter()
                .all(|action| matches!(action, Action::PrintService | Action::PrintDomain)));
            assert!(!f.plan.registration_file.exists());
        }
    }
    #[test]
    fn moved_detection_refuses_foreign_templates_and_linked_old_locations() {
        let f = moved_fixture();
        let previous = f
            .plan
            .alternatives
            .iter()
            .find(|path| **path != f.plan.executable)
            .unwrap();
        let bytes = fs::read(&f.plan.registration_file).unwrap();
        symlink("/missing-target", previous).unwrap();
        assert_eq!(
            checked_registration(&f.plan).unwrap(),
            Registration::Conflict
        );
        fs::remove_file(previous).unwrap();
        let changed = String::from_utf8(bytes.clone())
            .unwrap()
            .replace(LABEL, "app.unrelated.worker");
        fs::write(&f.plan.registration_file, changed).unwrap();
        assert_eq!(
            checked_registration(&f.plan).unwrap(),
            Registration::Conflict
        );
        fs::write(
            &f.plan.registration_file,
            plist(
                Path::new("/Applications/Other.app/Contents/MacOS/clamav-desktop"),
                &f.plan.data_directory,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            checked_registration(&f.plan).unwrap(),
            Registration::Conflict
        );
        fs::write(&f.plan.registration_file, bytes).unwrap();
        assert_eq!(
            checked_registration(&f.plan).unwrap(),
            Registration::Repairable
        );
    }
    #[tokio::test]
    async fn recovery_reconnects_healthy_busy_worker_without_os_mutation() {
        let f = Fixture::new();
        f.publish();
        let mut m = Mock::new(vec![]);
        m.owner = Some(Ownership::acquire(&f.plan.data_directory).unwrap());
        m.connected = true;
        m.busy = true;
        m.permission = Permission::RequiresApproval;
        run(&f, Operation::Recover, &mut m).await.unwrap();
        assert!(!m.stopped);
        assert!(m.calls.is_empty());
        assert_eq!(
            checked_registration(&f.plan).unwrap(),
            Registration::Matches
        );
    }
    #[tokio::test]
    async fn recovery_respects_approval_and_busy_work_before_launchctl() {
        for approval in [true, false] {
            let f = Fixture::new();
            f.publish();
            let mut m = Mock::new(vec![]);
            m.owner = Some(Ownership::acquire(&f.plan.data_directory).unwrap());
            m.busy = true;
            m.permission = if approval {
                Permission::RequiresApproval
            } else {
                Permission::Enabled
            };
            assert_eq!(
                run(&f, Operation::Recover, &mut m).await.unwrap_err(),
                if approval {
                    "login_service_approval_required"
                } else {
                    "busy"
                }
            );
            assert_eq!(m.stopped, !approval);
            assert!(m.calls.is_empty());
        }
    }
    #[tokio::test]
    async fn recovery_reloads_loaded_service_or_bootstraps_absent_one_without_enabling_override() {
        for (codes, actions) in [
            (
                vec![0, 0, 0],
                vec![Action::PrintService, Action::Bootout, Action::Bootstrap],
            ),
            (
                vec![113, 0, 0],
                vec![Action::PrintService, Action::PrintDomain, Action::Bootstrap],
            ),
        ] {
            let f = Fixture::new();
            f.publish();
            let before = fs::read(&f.plan.registration_file).unwrap();
            let mut m = Mock::new(codes);
            m.permission = Permission::Enabled;
            run(&f, Operation::Recover, &mut m).await.unwrap();
            assert_eq!(m.calls, actions);
            assert!(m.ready);
            assert_eq!(fs::read(&f.plan.registration_file).unwrap(), before);
        }
    }
    #[tokio::test]
    async fn uncertain_loading_or_failed_restart_preserves_registration() {
        for (codes, reason) in [
            (vec![5], "login_service_query_failed"),
            (vec![0, 5], "login_service_bootout_failed"),
            (vec![113, 0, 5], "login_service_bootstrap_failed"),
        ] {
            let f = Fixture::new();
            f.publish();
            let mut m = Mock::new(codes);
            assert_eq!(
                run(&f, Operation::Recover, &mut m).await.unwrap_err(),
                reason
            );
            assert_eq!(
                checked_registration(&f.plan).unwrap(),
                Registration::Matches
            );
            assert!(!m.calls.contains(&Action::Enable));
        }
    }
}
