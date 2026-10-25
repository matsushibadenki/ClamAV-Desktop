//! Versioned, same-user IPC. One bounded request per connection; no file contents.
use crate::{
    model::{Settings, Snapshot},
    scanner,
    store::{err, now, Core, Result},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    future::Future,
    io::Write,
    os::unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::{Mutex, Notify, Semaphore},
    task::JoinSet,
};

pub const VERSION: u32 = 1;
pub const REQUEST_LIMIT: usize = 64 * 1024;
pub const RESPONSE_LIMIT: usize = 8 * 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(10);
const RETRY_WINDOW: i64 = 24 * 3600;
const LEDGER_LIMIT: i64 = 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    GetSnapshot,
    StartScan {
        kind: String,
        target: Option<String>,
    },
    StopScan,
    UpdateDefinitions,
    RefreshEngine,
    GetEngineInspectionResult {
        request_id: String,
    },
    GetQuarantineDetail {
        id: String,
    },
    VerifyQuarantineIntegrity {
        id: String,
    },
    StopIntegrityCheck,
    QuarantineFile {
        id: String,
    },
    RestoreFile {
        id: String,
    },
    RecoverFileTransfers,
    GetDownloadCache,
    CleanupDownloadCache,
    ExportData {
        kind: crate::exports::Kind,
        destination: String,
        app_update: Option<serde_json::Value>,
    },
    GetFileOperationResult {
        request_id: String,
    },
    GetIntegrityResult {
        request_id: String,
    },
    SaveSettings {
        settings: Settings,
    },
    Shutdown,
    ShutdownIfIdle,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub id: String,
    pub issued_at: String,
    pub body: Command,
}
impl Request {
    pub fn new(body: Command) -> Self {
        Self {
            version: VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            issued_at: now(),
            body,
        }
    }
    fn validate(&self) -> Result<()> {
        if self.version != VERSION {
            return Err("ipc_version_mismatch".into());
        }
        if uuid::Uuid::parse_str(&self.id).is_err() {
            return Err("ipc_invalid_request_id".into());
        }
        let at = chrono::DateTime::parse_from_rfc3339(&self.issued_at)
            .map_err(|_| "ipc_invalid_request_time")?
            .timestamp();
        let age = chrono::Utc::now().timestamp() - at;
        if !(-300..RETRY_WINDOW).contains(&age) {
            return Err("ipc_request_expired".into());
        }
        if let Command::GetIntegrityResult { request_id }
        | Command::GetFileOperationResult { request_id }
        | Command::GetEngineInspectionResult { request_id } = &self.body
        {
            if uuid::Uuid::parse_str(request_id).is_err() {
                return Err("ipc_invalid_request_id".into());
            }
        }
        if let Command::ExportData {
            destination,
            app_update,
            ..
        } = &self.body
        {
            if !Path::new(destination).is_absolute()
                || destination.contains('\0')
                || destination.len() > 4096
            {
                return Err("invalid_path".into());
            }
            if let Some(status) = app_update {
                if !status.is_object() || serde_json::to_vec(status).map_err(err)?.len() > 48 * 1024
                {
                    return Err("ipc_frame_too_large".into());
                }
            }
        }
        match &self.body {
            Command::GetQuarantineDetail { id }
            | Command::VerifyQuarantineIntegrity { id }
            | Command::QuarantineFile { id }
            | Command::RestoreFile { id }
                if id.is_empty() || id.len() > 128 || id.contains('\0') =>
            {
                return Err("finding_missing".into())
            }
            _ => (),
        }
        if let Command::StartScan { kind, target } = &self.body {
            match kind.as_str() {
                "quick" | "full" if target.is_none() => (),
                "custom"
                    if target
                        .as_ref()
                        .is_some_and(|p| Path::new(p).is_absolute() && !p.contains('\0')) =>
                {
                    ()
                }
                "quick" | "full" | "custom" => return Err("ipc_invalid_scan_target".into()),
                _ => return Err("invalid_scan_kind".into()),
            }
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Payload {
    Snapshot(Box<Snapshot>),
    ScanStarted {
        scan_id: String,
    },
    QuarantineDetail(Box<crate::details::Detail>),
    IntegrityResult {
        result: Option<Result<()>>,
    },
    FileOperationResult {
        result: Option<Result<()>>,
    },
    DownloadCache(Box<crate::downloads::CacheReport>),
    EngineInspectionResult {
        result: Option<Result<crate::model::Engine>>,
    },
    Ack,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Response {
    pub version: u32,
    pub id: String,
    pub payload: Option<Payload>,
    pub error: Option<String>,
}
impl Response {
    fn new(id: &str, result: Result<Payload>) -> Self {
        match result {
            Ok(payload) => Self {
                version: VERSION,
                id: id.into(),
                payload: Some(payload),
                error: None,
            },
            Err(error) => Self {
                version: VERSION,
                id: id.into(),
                payload: None,
                error: Some(error),
            },
        }
    }
    pub fn into_result(self) -> Result<Payload> {
        match (self.payload, self.error) {
            (Some(payload), None) => Ok(payload),
            (None, Some(error)) => Err(error),
            _ => Err("ipc_invalid_response".into()),
        }
    }
}

pub fn socket_path(root: &Path) -> Result<PathBuf> {
    let path = root.join("worker.sock");
    let address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if !path.is_absolute()
        || path.as_os_str().as_bytes().contains(&0)
        || path.as_os_str().as_bytes().len() >= address.sun_path.len()
    {
        return Err("ipc_socket_path_too_long".into());
    }
    Ok(path)
}
fn private_parent(root: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(root).map_err(err)?;
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
        return Err("ipc_invalid_directory".into());
    }
    Ok(())
}
fn socket_metadata(path: &Path) -> Result<fs::Metadata> {
    let meta = fs::symlink_metadata(path).map_err(err)?;
    if !meta.file_type().is_socket()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o077 != 0
    {
        return Err("ipc_invalid_socket".into());
    }
    Ok(meta)
}
fn same_user(actual: u32, expected: u32) -> Result<()> {
    if actual != expected {
        return Err("ipc_peer_rejected".into());
    }
    Ok(())
}
fn authenticate(stream: &UnixStream) -> Result<()> {
    use std::os::fd::AsRawFd;
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    let uid = {
        let (mut uid, mut gid) = (0, 0);
        if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
            return Err(format!(
                "ipc_peer_failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        uid
    };
    #[cfg(target_os = "linux")]
    let uid = {
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut cred as *mut libc::ucred).cast(),
                &mut size,
            )
        } != 0
        {
            return Err("ipc_peer_failed".into());
        }
        cred.uid
    };
    #[cfg(not(any(target_os = "macos", target_os = "freebsd", target_os = "linux")))]
    return Err("ipc_peer_unsupported".into());
    #[cfg(any(target_os = "macos", target_os = "freebsd", target_os = "linux"))]
    same_user(uid, unsafe { libc::geteuid() })
}
struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("ipc_frame_too_large"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn encode(value: &impl Serialize, limit: usize) -> Result<Vec<u8>> {
    let mut writer = BoundedWriter {
        bytes: vec![],
        limit,
    };
    serde_json::to_writer(&mut writer, value).map_err(err)?;
    Ok(writer.bytes)
}
async fn read_frame(stream: &mut UnixStream, limit: usize) -> Result<Vec<u8>> {
    tokio::time::timeout(IO_TIMEOUT, async {
        let size = stream.read_u32().await.map_err(err)? as usize;
        if size == 0 || size > limit {
            return Err("ipc_frame_too_large".into());
        }
        let mut bytes = vec![0; size];
        stream.read_exact(&mut bytes).await.map_err(err)?;
        Ok(bytes)
    })
    .await
    .map_err(|_| "ipc_timeout")?
}
async fn write_frame(stream: &mut UnixStream, bytes: &[u8]) -> Result<()> {
    tokio::time::timeout(IO_TIMEOUT, async {
        stream.write_u32(bytes.len() as u32).await.map_err(err)?;
        stream.write_all(bytes).await.map_err(err)?;
        stream.shutdown().await.map_err(err)
    })
    .await
    .map_err(|_| "ipc_timeout")?
}

#[derive(Clone)]
pub struct Client {
    root: PathBuf,
}
impl Client {
    pub fn new(root: PathBuf) -> Result<Self> {
        let root = fs::canonicalize(root).map_err(err)?;
        private_parent(&root)?;
        socket_path(&root)?;
        Ok(Self { root })
    }
    /// A retry must reuse the entire original Request, including its id and timestamp.
    pub async fn send(&self, request: &Request) -> Result<Response> {
        request.validate()?;
        private_parent(&self.root)?;
        let path = socket_path(&self.root)?;
        let before = socket_metadata(&path)?;
        let mut stream = tokio::time::timeout(IO_TIMEOUT, UnixStream::connect(&path))
            .await
            .map_err(|_| "ipc_timeout")?
            .map_err(err)?;
        authenticate(&stream)?;
        let after = socket_metadata(&path)?;
        if before.dev() != after.dev() || before.ino() != after.ino() {
            return Err("ipc_socket_changed".into());
        }
        // Do not half-close here: the server still needs to write the response.
        let bytes = encode(request, REQUEST_LIMIT)?;
        tokio::time::timeout(IO_TIMEOUT, async {
            stream.write_u32(bytes.len() as u32).await.map_err(err)?;
            stream.write_all(&bytes).await.map_err(err)
        })
        .await
        .map_err(|_| "ipc_timeout")??;
        let response: Response =
            serde_json::from_slice(&read_frame(&mut stream, RESPONSE_LIMIT).await?)
                .map_err(|_| "ipc_invalid_response")?;
        if response.version != VERSION || response.id != request.id {
            return Err("ipc_invalid_response".into());
        }
        Ok(response)
    }
    pub async fn request(&self, command: Command) -> Result<Payload> {
        self.send(&Request::new(command)).await?.into_result()
    }
}
struct SocketFile {
    path: PathBuf,
    dev: u64,
    inode: u64,
}
struct ListenerLease(Arc<Core>);
impl Drop for ListenerLease {
    fn drop(&mut self) {
        self.0.ipc_active.store(false, Ordering::Release);
    }
}
impl Drop for SocketFile {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path).is_ok_and(|m| {
            m.file_type().is_socket() && m.dev() == self.dev && m.ino() == self.inode
        }) {
            let _ = fs::remove_file(&self.path);
        }
    }
}
type InspectionResults = Arc<
    std::sync::Mutex<std::collections::VecDeque<(String, Option<Result<crate::model::Engine>>)>>,
>;
struct ServiceState {
    core: Arc<Core>,
    commands: Mutex<()>,
    stopping: AtomicBool,
    shutdown: Notify,
    definition_job: Mutex<Option<tokio::task::JoinHandle<()>>>,
    inspection_job: Mutex<Option<tokio::task::JoinHandle<()>>>,
    inspection_results: InspectionResults,
    integrity_job: Mutex<Option<tokio::task::JoinHandle<()>>>,
    integrity_result:
        Arc<std::sync::Mutex<std::collections::VecDeque<(String, Option<Result<()>>)>>>,
    file_job: Mutex<Option<tokio::task::JoinHandle<()>>>,
    file_result: Arc<std::sync::Mutex<std::collections::VecDeque<(String, Option<Result<()>>)>>>,
}
impl ServiceState {
    fn maintenance_tick(&self, clock: &impl crate::scheduler::Clock) -> Result<()> {
        self.maintenance_tick_with(clock, |core| async move {
            let _ = crate::engine::complete_update(&core).await;
        })
    }
    fn maintenance_tick_with<F, Fut>(
        &self,
        clock: &impl crate::scheduler::Clock,
        update: F,
    ) -> Result<()>
    where
        F: FnOnce(Arc<Core>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        // The same gate as ShutdownIfIdle and all mutations. No new automatic
        // job can slip in after stopping, nor can repeated timer ticks queue up.
        let Ok(_commands) = self.commands.try_lock() else {
            return Ok(());
        };
        let Ok(_maintenance) = self.core.maintenance_gate.try_lock() else {
            return Ok(());
        };
        if self.stopping.load(Ordering::Acquire) {
            return Ok(());
        }
        match crate::scheduler::plan(&self.core, clock)? {
            crate::scheduler::Action::None => Ok(()),
            crate::scheduler::Action::Scan { date, id } => {
                crate::scheduler::start_claimed(&self.core, &date, &id, clock)
            }
            crate::scheduler::Action::Update { at } => {
                let Ok(mut slot) = self.definition_job.try_lock() else {
                    return Ok(());
                };
                if slot.as_ref().is_some_and(|job| !job.is_finished()) {
                    return Ok(());
                }
                if crate::scheduler::begin_auto_update(&self.core, at)? {
                    let core = self.core.clone();
                    *slot = Some(tokio::spawn(update(core)));
                }
                Ok(())
            }
        }
    }

