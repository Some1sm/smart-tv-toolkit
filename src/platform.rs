//! The TV operating systems the toolkit can talk to.
//!
//! Each platform has its own transport — ADB for Android TV; LG webOS and
//! Samsung Tizen will use their own protocols — so tabs and features are
//! gated on the platform of the connected device.

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
}

/// A connected TV.
#[derive(Debug, Clone)]
pub struct Device {
    pub platform: Platform,
    /// How the transport addresses the device, e.g. `192.168.1.78:5555`.
    pub address: String,
    pub label: String,
}
