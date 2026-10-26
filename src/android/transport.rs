//! A direct ADB connection to a TV over the network.
//!
//! The ADB protocol is spoken in-process (via `adb_client`), so there is no
//! `adb.exe` and no ADB server. The TV authorises our RSA key once; the key
//! lives at the same place adb uses (`~/.android/adbkey`), so a PC that
//! already used adb with this TV is trusted straight away.
//!
//! Not covered here: pairing for Android 11+ "Wireless debugging", which
//! needs a separate pairing protocol. TVs with classic network debugging
//! (port 5555) work as-is.

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adb_client::{ADBDeviceExt, RustADBError, tcp::ADBTcpDevice};

use crate::jobs::{CountingReader, CountingWriter, Progress, Unit};

/// Default timeout for short commands such as `getprop`.
pub const SHORT: Duration = Duration::from_secs(10);
/// How long a TCP connect may take before the TV counts as unreachable.
const REACH_TIMEOUT: Duration = Duration::from_secs(3);
/// Covers the 10 s the protocol waits for "Allow debugging?" to be accepted.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Nothing answered on that address and port.
    Unreachable(String),
    /// The TV did not accept our key in time (its prompt is probably showing).
    NeedsApproval,
    Timeout,
    Adb(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unreachable(e) => write!(f, "TV not reachable: {e}"),
            Error::NeedsApproval => write!(f, "the TV hasn't allowed debugging from this computer yet"),
            Error::Timeout => write!(f, "the TV stopped responding"),
            Error::Adb(e) => write!(f, "ADB error: {e}"),
        }
    }
}

fn adb_error(e: RustADBError) -> Error {
    match &e {
        RustADBError::IOError(io)
            if matches!(io.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) =>
        {
            Error::NeedsApproval
        }
        _ => Error::Adb(e.to_string()),
    }
}

/// An open connection. Cheap to clone; commands run one at a time.
#[derive(Debug, Clone)]
pub struct Connection {
    device: Arc<Mutex<ADBTcpDevice>>,
}

/// Result of a health check on a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ping {
    Alive,
    /// Another command is still running on this connection.
    Busy,
}

impl Connection {
    /// Run `work` with the device on a blocking thread, giving up after
    /// `timeout`. (A command that never returns keeps its thread and lock;
    /// the health check notices that and reconnects.)
    async fn with_device<T: Send + 'static>(
        &self,
        timeout: Duration,
        work: impl FnOnce(&mut ADBTcpDevice) -> Result<T, Error> + Send + 'static,
    ) -> Result<T, Error> {
        let device = self.device.clone();
        let task = tokio::task::spawn_blocking(move || {
            let mut device = device.lock().map_err(|_| Error::Adb("connection lost".into()))?;
            work(&mut device)
        });
        match tokio::time::timeout(timeout, task).await {
            Ok(Ok(result)) => result,
            Ok(Err(join)) => Err(Error::Adb(join.to_string())),
            Err(_) => Err(Error::Timeout),
        }
    }

    /// Run a shell command and return stdout and stderr, decoded lossily
    /// (device output is not always valid UTF-8).
    pub async fn shell(&self, command: impl Into<String>, timeout: Duration) -> Result<String, Error> {
        let command = command.into();
        self.with_device(timeout, move |device| {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            device.shell_command(&command, Some(&mut stdout), Some(&mut stderr)).map_err(adb_error)?;
            let mut text = String::from_utf8_lossy(&stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&stderr));
            Ok(text.trim().to_owned())
        })
        .await
    }

    /// Is the TV still answering? Doesn't queue behind a running command:
    /// if one holds the connection it reports [`Ping::Busy`] instead.
    pub async fn ping(&self, timeout: Duration) -> Result<Ping, Error> {
        let device = self.device.clone();
        let task = tokio::task::spawn_blocking(move || {
            let mut device = match device.try_lock() {
                Ok(d) => d,
                Err(std::sync::TryLockError::WouldBlock) => return Ok(Ping::Busy),
                Err(_) => return Err(Error::Adb("connection lost".into())),
            };
            let mut out = Vec::new();
            device.shell_command(&"echo ok", Some(&mut out), None).map_err(adb_error)?;
            if String::from_utf8_lossy(&out).contains("ok") { Ok(Ping::Alive) } else { Err(Error::Timeout) }
        });
        match tokio::time::timeout(timeout, task).await {
            Ok(Ok(result)) => result,
            Ok(Err(join)) => Err(Error::Adb(join.to_string())),
            Err(_) => Err(Error::Timeout),
        }
    }

    /// Copy a local file to `remote` on the TV, counting bytes into `progress`.
    pub async fn push(&self, local: PathBuf, remote: String, timeout: Duration, progress: Option<Progress>) -> Result<(), Error> {
        self.with_device(timeout, move |device| {
            let file = std::fs::File::open(&local).map_err(|e| Error::Adb(e.to_string()))?;
            let progress = progress.unwrap_or_else(|| Progress::new(Unit::Bytes));
            progress.set_total(file.metadata().map(|m| m.len()).unwrap_or(0));
            let mut reader = CountingReader { inner: file, progress };
            device.push(&mut reader, &remote).map_err(adb_error)
        })
        .await
    }

    /// Size of a file on the TV, if it exists (for progress bars).
    pub async fn file_size(&self, remote: String) -> Option<u64> {
        let out = self.shell(format!("stat -c %s {}", super::shell_quote(&remote)), SHORT).await.ok()?;
        out.trim().parse().ok()
    }

    /// Read a file from the TV into memory.
    pub async fn pull_bytes(&self, remote: String, timeout: Duration) -> Result<Vec<u8>, Error> {
        self.with_device(timeout, move |device| {
            let mut bytes = Vec::new();
            device.pull(&remote, &mut bytes).map_err(adb_error)?;
            Ok(bytes)
        })
        .await
    }

    /// Copy a file from the TV to `local`, counting bytes into `progress`.
    /// A partial file is removed on failure.
    pub async fn pull(&self, remote: String, local: PathBuf, timeout: Duration, progress: Option<Progress>) -> Result<u64, Error> {
        self.with_device(timeout, move |device| {
            let file = std::fs::File::create(&local).map_err(|e| Error::Adb(e.to_string()))?;
            let progress = progress.unwrap_or_else(|| Progress::new(Unit::Bytes));
            let mut writer = CountingWriter { inner: file, progress };
            let result = device.pull(&remote, &mut writer).map_err(adb_error);
            drop(writer);
            if result.is_err() {
                let _ = std::fs::remove_file(&local);
            }
            result?;
            Ok(std::fs::metadata(&local).map(|m| m.len()).unwrap_or(0))
        })
        .await
    }
}

