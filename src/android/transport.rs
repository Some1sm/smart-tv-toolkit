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
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adb_client::{ADBDeviceExt, RustADBError, tcp::ADBTcpDevice};

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

impl Connection {
    /// Run a shell command and return stdout and stderr, decoded lossily
    /// (device output is not always valid UTF-8).
    pub async fn shell(&self, command: impl Into<String>, timeout: Duration) -> Result<String, Error> {
        let device = self.device.clone();
        let command = command.into();
        let task = tokio::task::spawn_blocking(move || {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let mut device = device.lock().map_err(|_| Error::Adb("connection lost".into()))?;
            device
                .shell_command(&command, Some(&mut stdout), Some(&mut stderr))
                .map_err(adb_error)?;
            let mut text = String::from_utf8_lossy(&stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&stderr));
            Ok(text.trim().to_owned())
        });
        match tokio::time::timeout(timeout, task).await {
            Ok(Ok(result)) => result,
            Ok(Err(join)) => Err(Error::Adb(join.to_string())),
            Err(_) => Err(Error::Timeout),
        }
    }

    /// Copy a file from the TV to `local`. A partial file is removed on failure.
    pub async fn pull(&self, remote: String, local: PathBuf, timeout: Duration) -> Result<u64, Error> {
        let device = self.device.clone();
        let task = tokio::task::spawn_blocking(move || {
            let mut file = std::fs::File::create(&local).map_err(|e| Error::Adb(e.to_string()))?;
            let mut device = device.lock().map_err(|_| Error::Adb("connection lost".into()))?;
            let result = device.pull(&remote, &mut file).map_err(adb_error);
            drop(file);
            if result.is_err() {
                let _ = std::fs::remove_file(&local);
            }
            result?;
            Ok(std::fs::metadata(&local).map(|m| m.len()).unwrap_or(0))
        });
        match tokio::time::timeout(timeout, task).await {
            Ok(Ok(result)) => result,
            Ok(Err(join)) => Err(Error::Adb(join.to_string())),
            Err(_) => Err(Error::Timeout),
        }
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
fn ensure_key() -> Result<PathBuf, String> {
    let path = key_path().ok_or("no home directory for the ADB key")?;
    if path.is_file() {
        return Ok(path);
    }
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    let key = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).map_err(|e| e.to_string())?;
    let pem = key.to_pkcs8_pem(LineEnding::LF).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, pem.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path)
}
