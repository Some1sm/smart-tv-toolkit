//! Typing any text on the TV, including accents and non-Latin scripts.
//!
//! Android's `input text` only handles ASCII. For anything else this uses
//! ADBKeyboard: switch to it, send the text base64-encoded in a broadcast,
//! then switch back to the keyboard the TV had before.

use std::time::Duration;

use base64::Engine;

use super::{Connection, install, shell_quote};

pub const PACKAGE: &str = "com.android.adbkeyboard";
const IME: &str = "com.android.adbkeyboard/.AdbIME";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// ADBKeyboard isn't installed; offer to install it.
    NotInstalled,
    Failed(String),
}

/// Whether `text` needs ADBKeyboard (anything `input text` can't type).
pub fn needs_keyboard(text: &str) -> bool {
    !text.chars().all(|c| c.is_ascii() && !c.is_ascii_control())
}

async fn sh(conn: &Connection, cmd: String) -> Result<String, Error> {
    conn.shell(cmd, Duration::from_secs(15)).await.map_err(|e| Error::Failed(e.to_string()))
}

/// Type `text` into the focused field through ADBKeyboard.
pub async fn send(conn: Connection, text: String) -> Result<String, Error> {
    if !install::is_installed(&conn, PACKAGE).await {
        return Err(Error::NotInstalled);
    }
    let previous = sh(&conn, "settings get secure default_input_method".into()).await?.trim().to_owned();
    sh(&conn, format!("ime enable {IME}; ime set {IME}")).await?;
    // The focused field reconnects to the new keyboard before it can type.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    let out = sh(&conn, format!("am broadcast -a ADB_INPUT_B64 --es msg {b64}")).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    if !previous.is_empty() && previous != "null" && previous != IME {
        let _ = sh(&conn, format!("ime set {}", shell_quote(&previous))).await;
    }
    out?;
    Ok(format!("Sent text: {text}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_non_ascii_needs_the_keyboard() {
        assert!(!needs_keyboard("hello world 123 !?"));
        assert!(needs_keyboard("Televisión"));
        assert!(needs_keyboard("año"));
        assert!(needs_keyboard("你好"));
        assert!(needs_keyboard("tab\there"));
    }
}