    async fn start_inspection_job<F, Fut>(&self, request_id: String, run: F) -> Result<Payload>
    where
        F: FnOnce(Arc<Core>) -> Fut,
        Fut: Future<Output = Result<crate::model::Engine>> + Send + 'static,
    {
        let mut slot = self.inspection_job.lock().await;
        if slot.as_ref().is_some_and(|job| !job.is_finished()) {
            return Err("busy".into());
        }
        crate::engine::begin_inspection(&self.core)?;
        {
            let mut results = self.inspection_results.lock().unwrap();
            results.push_back((request_id.clone(), None));
            while results.len() > 64 {
                results.pop_front();
            }
        }
        let results = self.inspection_results.clone();
        let core = self.core.clone();
        let work = run(core.clone());
        *slot = Some(tokio::spawn(async move {
            let result = crate::engine::finish_inspection(&core, work.await);
            if let Some((_, saved)) = results
                .lock()
                .unwrap()
                .iter_mut()
                .find(|(id, _)| id == &request_id)
            {
                *saved = Some(result);
            }
        }));
        Ok(Payload::Ack)
    }
    async fn start_definition_job<F, Fut>(&self, run: F) -> Result<Payload>
    where
        F: FnOnce(Arc<Core>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let mut job = self.definition_job.lock().await;
        if job.as_ref().is_some_and(|task| !task.is_finished()) {
            return Err("busy".into());
        }
        crate::engine::begin_update(&self.core)?;
        *job = Some(tokio::spawn(run(self.core.clone())));
        Ok(Payload::Ack)
    }
    async fn start_integrity_job(&self, id: String, request_id: String) -> Result<Payload> {
        let mut slot = self.integrity_job.lock().await;
        if slot.as_ref().is_some_and(|job| !job.is_finished()) {
            return Err("busy".into());
        }
        {
            let mut results = self.integrity_result.lock().unwrap();
            results.push_back((request_id.clone(), None));
            while results.len() > 64 {
                results.pop_front();
            }
        }
        let final_result = self.integrity_result.clone();
        let (ready, accepted) = tokio::sync::oneshot::channel();
        let core = self.core.clone();
        *slot = Some(tokio::task::spawn_blocking(move || {
            let mut ready = Some(ready);
            let result = crate::details::verify_started(&core, &id, || {
                if let Some(ready) = ready.take() {
                    let _ = ready.send(Ok(()));
                }
            });
            if let Some((_, saved)) = final_result
                .lock()
                .unwrap()
                .iter_mut()
                .find(|(id, _)| id == &request_id)
            {
                *saved = Some(result.clone());
            }
            if let Some(ready) = ready {
                let _ = ready.send(result.clone());
            } else if let Err(reason) = result {
                core.log("error", reason);
            }
        }));
        accepted.await.map_err(|_| "worker_integrity_failed")??;
        Ok(Payload::Ack)
    }
    async fn start_file_job(
        &self,
        action: crate::quarantine::FileAction,
        request_id: String,
    ) -> Result<Payload> {
        let mut slot = self.file_job.lock().await;
        if slot.as_ref().is_some_and(|job| !job.is_finished()) {
            return Err("busy".into());
        }
        {
            let mut results = self.file_result.lock().unwrap();
            results.push_back((request_id.clone(), None));
            while results.len() > 64 {
                results.pop_front();
            }
        }
        let final_result = self.file_result.clone();
        let (ready, accepted) = tokio::sync::oneshot::channel();
        let core = self.core.clone();
        *slot = Some(tokio::task::spawn_blocking(move || {
            let mut ready = Some(ready);
            let result = crate::quarantine::run_file_operation(&core, action, || {
                if let Some(ready) = ready.take() {
                    let _ = ready.send(Ok(()));
                }
            });
            if let Some((_, saved)) = final_result
                .lock()
                .unwrap()
                .iter_mut()
                .find(|(id, _)| id == &request_id)
            {
                *saved = Some(result.clone());
            }
            if let Some(ready) = ready {
                let _ = ready.send(result.clone());
            }
        }));
        accepted
            .await
            .map_err(|_| "worker_file_operation_failed")??;
        Ok(Payload::Ack)
    }
    async fn dispatch(&self, request: &Request) -> Response {
        let result = self.execute(request).await;
        Response::new(&request.id, result)
    }
    async fn execute(&self, request: &Request) -> Result<Payload> {
        request.validate()?;
        if matches!(request.body, Command::GetSnapshot) {
            return Ok(Payload::Snapshot(Box::new(self.core.snapshot())));
        }
        if let Command::GetEngineInspectionResult { request_id } = &request.body {
            let slot = self.inspection_job.lock().await;
            let finished = slot.as_ref().is_some_and(|job| job.is_finished());
            return match self
                .inspection_results
                .lock()
                .unwrap()
                .iter()
                .find(|(id, _)| id == request_id)
            {
                Some((_, None)) if finished => Err("worker_engine_inspection_failed".into()),
                Some((_, result)) => Ok(Payload::EngineInspectionResult {
                    result: result.clone(),
                }),
                _ => Err("ipc_outcome_unknown".into()),
            };
        }
        if let Command::GetIntegrityResult { request_id } = &request.body {
            let slot = self.integrity_job.lock().await;
            let finished = slot.as_ref().is_some_and(|job| job.is_finished());
            return match self
                .integrity_result
                .lock()
                .unwrap()
                .iter()
                .find(|(id, _)| id == request_id)
            {
                Some((_, None)) if finished => Err("worker_integrity_failed".into()),
                Some((_, result)) => Ok(Payload::IntegrityResult {
                    result: result.clone(),
                }),
                _ => Err("ipc_outcome_unknown".into()),
            };
        }
        if let Command::GetFileOperationResult { request_id } = &request.body {
            let slot = self.file_job.lock().await;
            let finished = slot.as_ref().is_some_and(|job| job.is_finished());
            return match self
                .file_result
                .lock()
                .unwrap()
                .iter()
                .find(|(id, _)| id == request_id)
            {
                Some((_, None)) if finished => Err("worker_file_operation_failed".into()),
                Some((_, result)) => Ok(Payload::FileOperationResult {
                    result: result.clone(),
                }),
                _ => Err("ipc_outcome_unknown".into()),
            };
        }
        if matches!(request.body, Command::GetDownloadCache) {
            let core = self.core.clone();
            return tokio::task::spawn_blocking(move || {
                let _gate = core.file_gate.try_lock().map_err(|_| "busy")?;
                crate::downloads::report(&core)
            })
            .await
            .map_err(err)?
            .map(|report| Payload::DownloadCache(Box::new(report)));
        }
        if let Command::GetQuarantineDetail { id } = &request.body {
            let core = self.core.clone();
            let id = id.clone();
            return tokio::task::spawn_blocking(move || crate::details::get(&core, &id))
                .await
                .map_err(err)?
                .map(|detail| Payload::QuarantineDetail(Box::new(detail)));
        }
        let _gate = self.commands.lock().await;
        let fingerprint = serde_json::to_string(request).map_err(err)?;
        {
            let db = self.core.db.lock().unwrap();
            use rusqlite::OptionalExtension;
            let previous: Option<(String, Option<String>)> = db
                .query_row(
                    "SELECT request,response FROM ipc_requests WHERE id=?1",
                    [&request.id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(err)?;
            if let Some((stored, response)) = previous {
                if stored != fingerprint {
                    return Err("ipc_request_id_reused".into());
                }
                return match response {
                    Some(json) => serde_json::from_str::<Response>(&json)
                        .map_err(err)?
                        .into_result(),
                    None => Err("ipc_outcome_unknown".into()),
                };
            }
            if self.stopping.load(Ordering::Acquire) {
                return Err("worker_stopping".into());
            }
            let now = chrono::Utc::now().timestamp();
            db.execute("DELETE FROM ipc_requests WHERE expires_at<=?1", [now])
                .map_err(err)?;
            let count: i64 = db
                .query_row("SELECT COUNT(*) FROM ipc_requests", [], |r| r.get(0))
                .map_err(err)?;
            if count >= LEDGER_LIMIT {
                return Err("ipc_request_limit".into());
            }
            let expires = chrono::DateTime::parse_from_rfc3339(&request.issued_at)
                .map_err(err)?
                .timestamp()
                + RETRY_WINDOW;
            db.execute(
                "INSERT INTO ipc_requests(id,request,response,expires_at) VALUES (?1,?2,NULL,?3)",
                rusqlite::params![request.id, fingerprint, expires],
            )
            .map_err(err)?;
        }
        let result = match &request.body {
            Command::SaveSettings { settings } => {
                crate::settings::save(&self.core, settings.clone()).map(|()| Payload::Ack)
            }
            Command::StartScan { kind, target } => {
                scanner::start(self.core.clone(), kind.clone(), target.clone())
                    .await
                    .map(|scan_id| Payload::ScanStarted { scan_id })
            }
            Command::RefreshEngine => {
                self.start_inspection_job(request.id.clone(), |core| async move {
                    crate::engine::inspect(&core).await
                })
                .await
            }
            Command::UpdateDefinitions => {
                self.start_definition_job(|core| async move {
                    let _ = crate::engine::complete_update(&core).await;
                })
                .await
            }
            Command::VerifyQuarantineIntegrity { id } => {
                self.start_integrity_job(id.clone(), request.id.clone())
                    .await
            }
            Command::QuarantineFile { id } => {
                self.start_file_job(
                    crate::quarantine::FileAction::Quarantine(id.clone()),
                    request.id.clone(),
                )
                .await
            }
            Command::RestoreFile { id } => {
                self.start_file_job(
                    crate::quarantine::FileAction::Restore(id.clone()),
                    request.id.clone(),
                )
                .await
            }
            Command::RecoverFileTransfers => {
                self.start_file_job(crate::quarantine::FileAction::Recover, request.id.clone())
                    .await
            }
            Command::CleanupDownloadCache => {
                self.start_file_job(
                    crate::quarantine::FileAction::CleanupCache,
                    request.id.clone(),
                )
                .await
            }
            Command::ExportData {
                kind,
                destination,
                app_update,
            } => {
                self.start_file_job(
                    crate::quarantine::FileAction::Export {
                        kind: kind.clone(),
                        destination: destination.clone(),
                        app_update: app_update.clone(),
                    },
                    request.id.clone(),
                )
                .await
            }
            Command::StopIntegrityCheck => {
                crate::details::stop(&self.core);
                Ok(Payload::Ack)
            }
            Command::StopScan => {
                self.core.cancel.store(true, Ordering::Relaxed);
                Ok(Payload::Ack)
            }
            Command::ShutdownIfIdle => {
                let document = self.core.document.lock().unwrap();
                if document.operation.is_some()
                    || document
                        .scan
                        .as_ref()
                        .is_some_and(|scan| scan.status == "running")
                {
                    Err("busy".into())
                } else {
                    // Serialized with all mutation requests: no new job may be
                    // accepted between the idle check and the stopping flag.
                    self.stopping.store(true, Ordering::Release);
                    Ok(Payload::Ack)
                }
            }
            Command::Shutdown => {
                self.stopping.store(true, Ordering::Release);
                self.core.cancel.store(true, Ordering::Relaxed);
                Ok(Payload::Ack)
            }
            Command::GetSnapshot
            | Command::GetQuarantineDetail { .. }
            | Command::GetIntegrityResult { .. }
            | Command::GetFileOperationResult { .. }
            | Command::GetDownloadCache
            | Command::GetEngineInspectionResult { .. } => unreachable!(),
        };
        let response = Response::new(&request.id, result);
        let saved = self.core.db.lock().unwrap().execute(
            "UPDATE ipc_requests SET response=?1 WHERE id=?2",
            rusqlite::params![serde_json::to_string(&response).map_err(err)?, request.id],
        );
        if saved.is_err() {
            return Err("ipc_outcome_unknown".into());
        }
        response.into_result()
    }
    async fn connection(self: Arc<Self>, mut stream: UnixStream) -> Result<()> {
        authenticate(&stream)?;
        let bytes = read_frame(&mut stream, REQUEST_LIMIT).await?;
        let request: Request = serde_json::from_slice(&bytes).map_err(|_| "ipc_invalid_request")?;
        let response = self.dispatch(&request).await;
        let bytes = encode(&response, RESPONSE_LIMIT).unwrap_or_else(|_| {
            encode(
                &Response::new(&request.id, Err("ipc_frame_too_large".into())),
                RESPONSE_LIMIT,
            )
            .unwrap()
        });
        let result = write_frame(&mut stream, &bytes).await;
        if self.stopping.load(Ordering::Acquire) {
            self.shutdown.notify_one();
        }
        result
    }
}

pub struct Server {
    listener: UnixListener,
    socket: SocketFile,
    state: Arc<ServiceState>,
    _lease: ListenerLease,
}
impl Server {
    /// Core ownership must be acquired before removing a socket left by a crashed worker.
    pub fn bind(core: Arc<Core>) -> Result<Self> {
        if core.ipc_active.swap(true, Ordering::AcqRel) {
            return Err("worker_already_running".into());
        }
        let lease = ListenerLease(core.clone());
        private_parent(&core.root)?;
        let path = socket_path(&core.root)?;
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                socket_metadata(&path)?;
                fs::remove_file(&path).map_err(err)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(err(e)),
        }
        let listener = UnixListener::bind(&path).map_err(err)?;
        let meta = fs::symlink_metadata(&path).map_err(err)?;
        let socket = SocketFile {
            path: path.clone(),
            dev: meta.dev(),
            inode: meta.ino(),
        };
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(err)?;
        socket_metadata(&path)?;
        core.db.lock().unwrap().execute_batch("CREATE TABLE IF NOT EXISTS ipc_requests (id TEXT PRIMARY KEY, request TEXT NOT NULL, response TEXT, expires_at INTEGER NOT NULL);").map_err(err)?;
        Ok(Self {
            listener,
            socket,
            state: Arc::new(ServiceState {
                core,
                commands: Mutex::new(()),
                stopping: AtomicBool::new(false),
                shutdown: Notify::new(),
                definition_job: Mutex::new(None),
                inspection_job: Mutex::new(None),
                inspection_results: Arc::new(std::sync::Mutex::new(
                    std::collections::VecDeque::new(),
                )),
                file_job: Mutex::new(None),
                file_result: Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
                integrity_job: Mutex::new(None),
                integrity_result: Arc::new(
                    std::sync::Mutex::new(std::collections::VecDeque::new()),
                ),
            }),
            _lease: lease,
        })
    }
    pub fn path(&self) -> &Path {
        &self.socket.path
    }
    pub async fn run(self, stop: impl Future<Output = ()>) -> Result<()> {
        self.run_inner(stop, false).await
    }
    pub async fn run_scheduled(self, stop: impl Future<Output = ()>) -> Result<()> {
        self.run_inner(stop, true).await
    }
    async fn run_inner(self, stop: impl Future<Output = ()>, scheduled: bool) -> Result<()> {
        tokio::pin!(stop);
        let mut maintenance = tokio::time::interval(Duration::from_secs(30));
        maintenance.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let permits = Arc::new(Semaphore::new(8));
        let mut tasks = JoinSet::new();
        loop {
            tokio::select! {
                _ = &mut stop => break,
                _ = self.state.shutdown.notified() => break,
                _ = maintenance.tick(), if scheduled => {
                    if let Err(reason) = self.state.maintenance_tick(&crate::scheduler::SystemClock) {
                        self.state.core.log("error", format!("scheduler: {reason}"));
                    }
                }
                Some(_) = tasks.join_next(), if !tasks.is_empty() => (),
                accepted = self.listener.accept() => {
                    let (stream, _) = accepted.map_err(err)?;
                    if let Ok(permit) = permits.clone().try_acquire_owned() {
                        let state = self.state.clone();
                        tasks.spawn(async move { let _permit = permit; let _ = state.connection(stream).await; });
                    }
                }
            }
        }
        self.state.stopping.store(true, Ordering::Release);
        self.state.core.cancel.store(true, Ordering::Relaxed);
        crate::details::stop(&self.state.core);
        self.state
            .core
            .definitions_cancel
            .store(true, Ordering::Relaxed);
        // Existing replies finish before socket removal. Slow readers are bounded by IO_TIMEOUT.
        while tasks.join_next().await.is_some() {}
        self.state.core.cancel.store(true, Ordering::Relaxed);
        crate::details::stop(&self.state.core);
        self.state
            .core
            .definitions_cancel
            .store(true, Ordering::Relaxed);
        if let Some(mut job) = self.state.definition_job.lock().await.take() {
            match tokio::time::timeout(Duration::from_secs(30), &mut job).await {
                Ok(result) => result.map_err(|_| "worker_update_failed")?,
                Err(_) => {
                    job.abort();
                    let _ = job.await;
                    return Err("worker_stop_timeout".into());
                }
            }
        }
        if let Some(mut job) = self.state.inspection_job.lock().await.take() {
            match tokio::time::timeout(Duration::from_secs(30), &mut job).await {
                Ok(result) => result.map_err(|_| "worker_engine_inspection_failed")?,
                Err(_) => {
                    job.abort();
                    let _ = job.await;
                    return Err("worker_stop_timeout".into());
                }
            }
        }
        if let Some(job) = self.state.integrity_job.lock().await.take() {
            // Blocking filesystem reads cannot safely be aborted. Keep Core ownership until
            // the read returns and cancellation/result persistence has completed.
            job.await.map_err(|_| "worker_integrity_failed")?;
        }
        if let Some(job) = self.state.file_job.lock().await.take() {
            // Transfers are not interruptible: preserve journal/DB ownership until completion.
            job.await.map_err(|_| "worker_file_operation_failed")?;
        }
        tokio::time::timeout(Duration::from_secs(30), async {
            while self
                .state
                .core
                .snapshot()
                .scan
                .is_some_and(|s| s.status == "running")
            {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .map_err(|_| "worker_stop_timeout")?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn automatic_update_uses_owned_job_and_cannot_start_after_shutdown() {
        let root =
            PathBuf::from("/private/tmp").join(format!("clamav-auto-{}", uuid::Uuid::new_v4()));
        let core = Arc::new(Core::new(root.clone()).unwrap());
        {
            let mut doc = core.document.lock().unwrap();
            doc.settings.auto_update = true;
            doc.engine.checked = true;
            doc.engine.version = Some("fixture".into());
        }
        let server = Server::bind(core.clone()).unwrap();
        // Stand-in child exercises actual cancel/reap logic without any CDN request.
        server
            .state
            .maintenance_tick_with(&crate::scheduler::SystemClock, |core| async move {
                let outcome = crate::process::run(
                    Path::new("/bin/sleep"),
                    &["30".into()],
                    Some(&core.definitions_cancel),
                    Duration::from_secs(40),
                    |_| {},
                )
                .await
                .unwrap();
                assert!(outcome.cancelled);
                core.document.lock().unwrap().operation = None;
            })
            .unwrap();
        assert_eq!(core.snapshot().operation.unwrap().kind, "definitions");
        server
            .state
            .maintenance_tick_with(&crate::scheduler::SystemClock, |_| async {
                panic!("duplicate automatic update");
            })
            .unwrap();
        let state = server.state.clone();
        let task = tokio::spawn(server.run(std::future::pending()));
        let client = Client::new(root.clone()).unwrap();
        assert_eq!(
            client.request(Command::ShutdownIfIdle).await.err().unwrap(),
            "busy"
        );
        client.request(Command::Shutdown).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        state
            .maintenance_tick_with(&crate::scheduler::SystemClock, |_| async {
                panic!("started after stopping");
            })
            .unwrap();
        assert!(core.snapshot().operation.is_none());
        drop(state);
        drop(core);
        drop(Core::new(root.clone()).unwrap());
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn inspection_keeps_control_requests_responsive_and_shutdown_awaits_result() {
        let root = PathBuf::from("/private/tmp")
            .join(format!("clamav-inspection-{}", uuid::Uuid::new_v4()));
        let core = Arc::new(Core::new(root.clone()).unwrap());
        let server = Server::bind(core.clone()).unwrap();
        let (finish, finished) = tokio::sync::oneshot::channel::<()>();
        let request_id = uuid::Uuid::new_v4().to_string();
        let results = server.state.inspection_results.clone();
        server
            .state
            .start_inspection_job(request_id.clone(), |_| async move {
                finished.await.unwrap();
                Err("fixture_probe_failure".into())
            })
            .await
            .unwrap();
        let task = tokio::spawn(server.run(std::future::pending()));
        let client = Client::new(root.clone()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            match client.request(Command::GetSnapshot).await.unwrap() {
                Payload::Snapshot(state) => {
                    assert_eq!(state.operation.unwrap().kind, "engine_inspection")
                }
                _ => panic!("snapshot expected"),
            }
            client.request(Command::StopScan).await.unwrap();
            let busy = Request::new(Command::RefreshEngine);
            for _ in 0..2 {
                assert_eq!(
                    client
                        .send(&busy)
                        .await
                        .unwrap()
                        .into_result()
                        .err()
                        .unwrap(),
                    "busy"
                );
            }
            client.request(Command::Shutdown).await.unwrap();
        })
        .await
        .unwrap();
        assert_eq!(Core::new(root.clone()).err().unwrap(), "data_in_use");
        assert!(!task.is_finished());
        finish.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(core.snapshot().operation.is_none());
        let saved = results.lock().unwrap();
        let result = saved
            .iter()
            .find(|(id, _)| id == &request_id)
            .unwrap()
            .1
            .as_ref()
            .unwrap();
        assert_eq!(result.as_ref().err().unwrap(), "fixture_probe_failure");
        drop(saved);
        drop(core);
        let reopened = Core::new(root.clone()).unwrap();
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn definition_job_keeps_ipc_responsive_and_shutdown_waits_for_child() {
        let root = PathBuf::from("/private/tmp")
            .join(format!("clamav-update-ipc-{}", uuid::Uuid::new_v4()));
        let core = Arc::new(Core::new(root.clone()).unwrap());
        let server = Server::bind(core.clone()).unwrap();
        // A harmless local child exercises the production job ownership/cancellation path.
        // Never contact the FreshClam CDN in automated tests.
        server
            .state
            .start_definition_job(|core| async move {
                let outcome = crate::process::run(
                    Path::new("/bin/sleep"),
                    &["30".into()],
                    Some(&core.definitions_cancel),
                    Duration::from_secs(40),
                    |_| {},
                )
                .await
                .unwrap();
                assert!(outcome.cancelled);
                core.document.lock().unwrap().operation = None;
                crate::diagnostics::record(
                    &core,
                    "definitions_update",
                    &Err::<(), String>("definitions_update_cancelled".into()),
                );
            })
            .await
            .unwrap();
        let task = tokio::spawn(server.run(std::future::pending()));
        let client = Client::new(root.clone()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            match client.request(Command::GetSnapshot).await.unwrap() {
                Payload::Snapshot(snapshot) => {
                    assert_eq!(snapshot.operation.unwrap().kind, "definitions")
                }
                _ => panic!("snapshot expected"),
            }
            client
                .request(Command::SaveSettings {
                    settings: Settings {
                        language: "en".into(),
                        ..Settings::default()
                    },
                })
                .await
                .unwrap();
            client.request(Command::StopScan).await.unwrap();
            let request = Request::new(Command::UpdateDefinitions);
            assert_eq!(
                client
                    .send(&request)
                    .await
                    .unwrap()
                    .into_result()
                    .err()
                    .unwrap(),
                "busy"
            );
            // Busy failures must also be persisted, not left as an unknown outcome.
            assert_eq!(
                client
                    .send(&request)
                    .await
                    .unwrap()
                    .into_result()
                    .err()
                    .unwrap(),
                "busy"
            );
            client.request(Command::Shutdown).await.unwrap();
            task.await.unwrap().unwrap();
        })
        .await
        .expect("updates blocked IPC or failed to reap child on shutdown");
        assert_eq!(core.snapshot().settings.language, "en");
        assert!(core.snapshot().operation.is_none());
        assert!(core
            .snapshot()
            .diagnostics
            .iter()
            .any(|a| a.kind == "definitions_update"
                && a.status == "failed"
                && a.code == "cancelled"));
        drop(core);
        assert!(!root.join("worker.sock").exists());
        let reopened = Core::new(root.clone()).unwrap();
        assert!(reopened
            .snapshot()
            .diagnostics
            .iter()
            .any(|a| a.kind == "definitions_update"));
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn silent_peer_read_times_out() {
        let (mut reader, _peer) = UnixStream::pair().unwrap();
        assert_eq!(
            read_frame(&mut reader, REQUEST_LIMIT).await.err().unwrap(),
            "ipc_timeout"
        );
    }
    #[test]
    fn peer_rejection_and_protocol_validation() {
        assert!(same_user(100, 100).is_ok());
        assert_eq!(same_user(101, 100).unwrap_err(), "ipc_peer_rejected");
        let mut request = Request::new(Command::StopScan);
        request.version += 1;
        assert_eq!(request.validate().unwrap_err(), "ipc_version_mismatch");
        request.version = VERSION;
        request.issued_at = "2000-01-01T00:00:00Z".into();
        assert_eq!(request.validate().unwrap_err(), "ipc_request_expired");
        assert!(socket_path(&PathBuf::from(format!("/{}", "x".repeat(200)))).is_err());
        assert!(encode(&"x".repeat(100), 10).is_err());
        assert_eq!(
            Request::new(Command::StartScan {
                kind: "custom".into(),
                target: Some("relative".into())
            })
            .validate()
            .unwrap_err(),
            "ipc_invalid_scan_target"
        );
        assert_eq!(
            Request::new(Command::StartScan {
                kind: "quick".into(),
                target: Some("/tmp".into())
            })
            .validate()
            .unwrap_err(),
            "ipc_invalid_scan_target"
        );
        assert!(
            serde_json::from_str::<Command>(r#"{"command":"run_shell","script":"true"}"#).is_err()
        );
    }
}
