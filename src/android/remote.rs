//! Remote control (key events) and screenshots.

use std::time::Duration;

use super::Connection;

/// A remote button: label and Android key code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub label: &'static str,
    pub code: u16,
}

const fn key(label: &'static str, code: u16) -> Key {
    Key { label, code }
}

pub const UP: Key = key("▲", 19);
pub const DOWN: Key = key("▼", 20);
pub const LEFT: Key = key("◀", 21);
pub const RIGHT: Key = key("▶", 22);
pub const OK: Key = key("OK", 23);
pub const BACK: Key = key("Back", 4);
pub const HOME: Key = key("Home", 3);
pub const MENU: Key = key("Menu", 82);
pub const VOLUME_UP: Key = key("Vol +", 24);
pub const VOLUME_DOWN: Key = key("Vol −", 25);
pub const MUTE: Key = key("Mute", 164);
pub const PREVIOUS: Key = key("⏮", 88);
pub const PLAY_PAUSE: Key = key("⏯", 85);
pub const NEXT: Key = key("⏭", 87);
pub const POWER: Key = key("Power", 26);

pub async fn press(conn: Connection, key: Key) -> Result<(), String> {
    conn.shell(format!("input keyevent {}", key.code), Duration::from_secs(10))
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Capture the screen as PNG. Content protected by DRM (most streaming
/// apps) comes out black; that's the TV's doing, not a bug.
pub async fn screenshot(conn: Connection) -> Result<Vec<u8>, String> {
    // /data/local/tmp is writable by the ADB shell on every TV, unlike /sdcard.
    const PATH: &str = "/data/local/tmp/stt-screenshot.png";
    let out = conn.shell(format!("screencap -p {PATH}"), Duration::from_secs(20)).await.map_err(|e| e.to_string())?;
    let bytes = conn.pull_bytes(PATH.into(), Duration::from_secs(60)).await;
    let _ = conn.shell(format!("rm -f {PATH}"), Duration::from_secs(10)).await;
    let bytes = bytes.map_err(|e| e.to_string())?;
    if bytes.is_empty() {
        // screencap exits 1 with no message when the screen is protected.
        return Err("the TV refused the capture. This happens while protected content is on screen \
                    (live TV, HDMI inputs, most streaming apps); try from the home screen or another app."
            .into());
    }
    if !bytes.starts_with(b"\x89PNG") {
        return Err(if out.is_empty() { "the TV returned something that isn't an image".into() } else { out });
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captures one screenshot from a real TV (reads only):
    /// `STT_TEST_TV=ip:port STT_TEST_OUT=shot.png cargo test live_screenshot -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_screenshot() {
        let serial = std::env::var("STT_TEST_TV").expect("set STT_TEST_TV=ip:port");
        let out = std::env::var("STT_TEST_OUT").expect("set STT_TEST_OUT");
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let conn = crate::android::transport::connect(serial).await.expect("connect");
            let t = std::time::Instant::now();
            let png = screenshot(conn).await.expect("screenshot");
            println!("{} bytes in {:?}", png.len(), t.elapsed());
            std::fs::write(out, png).unwrap();
        });
    }
}
