//! Android 11+ "Wireless debugging" pairing (`adb pair host:port code`).
//!
//! Pairing puts our key into the TV's trusted list over a one-off pairing
//! port. Afterwards the TV accepts the key on its separate *connect* port,
//! which `transport::connect` reaches like any other (the TLS upgrade there
//! is handled by adb_client). The protocol itself comes from `libadb`.

use std::time::Duration;

use libadb::keys::rsa::rand_core::OsRng;
use libadb::pairing::{PairingError, pair as libadb_pair};
use libadb::tls::{TlsClientConfig, TlsIdentity};
use libadb::transport::tls::MaybeTls;
use libadb::AdbKey;

use super::transport;

/// Shown to the TV as the key's owner, like adb's `user@host`.
const KEY_NAME: &str = "smart-tv-toolkit@pc";
const TIMEOUT: Duration = Duration::from_secs(30);

/// A pairing code is six digits. Checked before contacting the TV: it
/// counts attempts and stops pairing after twenty.
pub fn valid_code(code: &str) -> bool {
    code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit())
}

/// Pair with the TV's pairing port (`ip:port` from its "Pair device with
/// pairing code" screen). Returns the TV's id on success.
pub async fn pair(address: String, code: String) -> Result<String, String> {
    let code = code.trim().to_owned();
    if !valid_code(&code) {
        return Err("the pairing code is the six digits shown on the TV".into());
    }
    let (host, port) = address.trim().rsplit_once(':').ok_or("enter the pairing address as IP:port")?;
    let port: u16 = port.parse().map_err(|_| "the pairing port is a number".to_owned())?;
    let host = host.trim_matches(['[', ']']).to_owned();

    let key_path = transport::ensure_key()?;
    let pem = std::fs::read_to_string(&key_path).map_err(|e| format!("can't read {}: {e}", key_path.display()))?;
    let key = AdbKey::from_pkcs8_pem(&pem, &mut OsRng, KEY_NAME).map_err(|e| format!("can't load the ADB key: {e}"))?;
    let identity = TlsIdentity::from_key(&key, &mut OsRng).map_err(|e| e.to_string())?;
    let tls = TlsClientConfig::adb(&identity).map_err(|e| e.to_string())?;

    let run = async {
        let socket = <libadb::transport::runtime::Tokio as libadb::transport::runtime::Runtime>::connect_tcp(&host, port)
            .await
            .map_err(|e| format!("can't reach {host}:{port}: {e}"))?;
        let mut transport = MaybeTls::plain(socket);
        match libadb_pair(&mut transport, &tls, &key, &code, &mut OsRng).await {
            Ok(paired) => Ok(paired.guid),
            Err(PairingError::WrongCode) => Err("the code didn't match. Check it, or get a new one on the TV".into()),
            Err(PairingError::Closed) => Err("the TV closed the pairing connection (the code may have expired)".into()),
            Err(e) => Err(e.to_string()),
        }
    };
    tokio::time::timeout(TIMEOUT, run).await.map_err(|_| "pairing timed out".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_six_digits() {
        assert!(valid_code("592781"));
        assert!(!valid_code("59278"));
        assert!(!valid_code("59278a"));
        assert!(!valid_code(" 592781"));
    }
}
