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
    SaveSettings {
        settings: Settings,
    },
    Shutdown,
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
    ScanStarted { scan_id: String },
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
struct ServiceState {
    core: Arc<Core>,
    commands: Mutex<()>,
    stopping: AtomicBool,
    shutdown: Notify,
}
impl ServiceState {
    async fn dispatch(&self, request: &Request) -> Response {
        let result = self.execute(request).await;
        Response::new(&request.id, result)
    }
    async fn execute(&self, request: &Request) -> Result<Payload> {
        request.validate()?;
        if matches!(request.body, Command::GetSnapshot) {
            return Ok(Payload::Snapshot(Box::new(self.core.snapshot())));
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
            Command::StopScan => {
                self.core.cancel.store(true, Ordering::Relaxed);
                Ok(Payload::Ack)
            }
            Command::Shutdown => {
                self.stopping.store(true, Ordering::Release);
                self.core.cancel.store(true, Ordering::Relaxed);
                Ok(Payload::Ack)
            }
            Command::GetSnapshot => unreachable!(),
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
            }),
            _lease: lease,
        })
    }
    pub fn path(&self) -> &Path {
        &self.socket.path
    }
    pub async fn run(self, stop: impl Future<Output = ()>) -> Result<()> {
        tokio::pin!(stop);
        let permits = Arc::new(Semaphore::new(8));
        let mut tasks = JoinSet::new();
        loop {
            tokio::select! {
                _ = &mut stop => break,
                _ = self.state.shutdown.notified() => break,
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
        // Existing replies finish before socket removal. Slow readers are bounded by IO_TIMEOUT.
        while tasks.join_next().await.is_some() {}
        self.state.core.cancel.store(true, Ordering::Relaxed);
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
