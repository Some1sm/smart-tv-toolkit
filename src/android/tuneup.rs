//! Optimize → Recommended: one read-only look at the TV that becomes a short
//! list of safe changes to make together, the TV settings ADB can only point
//! at, and the before/after measurement. Nothing here changes the TV until
//! the user applies the list (a snapshot is saved before that).

use std::collections::BTreeSet;
use std::time::Duration;

use super::changes::Outcome;
use super::health::{self, Fix};
use super::sysinfo::{parse_df, parse_meminfo, split_sections};
use super::{Connection, bloat, telemetry};
use crate::jobs::{Progress, Unit};
use crate::speedtests::Checkup;

const READ: &str = "echo @@system; pm list packages -s; \
    echo @@disabled; pm list packages -d; \
    echo @@global; for k in window_animation_scale transition_animation_scale animator_duration_scale \
    cached_apps_freezer; do echo $k=$(settings get global $k); done; \
    echo @@secure; settings list secure; \
    echo @@home; cmd package resolve-activity --brief -a android.intent.action.MAIN -c android.intent.category.HOME; \
    echo @@df; df -k /data 2>/dev/null; \
    echo @@uptime; cat /proc/uptime; \
    echo @@audio; dumpsys audio 2>/dev/null | grep -m1 'mic mute'; \
    echo @@top; dumpsys activity activities 2>/dev/null | grep -m1 -E 'mResumedActivity|ResumedActivity'";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TvState {
    /// Enabled bloatware that nothing in use depends on: (package, reason).
    pub bloat: Vec<(String, &'static str)>,
    /// Every animation scale at 0.5× or less.
    pub animations_fast: bool,
    pub freezer_on: bool,
    pub uptime_secs: Option<u64>,
    /// The screen saver's package; `None` when it's off.
    pub screensaver: Option<String>,
    /// The screen saver's app is disabled, so Android falls back to its default one.
    pub screensaver_disabled: bool,
    /// Whether the TV's microphone switch mutes it (`None`: not reported).
    pub mic_muted: Option<bool>,
}

/// Packages named in component settings (`pkg/.Class`): the screen saver,
/// keyboard, assistant and so on, which have to stay enabled.
fn in_use(secure: &str) -> BTreeSet<String> {
    secure
        .lines()
        .filter_map(|l| l.split_once('=').map(|(_, v)| v))
        .flat_map(|v| v.split([':', ',']))
        .filter_map(|c| c.split_once('/').map(|(p, _)| p.trim()))
        .filter(|p| p.contains('.') && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_'))
        .map(str::to_owned)
        .collect()
}

pub fn parse(raw: &str) -> TvState {
    let sec = split_sections(raw);
    let get = |k: &str| sec.get(k).map(String::as_str).unwrap_or("");
    let disabled = health::parse_pkg_list(get("disabled"));
    let mut keep = in_use(get("secure"));
    keep.extend(super::optimize::parse_home_activities(get("home")).iter().filter_map(|c| c.split('/').next()).map(str::to_owned));
    keep.insert(health::resumed_package(get("top")));
    let bloat = health::parse_pkg_list(get("system"))
        .into_iter()
        .filter(|p| !disabled.contains(p) && !keep.contains(p))
        .filter_map(|p| bloat::rate(&p, true).filter(|v| v.rating == bloat::Rating::Bloat).map(|v| (p, v.reason)))
        .collect();
    let global = health::parse_settings(get("global"));
    // "null" means never set, i.e. the default 1×.
    let scale = |k: &str| global.get(k).and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
    let secure = health::parse_settings(get("secure"));
    let screensaver = (secure.get("screensaver_enabled").map(String::as_str) != Some("0"))
        .then(|| secure.get("screensaver_components").or_else(|| secure.get("screensaver_default_component")))
        .flatten()
        .and_then(|c| c.split([',', '/']).next())
        .filter(|p| p.contains('.'))
        .map(str::to_owned);
    TvState {
        bloat,
        animations_fast: ["window_animation_scale", "transition_animation_scale", "animator_duration_scale"].iter().all(|k| scale(k) <= 0.5),
        freezer_on: global.get("cached_apps_freezer").is_some_and(|v| v == "enabled"),
        uptime_secs: get("uptime").split_whitespace().next().and_then(|v| v.parse::<f64>().ok()).map(|s| s as u64),
        screensaver_disabled: screensaver.as_ref().is_some_and(|p| disabled.contains(p)),
        screensaver,
        mic_muted: get("audio").split_whitespace().find_map(|w| w.strip_prefix("FromSwitch=")).map(|v| v == "true"),
    }
}

/// Read what the Recommended list and the TV settings need (read-only).
pub async fn read(conn: Connection) -> Result<TvState, String> {
    let raw = conn.shell(READ, Duration::from_secs(60)).await.map_err(|e| e.to_string())?;
    Ok(parse(&raw))
}

// ── Recommended changes ──────────────────────────────────────────────────────

/// One recommended change, in the order they run.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Step {
    Disable(String),
    Animations,
    Freezer,
    TrimCaches,
    Compile,
}

impl Step {
    /// Ticked unless it's slow or noticeable.
    pub fn default_on(&self) -> bool {
        !matches!(self, Step::Compile)
    }

    /// The title, translated.
    pub fn title(&self) -> String {
        match self {
            Step::Disable(p) => crate::i18n::trf("Disable {}", &[&health::app_label(p)]),
            Step::Animations => crate::i18n::tr("Speed up animations (0.5×)").to_owned(),
            Step::Freezer => crate::i18n::tr("Turn on the app freezer").to_owned(),
            Step::TrimCaches => crate::i18n::tr("Trim app caches").to_owned(),
            Step::Compile => crate::i18n::tr("Compile apps for speed").to_owned(),
        }
    }

    /// What it does, translated.
    pub fn detail(&self) -> String {
        let tr = |s: &str| crate::i18n::tr(s).to_owned();
        match self {
            Step::Disable(p) => {
                let why = bloat::rate(p, true).map_or("", |v| v.reason);
                format!("{} ({p})", crate::i18n::tr(why))
            }
            Step::Animations => tr("Menus and switching apps feel quicker."),
            Step::Freezer => tr("Apps left in the background are paused instead of using the CPU."),
            Step::TrimCaches => tr("Frees space; logins, settings and add-ons are kept."),
            Step::Compile => tr("Apps open faster. Takes several minutes and the TV is slow meanwhile, so run it when nobody is watching."),
        }
    }

    fn fix(&self) -> Fix {
        match self {
            Step::Disable(p) => Fix::Disable(p.clone()),
            Step::Animations => Fix::Animations("0.5"),
            Step::Freezer => Fix::EnableFreezer,
            Step::TrimCaches => Fix::ClearCaches,
            Step::Compile => Fix::CompileApps,
        }
    }
}

/// The changes worth making on this TV.
pub fn steps(state: &TvState) -> Vec<Step> {
    let mut steps: Vec<Step> = state.bloat.iter().map(|(p, _)| Step::Disable(p.clone())).collect();
    if !state.animations_fast {
        steps.push(Step::Animations);
    }
    if !state.freezer_on {
        steps.push(Step::Freezer);
    }
    steps.extend([Step::TrimCaches, Step::Compile]);
    steps
}

/// Make the chosen changes in order. One that fails is logged and the rest
/// still run.
pub async fn apply(conn: Connection, steps: Vec<Step>, progress: Progress) -> Outcome {
    progress.reset(Unit::Items, steps.len() as u64, "");
    let mut outcome = Outcome::default();
    for step in steps {
        let fix = step.fix();
        let what = match &step {
            Step::Disable(p) => format!("Disable {p}"),
            _ => fix.label().to_owned(),
        };
        progress.set_detail(what.clone());
        match health::apply(conn.clone(), fix).await {
            Ok(done) => {
                outcome.lines.extend(done.lines);
                outcome.changes.extend(done.changes);
            }
            Err(e) => outcome.lines.push(format!("{what}: failed: {e}")),
        }
        progress.add(1);
    }
    outcome
}

// ── Before and after ─────────────────────────────────────────────────────────

const CHECKUP: &str = "echo @@mem; cat /proc/meminfo; \
    echo @@df; df -k /data 2>/dev/null; \
    echo @@apps; ps -A -o USER 2>/dev/null | grep -c '^u0_a'";

/// How long the CPU is watched.
pub const CPU_WINDOW: Duration = Duration::from_secs(10);

pub fn parse_checkup(raw: &str, cpu_busy: f32) -> Checkup {
    let sec = split_sections(raw);
    let get = |k: &str| sec.get(k).map(String::as_str).unwrap_or("");
    let mem = parse_meminfo(get("mem"));
    let m = |k: &str| mem.get(k).copied().unwrap_or(0);
    Checkup {
        time: crate::journal::now(),
        note: String::new(),
        cpu_busy,
        mem_available: m("MemAvailable"),
        mem_total: m("MemTotal"),
        swap_used: m("SwapTotal").saturating_sub(m("SwapFree")),
        storage_free: parse_df(get("df")).first().map_or(0, |r| r.avail),
        apps_running: get("apps").trim().parse().unwrap_or(0),
    }
}

/// How hard the TV works right now: CPU over [`CPU_WINDOW`], memory, swap,
/// free storage and running apps. Read-only.
pub async fn checkup(conn: Connection) -> Result<Checkup, String> {
    let first = telemetry::sample(conn.clone()).await?;
    tokio::time::sleep(CPU_WINDOW).await;
    let second = telemetry::sample(conn.clone()).await?;
    let cpu = telemetry::cpu_load(&first, &second).ok_or("the TV's CPU counters didn't move")?;
    let raw = conn.shell(CHECKUP, Duration::from_secs(20)).await.map_err(|e| e.to_string())?;
    Ok(parse_checkup(&raw, cpu * 100.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed from the Sony A80J (Android 12).
    const RAW: &str = "@@system
package:android
package:com.sony.dtv.livingfit
package:com.android.printspooler
package:com.google.android.play.games
package:tv.samba.ssm
package:com.google.android.apps.tv.launcherx
@@disabled
package:tv.samba.ssm
@@global
window_animation_scale=0.5
transition_animation_scale=0.5
animator_duration_scale=null
cached_apps_freezer=enabled
@@secure
screensaver_components=com.sony.dtv.livingfit/com.sony.dtv.livingfit.MainDreamService
screensaver_enabled=1
enabled_input_methods=com.google.android.inputmethod.latin/com.android.inputmethod.latin.LatinIME:com.google.android.tts/x.VoiceInputMethodService
cert_pin_content_url=https://www.gstatic.com/android/config_update/pins.txt
@@home
priority=0 preferredOrder=0 match=0x108000 specificIndex=-1 isDefault=true
com.google.android.apps.tv.launcherx/.home.HomeActivity
@@df
Filesystem 1K-blocks Used Available Use% Mounted on
/dev/block/dm-5 4300000 3500000 800000 82% /data
@@uptime
17765.67 37839.03
@@audio
  mic mute FromSwitch=false FromRestrictions=false FromApi=false from system=false
@@top
    mResumedActivity: ActivityRecord{d085aae u0 com.google.android.play.games/.Main t1452}
";

    #[test]
    fn reads_the_tv() {
        let tv = parse(RAW);
        // livingfit is the screen saver and Play Games is on screen: both kept.
        let bloat: Vec<&str> = tv.bloat.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(bloat, ["com.android.printspooler"]);
        assert!(!tv.animations_fast, "animator_duration_scale is unset, so 1×");
        assert!(tv.freezer_on);
        assert_eq!(tv.uptime_secs, Some(17765));
        assert_eq!(tv.screensaver.as_deref(), Some("com.sony.dtv.livingfit"));
        assert!(!tv.screensaver_disabled);
        assert!(parse(&RAW.replace("@@disabled\n", "@@disabled\npackage:com.sony.dtv.livingfit\n")).screensaver_disabled);
        assert_eq!(tv.mic_muted, Some(false));
        let in_use = in_use("a=com.x.y/.Z:com.q.r/s.T\nurl=https://www.gstatic.com/a/b\n");
        assert_eq!(in_use.into_iter().collect::<Vec<_>>(), ["com.q.r", "com.x.y"]);
    }

    #[test]
    fn screen_saver_off_and_nothing_to_do() {
        let tv = parse("@@secure\nscreensaver_enabled=0\nscreensaver_components=com.a.b/.C\n@@global\nwindow_animation_scale=0.5\n\
                        transition_animation_scale=0\nanimator_duration_scale=0.5\ncached_apps_freezer=enabled\n");
        assert_eq!(tv.screensaver, None);
        assert_eq!(tv.mic_muted, None);
        assert_eq!(steps(&tv), [Step::TrimCaches, Step::Compile]);
    }

    #[test]
    fn steps_run_in_order() {
        let tv = TvState { bloat: vec![("com.android.printspooler".into(), "")], ..TvState::default() };
        let steps = steps(&tv);
        assert_eq!(steps, [Step::Disable("com.android.printspooler".into()), Step::Animations, Step::Freezer, Step::TrimCaches, Step::Compile]);
        let mut sorted = steps.clone();
        sorted.sort();
        assert_eq!(sorted, steps, "a set of picks runs in the listed order");
        assert!(!Step::Compile.default_on() && Step::TrimCaches.default_on());
    }

    #[test]
    fn reads_a_checkup() {
        let raw = "@@mem\nMemTotal: 3000000 kB\nMemAvailable: 1000000 kB\nSwapTotal: 1000 kB\nSwapFree: 200 kB\n\
                   @@df\nFilesystem 1K-blocks Used Available Use% Mounted on\n/dev/block/dm-5 4300000 3500000 800000 82% /data\n@@apps\n49\n";
        let c = parse_checkup(raw, 31.5);
        assert_eq!((c.mem_available, c.mem_total, c.swap_used), (1_000_000 * 1024, 3_000_000 * 1024, 800 * 1024));
        assert_eq!((c.storage_free, c.apps_running, c.cpu_busy), (800_000 * 1024, 49, 31.5));
    }

    /// Read-only: the Recommended list, the TV settings and one measurement.
    /// `STT_TEST_TV=ip:port cargo test live_tuneup -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_tuneup() {
        let serial = std::env::var("STT_TEST_TV").expect("set STT_TEST_TV=ip:port");
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let conn = super::super::transport::connect(serial).await.expect("connect");
            let tv = read(conn.clone()).await.expect("read");
            println!("{tv:#?}");
            for s in steps(&tv) {
                println!("  [{}] {} — {}", if s.default_on() { "x" } else { " " }, s.title(), s.detail());
            }
            println!("{:#?}", checkup(conn).await.expect("checkup"));
        });
    }
}