/// Connect to `host:port`, authorising with our key (created on first use).
pub async fn connect(serial: String) -> Result<Connection, Error> {
    let task = tokio::task::spawn_blocking(move || {
        let addr: SocketAddr = serial
            .to_socket_addrs()
            .map_err(|e| Error::Unreachable(e.to_string()))?
            .next()
            .ok_or_else(|| Error::Unreachable(format!("can't resolve {serial}")))?;
        // Fail fast on a wrong IP instead of waiting for the OS connect timeout.
        TcpStream::connect_timeout(&addr, REACH_TIMEOUT)
            .map_err(|e| Error::Unreachable(e.to_string()))?;
        let key = ensure_key().map_err(Error::Adb)?;
        ADBTcpDevice::new_with_custom_private_key(addr, key).map_err(adb_error)
    });
    match tokio::time::timeout(CONNECT_TIMEOUT, task).await {
        Ok(Ok(Ok(device))) => Ok(Connection { device: Arc::new(Mutex::new(device)) }),
        Ok(Ok(Err(e))) => Err(e),
        Ok(Err(join)) => Err(Error::Adb(join.to_string())),
        Err(_) => Err(Error::NeedsApproval),
    }
}

/// The key path adb itself uses, so keys are shared with any installed adb.
pub fn key_path() -> Option<PathBuf> {
    let dir = match std::env::var_os("ANDROID_USER_HOME") {
        Some(home) => PathBuf::from(home).join("android"),
        None => dirs::home_dir()?.join(".android"),
    };
    Some(dir.join("adbkey"))
}

/// Return the key path, generating and saving a key if there is none.
/// Without a saved key every connection would use a new random key and the
/// TV would ask to allow debugging each time.
pub fn ensure_key() -> Result<PathBuf, String> {
    let path = key_path().ok_or("no home directory for the ADB key")?;
    if path.is_file() {
        make_private(&path);
        return Ok(path);
    }
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    let key = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).map_err(|e| e.to_string())?;
    let pem = key.to_pkcs8_pem(LineEnding::LF).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    write_private(&path, pem.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Write a file only the current user can read: whoever holds the key can
/// control every TV that trusts it. (Windows: the user profile is already
/// private, so a plain write is enough.)
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
        f.write_all(bytes)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes)
    }
}

/// Tighten an existing key that other users could read (e.g. one written
/// by an older version of this app).
fn make_private(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path)
            && meta.permissions().mode() & 0o077 != 0
        {
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}
