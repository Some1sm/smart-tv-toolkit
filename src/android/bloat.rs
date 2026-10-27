//! Which preinstalled apps are bloatware. A curated list (Google TV, Sony
//! BRAVIA and common partners) plus a few patterns that hold on any brand
//! (ad tracking, promos, demo mode). Apps not listed stay unrated: guessing
//! wrong about a system package is worse than saying nothing.
//!
//! This only labels apps; nothing here changes the TV.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rating {
    /// Ads, tracking, demos and the like: safe to disable.
    Bloat,
    /// Preinstalled extras: fine to disable if you don't use them.
    Optional,
    /// Needed by the TV or by other apps: leave it alone.
    Keep,
}

impl Rating {
    pub fn label(self) -> &'static str {
        match self {
            Rating::Bloat => "Bloatware",
            Rating::Optional => "Optional",
            Rating::Keep => "Keep",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    pub rating: Rating,
    pub reason: &'static str,
}

// Shared reasons (also the keys for translations).
const TRACKING: &str = "Watches what's on screen to build viewing and ad data.";
const PROMOS: &str = "Ads and promotions on the home screen.";
const DEMO: &str = "Store demo mode; not needed at home.";
const PARTNER: &str = "Preinstalled streaming app; remove it if you don't use it.";
const REGIONAL: &str = "Shortcut or service for a regional TV channel.";
const CAMERA: &str = "Only for the BRAVIA CAM camera.";
const HOTEL: &str = "Hotel and business mode features.";
const DIAGNOSTICS: &str = "Usage reporting and diagnostics for the maker.";
const APPLE: &str = "Apple AirPlay and HomeKit support.";
const DLNA: &str = "Plays media shared from other devices on the network (DLNA).";
const BROADCAST_WEB: &str = "Red-button extras on broadcast channels (HbbTV).";
const VOICE: &str = "Voice search and the Assistant (the remote's mic button).";
const CAST: &str = "Casting from phones and Chrome (Chromecast built-in).";
const ACCESSIBILITY: &str = "Accessibility (screen reader, speech).";
const HELP: &str = "Help and support guides.";
const GOOGLE_APP: &str = "Google app; remove it if you don't use it.";
const EXTRA: &str = "Preinstalled extra you can live without.";
const SYSTEM: &str = "Part of the system; the TV needs it.";
const GOOGLE_CORE: &str = "Google service other apps and the home screen rely on.";
const TV_CORE: &str = "Part of the TV's own picture, sound, inputs or settings.";

/// Exact packages.
const KNOWN: &[(&str, Rating, &str)] = &[
    // Tracking and ads
    ("tv.samba.ssm", Rating::Bloat, TRACKING),
    ("tv.samba.ssm.ui", Rating::Bloat, TRACKING),
    ("com.sony.dtv.promos", Rating::Bloat, PROMOS),
    ("com.sony.dtv.sonyselect", Rating::Bloat, PROMOS),
    ("com.sonyericsson.idd.agent", Rating::Bloat, DIAGNOSTICS),
    // Demo mode
    ("com.sony.dtv.demosupport", Rating::Bloat, DEMO),
    ("com.sony.dtv.demosystemsupport", Rating::Bloat, DEMO),
    ("com.sony.dtv.multiscreendemo", Rating::Bloat, DEMO),
    // Useless on a TV
    ("com.android.printspooler", Rating::Bloat, "Printing service; TVs don't print."),
    ("com.google.android.syncadapters.calendar", Rating::Bloat, "Calendar sync; nothing on the TV uses it."),
    ("com.google.android.play.games", Rating::Bloat, "Google Play Games, discontinued on TVs."),
    ("com.sony.dtv.livingfit", Rating::Bloat, CAMERA),
    // Partner streaming apps
    ("com.netflix.ninja", Rating::Optional, PARTNER),
    ("com.netflix.partner.ncm", Rating::Optional, PARTNER),
    ("com.amazon.amazonvideo.livingroom", Rating::Optional, PARTNER),
    ("com.apple.atve.sony.appletv", Rating::Optional, PARTNER),
    ("com.sonypicturescore", Rating::Optional, PARTNER),
    ("com.disney.disneyplus", Rating::Optional, PARTNER),
    ("com.spotify.tv.android", Rating::Optional, PARTNER),
    ("com.sony.dtv.osat.music", Rating::Optional, PARTNER),
    // Regional
    ("de.hdplus.tvapp.androidtv", Rating::Optional, REGIONAL),
    ("com.youview.poa", Rating::Optional, REGIONAL),
    ("com.sony.dtv.all4launcher", Rating::Optional, REGIONAL),
    ("com.sony.dtv.my5launcher", Rating::Optional, REGIONAL),
    ("com.sony.dtv.iplayerlauncher", Rating::Optional, REGIONAL),
    ("com.sony.dtv.itvlauncher", Rating::Optional, REGIONAL),
    ("com.sony.dtv.ardlauncher", Rating::Optional, REGIONAL),
    // Google apps
    ("com.google.android.youtube.tv", Rating::Optional, GOOGLE_APP),
    ("com.google.android.youtube.tvmusic", Rating::Optional, GOOGLE_APP),
    ("com.google.android.youtube.tvkids", Rating::Optional, GOOGLE_APP),
    ("com.google.android.videos", Rating::Optional, GOOGLE_APP),
    ("com.google.android.feedback", Rating::Optional, DIAGNOSTICS),
    ("com.google.android.apps.mediashell", Rating::Optional, CAST),
    ("com.google.android.katniss", Rating::Optional, VOICE),
    ("com.sony.dtv.voicesearch", Rating::Optional, VOICE),
    ("com.sony.dtv.assistantadapter", Rating::Optional, VOICE),
    ("com.google.android.marvin.talkback", Rating::Optional, ACCESSIBILITY),
    ("com.google.android.tts", Rating::Optional, ACCESSIBILITY),
    ("com.sony.dtv.accessibility.tutorial", Rating::Optional, ACCESSIBILITY),
    // Sony extras
    ("com.sony.dtv.airplayapp", Rating::Optional, APPLE),
    ("com.sony.dtv.homekit", Rating::Optional, APPLE),
    ("com.sony.dtv.gestureapp", Rating::Optional, CAMERA),
    ("com.sony.dtv.distancealert", Rating::Optional, CAMERA),
    ("com.sony.dtv.sensingcamera.fwupdate", Rating::Optional, CAMERA),
    ("com.sony.dtv.sensingcamera.settings", Rating::Optional, CAMERA),
    ("com.sony.dtv.sensingfusionservice", Rating::Optional, CAMERA),
    ("com.sonyericsson.dlna", Rating::Optional, DLNA),
    ("com.sony.huey.dlna.module", Rating::Optional, DLNA),
    ("com.sony.huey.dlna.renderersettings", Rating::Optional, DLNA),
    ("com.sony.dtv.hbbtvplayer", Rating::Optional, BROADCAST_WEB),
    ("com.vewd.core.integration.dia", Rating::Optional, BROADCAST_WEB),
    ("com.vewd.core.service", Rating::Optional, BROADCAST_WEB),
    ("com.sony.dtv.customersupport", Rating::Optional, HELP),
    ("com.sony.dtv.smarthelp", Rating::Optional, HELP),
    ("com.sony.dtv.helpguidecontents", Rating::Optional, HELP),
    ("com.sony.dtv.ecodashboard", Rating::Optional, EXTRA),
    ("com.sony.dtv.sonybugreportsys", Rating::Optional, DIAGNOSTICS),
    ("com.sony.dtv.sonyloglevelsettingsys", Rating::Optional, DIAGNOSTICS),
    ("com.uei.quicksetsdk.sony", Rating::Optional, "Lets the TV remote control set-top boxes and soundbars."),
    ("com.sony.dtv.irblaster", Rating::Optional, "Lets the TV remote control set-top boxes and soundbars."),
    ("com.android.dreams.basic", Rating::Optional, EXTRA),
    ("com.android.htmlviewer", Rating::Optional, EXTRA),
    ("com.google.android.apps.tv.dreamx", Rating::Optional, "Ambient mode (the screen saver with photos)."),
    // Keep
    ("android", Rating::Keep, SYSTEM),
    ("com.google.android.gms", Rating::Keep, GOOGLE_CORE),
    ("com.google.android.gsf", Rating::Keep, GOOGLE_CORE),
    ("com.android.vending", Rating::Keep, GOOGLE_CORE),
    ("com.google.android.webview", Rating::Keep, GOOGLE_CORE),
    ("com.google.android.apps.tv.launcherx", Rating::Keep, "The Google TV home screen (pick another home app first)."),
    ("com.google.android.tungsten.setupwraith", Rating::Keep, GOOGLE_CORE),
    ("com.google.android.tv.remote.service", Rating::Keep, "Pairs and runs the remote and the phone remote."),
    ("com.google.android.inputmethod.latin", Rating::Keep, "The on-screen keyboard."),
    ("com.google.android.partnersetup", Rating::Keep, GOOGLE_CORE),
    ("com.google.android.packageinstaller", Rating::Keep, SYSTEM),
    ("com.google.android.permissioncontroller", Rating::Keep, SYSTEM),
    ("com.sony.dtv.b2b.frameworkextension", Rating::Keep, SYSTEM),
    ("com.sony.dtv.da.service", Rating::Keep, SYSTEM),
    ("com.qterics.da.product", Rating::Keep, SYSTEM),
];

/// Prefixes: (prefix, rating, reason). Checked after exact matches.
const PREFIXES: &[(&str, Rating, &str)] = &[
    ("com.sony.dtv.b2b.", Rating::Optional, HOTEL),
    ("com.sony.dtv.seeds.", Rating::Optional, "Sony smart-home and phone remote-start link."),
    ("com.sony.dtv.tvinput.", Rating::Keep, TV_CORE),
    ("com.sony.dtv.tvx", Rating::Keep, TV_CORE),
    ("com.sony.dtv.tvapi", Rating::Keep, TV_CORE),
    ("com.sony.dtv.settings", Rating::Keep, TV_CORE),
    ("com.sony.dtv.sonysystemservice", Rating::Keep, TV_CORE),
    ("com.sony.dtv.sonyvendorservice", Rating::Keep, TV_CORE),
    ("com.sony.dtv.panelsettings", Rating::Keep, TV_CORE),
    ("com.sony.dtv.pictureaudio", Rating::Keep, TV_CORE),
    ("com.sony.dtv.audiooutput", Rating::Keep, TV_CORE),
    ("com.sony.dtv.systemupdate", Rating::Keep, "Installs the TV's software updates."),
    ("com.sony.dtv.update", Rating::Keep, "Installs the TV's software updates."),
    ("com.android.", Rating::Keep, SYSTEM),
    ("com.google.android.overlay", Rating::Keep, SYSTEM),
    ("com.google.android.ext.", Rating::Keep, SYSTEM),
    ("com.google.android.modulemetadata", Rating::Keep, SYSTEM),
    ("com.google.android.onetimeinitializer", Rating::Keep, SYSTEM),
    ("com.mediatek.", Rating::Keep, TV_CORE),
    ("mtktvapi.", Rating::Keep, TV_CORE),
    ("com.droidlogic.", Rating::Keep, TV_CORE),
    ("com.amlogic.", Rating::Keep, TV_CORE),
    ("com.dolby.", Rating::Keep, TV_CORE),
];

/// A preinstalled app's rating; `None` for apps the user installed or that
/// aren't known well enough to judge.
pub fn rate(package: &str, preinstalled: bool) -> Option<Verdict> {
    if !preinstalled {
        return None;
    }
    if let Some((_, rating, reason)) = KNOWN.iter().find(|(p, _, _)| *p == package) {
        return Some(Verdict { rating: *rating, reason });
    }
    // Brand-independent signs of bloat (checked before the broad prefixes).
    let parts: Vec<&str> = package.split('.').collect();
    let has = |w: &str| parts.iter().any(|p| *p == w);
    if package.contains("samba") || has("acr") {
        return Some(Verdict { rating: Rating::Bloat, reason: TRACKING });
    }
    if has("promos") || has("promo") || has("ads") || has("advertising") {
        return Some(Verdict { rating: Rating::Bloat, reason: PROMOS });
    }
    if parts.iter().any(|p| p.starts_with("demo") || p.ends_with("demo")) {
        return Some(Verdict { rating: Rating::Bloat, reason: DEMO });
    }
    if package.contains(".overlay") || package.ends_with(".rro") || package.contains("auto_generated_rro") {
        return Some(Verdict { rating: Rating::Keep, reason: SYSTEM });
    }
    PREFIXES
        .iter()
        .find(|(prefix, _, _)| package.starts_with(prefix))
        .map(|(_, rating, reason)| Verdict { rating: *rating, reason })
}

/// Every reason text, for the translation dictionary's tests.
#[cfg(test)]
pub fn all_reasons() -> Vec<&'static str> {
    let mut all: Vec<&str> = KNOWN.iter().map(|(_, _, r)| *r).chain(PREFIXES.iter().map(|(_, _, r)| *r)).collect();
    all.sort_unstable();
    all.dedup();
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_known_and_patterned_apps() {
        assert_eq!(rate("tv.samba.ssm", true).map(|v| v.rating), Some(Rating::Bloat));
        assert_eq!(rate("com.sony.dtv.promos", true).map(|v| v.rating), Some(Rating::Bloat));
        assert_eq!(rate("com.netflix.ninja", true).map(|v| v.rating), Some(Rating::Optional));
        assert_eq!(rate("com.sony.dtv.b2b.hotelmode", true).map(|v| v.rating), Some(Rating::Optional));
        assert_eq!(rate("com.google.android.gms", true).map(|v| v.rating), Some(Rating::Keep));
        assert_eq!(rate("com.android.providers.tv", true).map(|v| v.rating), Some(Rating::Keep));
        // Other brands: patterns.
        assert_eq!(rate("com.tcl.acr.service", true).map(|v| v.rating), Some(Rating::Bloat));
        assert_eq!(rate("com.example.retaildemo", true).map(|v| v.rating), Some(Rating::Bloat));
        // User apps and unknowns aren't judged.
        assert_eq!(rate("com.netflix.ninja", false), None);
        assert_eq!(rate("com.sony.dtv.sonytab", true), None);
        // No package is listed twice.
        for (p, _, _) in KNOWN {
            assert_eq!(KNOWN.iter().filter(|(q, _, _)| q == p).count(), 1, "{p}");
        }
    }
}
