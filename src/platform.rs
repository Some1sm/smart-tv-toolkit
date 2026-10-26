//! The TV platforms the toolkit can talk to, and what each one supports.
//!
//! Each platform has its own transport and backend module (`android` today;
//! LG webOS and Samsung Tizen later). A connected TV is a [`Session`]; the
//! app asks it which tabs make sense ([`Platform::tabs`]) and tabs reach the
//! platform-specific connection through [`Session::android`] and siblings,
//! so adding a platform means a new variant plus the tabs it can serve.

use crate::android;
use crate::tabs::Tab;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    AndroidTv,
}

impl Platform {
    pub fn name(self) -> &'static str {
        match self {
            Platform::AndroidTv => "Android TV",
        }
    }

    /// Tabs this platform supports, in display order.
    pub fn tabs(self) -> &'static [Tab] {
        match self {
            Platform::AndroidTv => &Tab::ALL,
        }
    }
}

/// A connected TV.
#[derive(Debug, Clone)]
pub enum Session {
    Android(android::Session),
}

impl Session {
    pub fn platform(&self) -> Platform {
        match self {
            Session::Android(_) => Platform::AndroidTv,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Session::Android(s) => s.summary.label(),
        }
    }

    /// How the transport reaches the TV, e.g. `192.168.1.78:5555`.
    pub fn address(&self) -> &str {
        match self {
            Session::Android(s) => &s.serial,
        }
    }

    pub fn supports(&self, tab: Tab) -> bool {
        self.platform().tabs().contains(&tab)
    }

    /// The Android backend, if this is an Android TV.
    pub fn android(&self) -> Option<&android::Session> {
        match self {
            Session::Android(s) => Some(s),
        }
    }
}
