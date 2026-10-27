//! Settings worth knowing by name: friendly titles, what they do and which
//! values make sense, so the settings browser can offer a picker instead of
//! a bare text field. Anything not listed is still shown and editable.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `1` on, `0` off.
    Switch,
    /// Fixed choices: (label, value).
    Choice(&'static [(&'static str, &'static str)]),
    /// A whole number, with its unit for display.
    Number(&'static str),
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Known {
    pub namespace: &'static str,
    pub key: &'static str,
    pub name: &'static str,
    pub about: &'static str,
    pub kind: Kind,
    /// Changing it can cut the ADB connection or confuse setup.
    pub risky: bool,
}

const SCALES: &[(&str, &str)] = &[("Off", "0"), ("0.5×", "0.5"), ("1× (default)", "1"), ("1.5×", "1.5"), ("2×", "2")];

const fn k(namespace: &'static str, key: &'static str, name: &'static str, about: &'static str, kind: Kind) -> Known {
    Known { namespace, key, name, about, kind, risky: false }
}

const fn risky(namespace: &'static str, key: &'static str, name: &'static str, about: &'static str, kind: Kind) -> Known {
    Known { namespace, key, name, about, kind, risky: true }
}

pub const KNOWN: &[Known] = &[
    // ── Speed & animations ──
    k("global", "window_animation_scale", "Window animation speed", "How fast windows open and close. Lower feels snappier.", Kind::Choice(SCALES)),
    k("global", "transition_animation_scale", "Transition animation speed", "Animations when moving between screens.", Kind::Choice(SCALES)),
    k("global", "animator_duration_scale", "Animator duration", "Length of in-app animations.", Kind::Choice(SCALES)),
    k("global", "cached_apps_freezer", "App freezer", "Freezes apps in the background so they use no CPU.",
      Kind::Choice(&[("Enabled", "enabled"), ("Disabled", "disabled"), ("Device default", "device_default")])),
    k("global", "always_finish_activities", "Don't keep activities", "Closes every screen of an app as soon as you leave it. Saves memory, slows switching.", Kind::Switch),
    // ── Display ──
    k("system", "screen_off_timeout", "Screen timeout", "Idle time before the screen turns off.", Kind::Number("ms")),
    k("secure", "sleep_timeout", "Sleep after inactivity", "Idle time before the TV goes to sleep (-1 = never).", Kind::Number("ms")),
    k("system", "user_rotation", "Screen rotation", "Rotation used when auto-rotate is off.",
      Kind::Choice(&[("Normal (0°)", "0"), ("90°", "1"), ("180°", "2"), ("270°", "3")])),
    k("system", "accelerometer_rotation", "Auto-rotate", "Rotate with the device (rarely used on TVs).", Kind::Switch),
    k("system", "screen_brightness", "Brightness", "Backlight level, 0–255 (many TVs use their own picture settings instead).", Kind::Number("of 255")),
    k("system", "font_scale", "Font size", "Text size across the TV's menus and apps.",
      Kind::Choice(&[("Small", "0.85"), ("Default", "1.0"), ("Large", "1.15"), ("Largest", "1.3")])),
    k("secure", "screensaver_enabled", "Screen saver", "Show the screen saver (Ambient mode) when idle.", Kind::Switch),
    k("secure", "screensaver_activate_on_sleep", "Screen saver on sleep", "Start the screen saver instead of sleeping.", Kind::Switch),
    k("secure", "screensaver_components", "Screen saver app", "The screen saver component to run.", Kind::Text),
    k("secure", "doze_enabled", "Ambient display", "Show information on the screen while idle.", Kind::Switch),
    // ── Sound ──
    k("system", "sound_effects_enabled", "System sounds", "Clicks and navigation sounds in menus.", Kind::Switch),
    // ── Network & time ──
    k("global", "device_name", "Device name", "The name the TV shows to other devices (Cast, Bluetooth).", Kind::Text),
    k("global", "private_dns_mode", "Private DNS", "Encrypted DNS for every app.",
      Kind::Choice(&[("Off", "off"), ("Automatic", "opportunistic"), ("Use provider below", "hostname")])),
    k("global", "private_dns_specifier", "Private DNS provider", "Hostname for Private DNS, e.g. dns.adguard.com.", Kind::Text),
    k("global", "captive_portal_mode", "Captive portal check", "Whether the TV checks for Wi-Fi login pages.",
      Kind::Choice(&[("Don't check", "0"), ("Prompt", "1"), ("Avoid such networks", "2")])),
    k("global", "auto_time", "Automatic date & time", "Take the time from the network.", Kind::Switch),
    k("global", "auto_time_zone", "Automatic time zone", "Take the time zone from the network.", Kind::Switch),
    k("global", "ntp_server", "Time server", "NTP server used for the clock.", Kind::Text),
    k("system", "time_12_24", "Time format", "12- or 24-hour clock.", Kind::Choice(&[("12-hour", "12"), ("24-hour", "24")])),
    // ── HDMI-CEC ──
    k("global", "hdmi_control_enabled", "HDMI-CEC", "Control and be controlled by devices on HDMI.", Kind::Switch),
    k("global", "hdmi_control_auto_wakeup_enabled", "Turn on with HDMI devices", "Wake the TV when a connected device turns on.", Kind::Switch),
    k("global", "hdmi_control_auto_device_off_enabled", "Turn HDMI devices off with TV", "Switch connected devices off when the TV sleeps.", Kind::Switch),
    // ── Apps & security ──
    k("global", "package_verifier_enable", "Play Protect scanning", "Check installed apps for harmful behaviour.", Kind::Switch),
    k("global", "verifier_verify_adb_installs", "Verify ADB installs", "Scan apps installed over ADB before they install.", Kind::Switch),
    k("secure", "install_non_market_apps", "Unknown sources (legacy)", "Allow installing apps from outside the Play Store on old Android versions.", Kind::Switch),
    k("secure", "location_mode", "Location", "Location services for apps.", Kind::Choice(&[("Off", "0"), ("On", "3")])),
    // ── Input & accessibility ──
    k("secure", "default_input_method", "Keyboard", "The on-screen keyboard component in use.", Kind::Text),
    k("secure", "show_ime_with_hard_keyboard", "Keyboard with hardware keys", "Keep the on-screen keyboard while a physical keyboard is connected.", Kind::Switch),
    k("secure", "accessibility_enabled", "Accessibility", "Whether accessibility services run.", Kind::Switch),
    k("secure", "enabled_accessibility_services", "Accessibility services", "Services allowed to read and control the screen (e.g. virtual mouse apps).", Kind::Text),
    k("system", "show_touches", "Show taps", "Show a dot where the screen is touched.", Kind::Switch),
    k("system", "pointer_location", "Pointer location", "Overlay showing pointer coordinates.", Kind::Switch),
    // ── Developer ──
    risky("global", "development_settings_enabled", "Developer options", "Show the developer menu. Turning it off also turns off ADB debugging.", Kind::Switch),
    risky("global", "adb_enabled", "ADB debugging", "Turning this off disconnects this app.", Kind::Switch),
    risky("global", "adb_wifi_enabled", "Wireless debugging", "Android 11+ wireless ADB. Turning it off disconnects this app.", Kind::Switch),
    k("global", "stay_on_while_plugged_in", "Stay awake", "Keep the screen on while powered (bit mask: 1 AC, 2 USB, 4 wireless; 7 = always).",
      Kind::Choice(&[("Off", "0"), ("On AC power", "1"), ("Always", "7")])),
];

pub fn find(namespace: &str, key: &str) -> Option<&'static Known> {
    KNOWN.iter().find(|k| k.namespace == namespace && k.key == key)
}

/// Keys that cut the connection or break setup when changed, known or not.
pub fn is_risky(namespace: &str, key: &str) -> bool {
    find(namespace, key).is_some_and(|k| k.risky)
        || key.starts_with("adb_")
        || matches!(key, "device_provisioned" | "user_setup_complete" | "tv_user_setup_complete")
}

/// A value as people read it, e.g. `0.5` → `0.5×`, `1` → `On`.
pub fn describe(known: Option<&Known>, value: Option<&str>) -> String {
    let Some(v) = value else { return crate::i18n::tr("not set").into() };
    match known.map(|k| k.kind) {
        Some(Kind::Switch) => match v {
            "1" => crate::i18n::tr("On").into(),
            "0" => crate::i18n::tr("Off").into(),
            other => other.into(),
        },
        Some(Kind::Choice(options)) => {
            options.iter().find(|(_, x)| *x == v).map_or_else(|| v.to_owned(), |(l, _)| crate::i18n::tr(l).to_owned())
        }
        Some(Kind::Number(unit)) => format!("{v} {unit}"),
        _ => v.to_owned(),
    }
}

/// Settings screens the TV can open directly: (title, intent action).
pub const SCREENS: &[(&str, &str)] = &[
    ("All Settings", "android.settings.SETTINGS"),
    ("Display", "android.settings.DISPLAY_SETTINGS"),
    ("Sound", "android.settings.SOUND_SETTINGS"),
    ("Network & Wi-Fi", "android.settings.WIFI_SETTINGS"),
    ("Bluetooth & Remotes", "android.settings.BLUETOOTH_SETTINGS"),
    ("Apps", "android.settings.MANAGE_APPLICATIONS_SETTINGS"),
    ("Default Apps", "android.settings.MANAGE_DEFAULT_APPS_SETTINGS"),
    ("Storage", "android.settings.INTERNAL_STORAGE_SETTINGS"),
    ("Date & Time", "android.settings.DATE_SETTINGS"),
    ("Language", "android.settings.LOCALE_SETTINGS"),
    ("Keyboard", "android.settings.INPUT_METHOD_SETTINGS"),
    ("Accessibility", "android.settings.ACCESSIBILITY_SETTINGS"),
    ("Captions", "android.settings.CAPTIONING_SETTINGS"),
    ("Screen Saver", "android.settings.DREAM_SETTINGS"),
    ("Security & Restrictions", "android.settings.SECURITY_SETTINGS"),
    ("Location", "android.settings.LOCATION_SOURCE_SETTINGS"),
    ("Accounts", "android.settings.SYNC_SETTINGS"),
    ("About / Device Info", "android.settings.DEVICE_INFO_SETTINGS"),
    ("System Update", "android.settings.SYSTEM_UPDATE_SETTINGS"),
    ("Developer Options", "android.settings.APPLICATION_DEVELOPMENT_SETTINGS"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_consistent() {
        for k in KNOWN {
            assert!(crate::android::changes::NAMESPACES.contains(&k.namespace), "{}", k.key);
            assert!(crate::android::changes::is_setting_key(k.key), "{}", k.key);
            assert_eq!(KNOWN.iter().filter(|o| o.namespace == k.namespace && o.key == k.key).count(), 1, "{}", k.key);
        }
        assert!(is_risky("global", "adb_enabled"));
        assert!(is_risky("global", "adb_something_new"));
        assert!(!is_risky("global", "window_animation_scale"));
        let scale = find("global", "window_animation_scale");
        assert_eq!(describe(scale, Some("0.5")), "0.5×");
        assert_eq!(describe(find("system", "sound_effects_enabled"), Some("0")), "Off");
        assert_eq!(describe(None, None), "not set");
    }
}
