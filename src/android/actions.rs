//! One-shot commands behind the Settings and Tools tabs, plus reading the
//! current settings so the Settings tab can show them.

use std::collections::HashMap;
use std::time::Duration;

use super::Connection;
use super::changes::{self, Outcome};
use super::sysinfo::split_sections;
use crate::journal::Change;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    // Settings
    Rotation(&'static str),
    ScreenTimeout(&'static str),
    Density(u32),
    ResetDensity,
    DeveloperOptions(bool),
    Location(bool),
    PlayProtect(bool),
    AmbientDisplay(bool),
    RepairNtp,
    // Tools
    Wake,
    Sleep,
    Reboot,
    RebootRecovery,
    Notifications,
    SearchWeather,
    SystemUpdates,
    OpenSettings(&'static str),
    LaunchApp(String),
    SendText(String),
    /// A raw shell command typed by the user.
    Console(String),
}

/// Screen rotation values (`user_rotation`) with labels.
pub const ROTATIONS: [(&str, &str); 4] =
    [("Normal (0°)", "0"), ("Rotated 90°", "1"), ("Upside down (180°)", "2"), ("Rotated 270°", "3")];

/// Screen timeout values in ms (`screen_off_timeout`) with labels.
pub const TIMEOUTS: [(&str, &str); 6] = [
    ("30 seconds", "30000"),
    ("1 minute", "60000"),
    ("2 minutes", "120000"),
    ("5 minutes", "300000"),
    ("10 minutes", "600000"),
    ("Never", "2147483647"),
];

/// Settings screens the TV can open directly.
pub const SETTINGS_SHORTCUTS: [(&str, &str); 5] = [
    ("Display Settings", "android.settings.DISPLAY_SETTINGS"),
    ("Wi-Fi Settings", "android.settings.WIFI_SETTINGS"),
    ("Bluetooth Settings", "android.settings.BLUETOOTH_SETTINGS"),
    ("App Settings", "android.settings.MANAGE_APPLICATIONS_SETTINGS"),
    ("Developer Options", "android.settings.APPLICATION_DEVELOPMENT_SETTINGS"),
];

/// Escape text for `input text`: spaces become %s, shell specials get a
/// backslash (the command runs through the TV's shell).
pub fn escape_input_text(text: &str) -> String {
    const SPECIAL: &str = "\\\"`$&|;()<>!#*?[]{}^~'";
    let mut out = String::with_capacity(text.len() * 2);
    for ch in text.chars() {
        match ch {
            ' ' => out.push_str("%s"),
            c if SPECIAL.contains(c) => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

/// `com.example.app`: letters, digits, `_` and dots only, so it's safe to
/// put in a shell command.
pub fn is_package_name(s: &str) -> bool {
    !s.is_empty() && s.contains('.') && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
}

/// Accept what people paste from guides: `adb shell cmd…` or `adb cmd…`.
pub fn console_command(raw: &str) -> String {
    let t = raw.trim();
    for prefix in ["adb shell ", "adb "] {
        if t.len() > prefix.len() && t[..prefix.len()].eq_ignore_ascii_case(prefix) {
            return t[prefix.len()..].trim().to_owned();
        }
    }
    t.to_owned()
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

impl Action {
    fn command(&self) -> String {
        match self {
            Action::Rotation(v) => format!("settings put system user_rotation {v}"),
            Action::ScreenTimeout(ms) => format!("settings put system screen_off_timeout {ms}"),
            Action::Density(d) => format!("wm density {d}"),
            Action::ResetDensity => "wm density reset".into(),
            Action::DeveloperOptions(on) => format!("settings put global development_settings_enabled {}", u8::from(*on)),
            Action::Location(on) => format!("settings put secure location_mode {}", if *on { 3 } else { 0 }),
            Action::PlayProtect(on) => format!("settings put global package_verifier_enable {}", u8::from(*on)),
            Action::AmbientDisplay(on) => format!("settings put secure doze_enabled {}", u8::from(*on)),
            Action::RepairNtp => {
                "settings put global ntp_server pool.ntp.org; am broadcast -a android.intent.action.TIME_TICK".into()
            }
            Action::Wake => "input keyevent 224".into(),
            Action::Sleep => "input keyevent 223".into(),
            Action::Reboot => "reboot".into(),
            Action::RebootRecovery => "reboot recovery".into(),
            Action::Notifications => "cmd statusbar expand-notifications".into(),
            Action::SearchWeather => {
                "am start -a android.search.action.GLOBAL_SEARCH --es query 'What is the weather like today'".into()
            }
            Action::SystemUpdates => "am start -a android.settings.SYSTEM_UPDATE_SETTINGS".into(),
            Action::OpenSettings(a) => format!("am start -a {a}"),
            // TV-only apps register LEANBACK_LAUNCHER instead of LAUNCHER;
            // try that first, then the phone launcher.
            Action::LaunchApp(pkg) if is_package_name(pkg) => format!(
                "out=$(monkey -p {pkg} -c android.intent.category.LEANBACK_LAUNCHER 1 2>&1); \
                 case \"$out\" in *'No activities'*) monkey -p {pkg} -c android.intent.category.LAUNCHER 1 2>&1;; \
                 *) echo \"$out\";; esac"
            ),
            Action::LaunchApp(_) => String::new(),
            Action::SendText(t) => format!("input text {}", escape_input_text(t)),
            Action::Console(c) => console_command(c),
        }
    }

    /// What to log when it worked and printed nothing.
    fn done(&self) -> String {
        match self {
            Action::Rotation(v) => format!("Screen rotation set to {v}."),
            Action::ScreenTimeout(ms) => format!("Screen timeout set to {ms} ms."),
            Action::Density(d) => format!("Display density set to {d}."),
            Action::ResetDensity => "Display density reset.".into(),
            Action::DeveloperOptions(on) => format!("Developer options {}.", if *on { "shown" } else { "hidden" }),
            Action::Location(on) => format!("Location {}.", on_off(*on)),
            Action::PlayProtect(on) => format!("Play Protect {}.", on_off(*on)),
            Action::AmbientDisplay(on) => format!("Ambient display {}.", on_off(*on)),
            Action::RepairNtp => "Time server set to pool.ntp.org.".into(),
            Action::Wake => "Wake sent.".into(),
            Action::Sleep => "Sleep sent.".into(),
            Action::Reboot | Action::RebootRecovery => "Rebooting; the connection will drop.".into(),
            Action::Notifications => "Notifications opened.".into(),
            Action::SearchWeather => "Opened Google search.".into(),
            Action::SystemUpdates => "Opened system updates.".into(),
            Action::OpenSettings(a) => format!("Opened {a}."),
            Action::LaunchApp(p) => format!("Launched {p}."),
            Action::SendText(t) => format!("Sent text: {t}"),
            Action::Console(_) => "(no output)".into(),
        }
    }

    pub fn confirmation(&self) -> Option<(String, String)> {
        match self {
            Action::Reboot => Some(("Reboot".into(), "Restart the TV now?".into())),
            Action::RebootRecovery => Some((
                "Reboot to Recovery".into(),
                "Restart the TV into recovery mode?\n\nYou'll need the TV's remote to get back out \
                 (usually 'Reboot system now'). The app loses its connection until then."
                    .into(),
            )),
            Action::PlayProtect(false) => Some((
                "Disable Play Protect".into(),
                "Turn off Play Protect's app scanning?\n\nThe TV will no longer check installed apps for \
                 harmful behaviour."
                    .into(),
            )),
            _ => None,
        }
    }

    /// Settings actions as `(namespace, key, value)`, so they go through
    /// [`changes::put_setting`] and can be undone.
    fn setting(&self) -> Option<(&'static str, &'static str, String)> {
        let flag = |on: &bool| u8::from(*on).to_string();
        Some(match self {
            Action::Rotation(v) => ("system", "user_rotation", (*v).to_owned()),
            Action::ScreenTimeout(ms) => ("system", "screen_off_timeout", (*ms).to_owned()),
            Action::DeveloperOptions(on) => ("global", "development_settings_enabled", flag(on)),
            Action::Location(on) => ("secure", "location_mode", if *on { "3" } else { "0" }.to_owned()),
            Action::PlayProtect(on) => ("global", "package_verifier_enable", flag(on)),
            Action::AmbientDisplay(on) => ("secure", "doze_enabled", flag(on)),
            Action::RepairNtp => ("global", "ntp_server", "pool.ntp.org".to_owned()),
            _ => return None,
        })
    }

    pub fn is_reboot(&self) -> bool {
        matches!(self, Action::Reboot | Action::RebootRecovery)
    }
}

/// Run one action: a line (or the command's output) for the log, and the
/// changes made.
pub async fn run(conn: Connection, action: Action) -> Result<Outcome, String> {
    if let Some((namespace, key, value)) = action.setting() {
        let change = changes::put_setting(&conn, namespace, key, Some(&value)).await?;
        if action == Action::RepairNtp {
            let _ = conn.shell("am broadcast -a android.intent.action.TIME_TICK", Duration::from_secs(15)).await;
        }
        return Ok(Outcome { lines: vec![action.done()], changes: vec![change] });
    }
    match action {
        Action::Density(d) => {
            let change = changes::set_density(&conn, Some(d)).await?;
            return Ok(Outcome { lines: vec![action.done()], changes: vec![change] });
        }
        Action::ResetDensity => {
            let change = changes::set_density(&conn, None).await?;
            return Ok(Outcome { lines: vec![action.done()], changes: vec![change] });
        }
        _ => {}
    }
    let note = match &action {
        Action::Console(c) => Some(Change::Note { text: format!("Ran on the TV: {}", console_command(c)) }),
        _ => None,
    };
    let line = run_command(conn, action).await?;
    Ok(Outcome { lines: vec![line], changes: note.into_iter().collect() })
}

/// Actions that don't change a setting, run as their command.
async fn run_command(conn: Connection, action: Action) -> Result<String, String> {
    let cmd = action.command();
    if cmd.is_empty() {
        return Err(match action {
            Action::LaunchApp(p) => format!("'{p}' isn't a package name (like com.example.app)"),
            _ => "nothing to run".into(),
        });
    }
    let secs = if matches!(action, Action::Console(_)) { 60 } else { 20 };
    match conn.shell(cmd, Duration::from_secs(secs)).await {
        Ok(out) if out.is_empty() => Ok(action.done()),
        Ok(out) => Ok(out),
        // A reboot drops the connection mid-command; that's success.
        Err(_) if action.is_reboot() => Ok(action.done()),
        Err(e) => Err(e.to_string()),
    }
}

// ── current settings ─────────────────────────────────────────────────────────

const READ: &str = "echo @@rotation; settings get system user_rotation; \
    echo @@timeout; settings get system screen_off_timeout; \
    echo @@density; wm density; \
    echo @@dev; settings get global development_settings_enabled; \
    echo @@location; settings get secure location_mode; \
    echo @@verifier; settings get global package_verifier_enable; \
    echo @@doze; settings get secure doze_enabled; \
    echo @@ntp; settings get global ntp_server";

/// Current values, already phrased for display.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Current {
    pub rotation: String,
    pub timeout: String,
    pub density: String,
    pub developer_options: String,
    pub location: String,
    pub play_protect: String,
    pub ambient: String,
    pub ntp: String,
}

pub fn parse_current(sec: &HashMap<String, String>) -> Current {
    let v = |k: &str| sec.get(k).map(|s| s.trim()).filter(|s| !s.is_empty() && *s != "null").unwrap_or("");
    let flag = |k: &str, default_on: bool| match v(k) {
        "1" => "on",
        "0" => "off",
        _ if default_on => "on (default)",
        _ => "off (default)",
    };
    let label = |list: &[(&str, &str)], value: &str| {
        list.iter().find(|(_, x)| *x == value).map(|(l, _)| (*l).to_owned()).unwrap_or_else(|| value.to_owned())
    };
    let timeout = v("timeout");
    let density = super::sysinfo::parse_density(sec.get("density").map(String::as_str).unwrap_or(""));
    Current {
        rotation: if v("rotation").is_empty() { "Normal (0°)".into() } else { label(&ROTATIONS, v("rotation")) },
        timeout: match timeout.parse::<u64>() {
            Ok(_) if TIMEOUTS.iter().any(|(_, x)| *x == timeout) => label(&TIMEOUTS, timeout),
            Ok(ms) => format!("{} s", ms / 1000),
            Err(_) => "—".into(),
        },
        density,
        developer_options: flag("dev", false).into(),
        location: match v("location") {
            "0" => "off".into(),
            "" => "—".into(),
            _ => "on".into(),
        },
        play_protect: flag("verifier", true).into(),
        ambient: flag("doze", false).into(),
        ntp: if v("ntp").is_empty() { "system default".into() } else { v("ntp").to_owned() },
    }
}

pub async fn read_current(conn: Connection) -> Result<Current, String> {
    let raw = conn.shell(READ, Duration::from_secs(20)).await.map_err(|e| e.to_string())?;
    Ok(parse_current(&split_sections(&raw)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads settings and lists /sdcard on a real TV (changes nothing):
    /// `STT_TEST_TV=ip:port cargo test live_settings_and_files -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_settings_and_files() {
        let serial = std::env::var("STT_TEST_TV").expect("set STT_TEST_TV=ip:port");
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let conn = crate::android::transport::connect(serial).await.expect("connect");
            println!("{:#?}", read_current(conn.clone()).await.expect("settings"));
            let list = crate::android::files::list(conn, "/sdcard/".into()).await.expect("list");
            let names: Vec<String> = list.iter().map(|e| if e.is_dir { format!("{}/", e.name) } else { e.name.clone() }).collect();
            println!("/sdcard/: {}", names.join("  "));
        });
    }

    #[test]
    fn text_escaping_matches_the_python_version() {
        assert_eq!(escape_input_text("hello world"), "hello%sworld");
        assert_eq!(escape_input_text("a&b;c'd"), "a\\&b\\;c\\'d");
        assert_eq!(escape_input_text("$(rm)"), "\\$\\(rm\\)");
    }

    #[test]
    fn package_names_are_validated() {
        assert!(is_package_name("com.netflix.ninja"));
        assert!(is_package_name("org.drive_hunter"));
        assert!(!is_package_name("netflix"));
        assert!(!is_package_name("com.x; reboot"));
        assert_eq!(Action::LaunchApp("x;y".into()).command(), "");
    }

    #[test]
    fn console_strips_adb_prefixes() {
        assert_eq!(console_command("adb shell input keyevent 3"), "input keyevent 3");
        assert_eq!(console_command("ADB reboot"), "reboot");
        assert_eq!(console_command("  pm list packages -3 "), "pm list packages -3");
        assert_eq!(console_command("adb"), "adb");
    }

    #[test]
    fn reads_current_settings_like_the_sony() {
        let sec = split_sections(
            "@@rotation\n0\n@@timeout\n600000\n@@density\nPhysical density: 320\n@@dev\n1\n\
             @@location\n3\n@@verifier\n1\n@@doze\nnull\n@@ntp\nnull\n",
        );
        let c = parse_current(&sec);
        assert_eq!(c.rotation, "Normal (0°)");
        assert_eq!(c.timeout, "10 minutes");
        assert_eq!(c.density, "320 dpi");
        assert_eq!(c.developer_options, "on");
        assert_eq!(c.location, "on");
        assert_eq!(c.play_protect, "on");
        assert_eq!(c.ambient, "off (default)");
        assert_eq!(c.ntp, "system default");
        assert_eq!(parse_current(&split_sections("@@timeout\n45000\n")).timeout, "45 s");
    }
}
