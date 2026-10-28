//! The TV platforms the toolkit can talk to, and what each one supports.
//!
//! Each platform has its own backend: `android` (ADB), `samsung` (Tizen's
//! remote-control API) and `lg` (webOS SSAP). A connected TV is a
//! [`Session`]; the app asks it which tabs make sense ([`Platform::tabs`]).
//! Which platform a saved address belongs to follows from its port, so the
//! device history needs no extra field.

use crate::tabs::Tab;
use crate::{android, credentials, lg, samsung};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    AndroidTv,
    Tizen,
    WebOs,
}

impl Platform {
    pub fn name(self) -> &'static str {
        match self {
            Platform::AndroidTv => "Android TV",
            Platform::Tizen => "Samsung Tizen",
            Platform::WebOs => "LG webOS",
        }
    }

    /// Tabs this platform supports, in display order.
    pub fn tabs(self) -> &'static [Tab] {
        const ANDROID: [Tab; 12] = [
            Tab::Info,
            Tab::Packages,
            Tab::Performance,
            Tab::Optimize,
            Tab::Install,
            Tab::Settings,
            Tab::Tools,
            Tab::Media,
            Tab::Files,
            Tab::Remote,
            Tab::Screenshot,
            Tab::History,
        ];
        const REMOTE_API: [Tab; 3] = [Tab::Info, Tab::Apps, Tab::Remote];
        match self {
            Platform::AndroidTv => &ANDROID,
            Platform::Tizen | Platform::WebOs => &REMOTE_API,
        }
    }

    /// `8001`/`8002` are Samsung's, `3000`/`3001` LG's; anything else is ADB.
    pub fn for_port(port: u16) -> Platform {
        match port {
            samsung::PORT | samsung::REST_PORT => Platform::Tizen,
            lg::PORT | lg::PLAIN_PORT => Platform::WebOs,
            _ => Platform::AndroidTv,
        }
    }

    pub fn for_serial(serial: &str) -> Platform {
        Platform::for_port(serial.rsplit_once(':').and_then(|(_, p)| p.parse().ok()).unwrap_or(android::ADB_PORT))
    }
}

/// A connected TV.
#[derive(Debug, Clone)]
pub enum Session {
    Android(android::Session),
    Samsung(samsung::Session),
    Lg(lg::Session),
}

impl Session {
    pub fn platform(&self) -> Platform {
        match self {
            Session::Android(_) => Platform::AndroidTv,
            Session::Samsung(_) => Platform::Tizen,
            Session::Lg(_) => Platform::WebOs,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Session::Android(s) => s.summary.label(),
            Session::Samsung(s) => s.label(),
            Session::Lg(s) => s.label(),
        }
    }

    /// How the transport reaches the TV, e.g. `192.168.1.78:5555`.
    pub fn address(&self) -> &str {
        match self {
            Session::Android(s) => &s.serial,
            Session::Samsung(s) => &s.serial,
            Session::Lg(s) => &s.serial,
        }
    }

    pub fn supports(&self, tab: Tab) -> bool {
        self.platform().tabs().contains(&tab)
    }

    /// The Android backend, if this is an Android TV.
    pub fn android(&self) -> Option<&android::Session> {
        match self {
            Session::Android(s) => Some(s),
            _ => None,
        }
    }

    /// Close the remote-control socket of a Samsung/LG TV (ADB needs nothing).
    pub fn close(&self) {
        match self {
            Session::Android(_) => {}
            Session::Samsung(s) => s.client.close(),
            Session::Lg(s) => s.close(),
        }
    }

    /// Device facts for the Overview tab, for platforms without ADB's
    /// detailed system info.
    pub fn facts(&self) -> Option<&[(String, String)]> {
        match self {
            Session::Android(_) => None,
            Session::Samsung(s) => Some(&s.info.fields),
            Session::Lg(s) => Some(&s.info.fields),
        }
    }
}

/// Outcome of connecting to a TV of any platform.
#[derive(Debug, Clone)]
pub enum Connect {
    Connected(Session),
    /// The TV is waiting for the user (e.g. "Allow debugging?").
    NeedsApproval(String),
    Failed(String),
}

fn ip_of(serial: &str) -> &str {
    serial.rsplit_once(':').map_or(serial, |(ip, _)| ip)
}

/// Connect to `ip:port`; the port says which platform it is.
pub async fn connect(serial: String) -> Connect {
    let ip = ip_of(&serial).to_owned();
    match Platform::for_serial(&serial) {
        Platform::AndroidTv => match android::connect(serial).await {
            android::ConnectResult::Connected(s) => Connect::Connected(Session::Android(s)),
            android::ConnectResult::NeedsApproval => Connect::NeedsApproval(
                "On the TV, choose Allow (tick \"Always allow\"), then press Connect again.".into(),
            ),
            android::ConnectResult::Failed(e) => Connect::Failed(e),
        },
        Platform::Tizen => match samsung::connect(serial, credentials::get(&ip)).await {
            Ok(samsung::Connected::Ready(s, token)) => {
                if let Some(t) = token {
                    let _ = credentials::set(&ip, &t);
                }
                Connect::Connected(Session::Samsung(s))
            }
            Ok(samsung::Connected::Denied) => {
                Connect::NeedsApproval("The TV denied access. On the TV: Settings → General → External Device Manager → Device Connection Manager → Device List, allow \"Smart TV Toolkit\".".into())
            }
            Err(e) => Connect::Failed(e),
        },
        Platform::WebOs => match lg::connect(serial, credentials::get(&ip)).await {
            Ok(lg::Connected::Ready(s, key)) => {
                if !key.is_empty() {
                    let _ = credentials::set(&ip, &key);
                }
                Connect::Connected(Session::Lg(s))
            }
            Ok(lg::Connected::Denied) => Connect::NeedsApproval("The pairing was declined on the TV. Press Connect and accept it.".into()),
            Err(e) => Connect::Failed(e),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_follows_the_port() {
        assert_eq!(Platform::for_serial("192.168.1.78:5555"), Platform::AndroidTv);
        assert_eq!(Platform::for_serial("192.168.1.78:41234"), Platform::AndroidTv);
        assert_eq!(Platform::for_serial("192.168.1.50:8002"), Platform::Tizen);
        assert_eq!(Platform::for_serial("192.168.1.60:3001"), Platform::WebOs);
        assert_eq!(Platform::for_serial("192.168.1.60"), Platform::AndroidTv);
        assert!(Platform::Tizen.tabs().contains(&Tab::Apps));
        assert!(!Platform::AndroidTv.tabs().contains(&Tab::Apps));
    }
}
