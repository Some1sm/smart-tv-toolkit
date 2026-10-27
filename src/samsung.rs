//! Samsung Tizen TVs (2016 and newer): the TV's own remote-control API.
//!
//! - `wss://<ip>:8002/api/v2/channels/samsung.remote.control?name=<base64>&token=<t>`
//!   for keys, text and apps. The first connection shows "Allow" on the TV;
//!   the TV then hands out a token so later connections need no prompt.
//! - `http://<ip>:8001/api/v2/` for device info and app status.
//!
//! No developer mode needed. Protocol as used by the `samsungtvws` library.

use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};

use crate::ws::{self, Client};

pub const PORT: u16 = 8002;
/// The REST port, also probed to spot Samsung TVs on the network.
pub const REST_PORT: u16 = 8001;
const APP_NAME: &str = "Smart TV Toolkit";
/// The TV waits this long for the user to press Allow.
const ALLOW_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct Session {
    /// `ip:8002`
    pub serial: String,
    pub info: Info,
    pub client: Client,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Info {
    pub name: String,
    pub model: String,
    pub fields: Vec<(String, String)>,
    pub wifi_mac: Option<String>,
}

impl Session {
    pub fn label(&self) -> String {
        if self.info.model.is_empty() { self.info.name.clone() } else { format!("Samsung {}", self.info.model) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub id: String,
    pub name: String,
    /// 2 = web/deep-link app, 4 = native app (they launch differently).
    pub app_type: u64,
}

fn ip_of(serial: &str) -> &str {
    serial.rsplit_once(':').map_or(serial, |(ip, _)| ip)
}

/// A REST call to the TV's port 8001 (blocking, so on a worker thread).
async fn rest(method: &'static str, url: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(4))).build().into();
        let mut resp = match method {
            "DELETE" => agent.delete(&url).call(),
            _ => agent.get(&url).call(),
        }
        .map_err(|e| e.to_string())?;
        resp.body_mut().read_to_string().map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `GET http://<ip>:8001/api/v2/`: name, model, OS and network details.
pub async fn info(ip: String) -> Result<Info, String> {
    parse_info(&rest("GET", format!("http://{ip}:{REST_PORT}/api/v2/")).await?)
}

pub fn parse_info(body: &str) -> Result<Info, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("not a Samsung TV answer: {e}"))?;
    let d = &v["device"];
    if !d.is_object() {
        return Err("not a Samsung TV answer".into());
    }
    let s = |key: &str| d[key].as_str().unwrap_or("").to_owned();
    let mut fields = Vec::new();
    for (label, key) in [
        ("Name", "name"),
        ("Model", "modelName"),
        ("Model code", "model"),
        ("Operating system", "OS"),
        ("Firmware", "firmwareVersion"),
        ("Resolution", "resolution"),
        ("Power", "PowerState"),
        ("Network", "networkType"),
        ("IP address", "ip"),
        ("Wi-Fi MAC", "wifiMac"),
        ("Frame TV", "FrameTVSupport"),
        ("Voice", "VoiceSupport"),
        ("Developer mode", "developerMode"),
        ("Token auth", "TokenAuthSupport"),
    ] {
        let value = s(key);
        if !value.is_empty() {
            fields.push((label.to_owned(), value));
        }
    }
    if let Some(version) = v["version"].as_str() {
        fields.push(("API version".into(), version.to_owned()));
    }
    let name = s("name");
    Ok(Info {
        name: if name.is_empty() { "Samsung TV".into() } else { name },
        model: s("modelName"),
        wifi_mac: Some(s("wifiMac")).filter(|m| !m.is_empty()),
        fields,
    })
}

pub fn url(ip: &str, token: Option<&str>) -> String {
    let name = base64::engine::general_purpose::STANDARD.encode(APP_NAME);
    let mut url = format!("wss://{ip}:{PORT}/api/v2/channels/samsung.remote.control?name={name}");
    if let Some(t) = token.filter(|t| !t.is_empty()) {
        url.push_str("&token=");
        url.push_str(t);
    }
    url
}

pub enum Connected {
    /// The session and the token to keep for next time.
    Ready(Session, Option<String>),
    /// The user pressed Deny on the TV.
    Denied,
}

/// Connect, waiting for the user to press Allow on the TV if needed.
pub async fn connect(serial: String, token: Option<String>) -> Result<Connected, String> {
    let ip = ip_of(&serial).to_owned();
    let info = info(ip.clone()).await?;
    let (client, mut rx) = Client::connect(&url(&ip, token.as_deref()), Duration::from_secs(5)).await?;
    let first = ws::wait_for(
        &mut rx,
        |v| matches!(v["event"].as_str(), Some("ms.channel.connect" | "ms.channel.unauthorized" | "ms.channel.timeOut")),
        ALLOW_TIMEOUT,
    )
    .await
    .map_err(|_| "no answer from the TV: press Allow on the TV's screen, then Connect again".to_owned())?;
    match first["event"].as_str() {
        Some("ms.channel.connect") => {
            let token = first["data"]["token"].as_str().map(str::to_owned).or(token);
            Ok(Connected::Ready(Session { serial: format!("{ip}:{PORT}"), info, client }, token))
        }
        Some("ms.channel.unauthorized") => Ok(Connected::Denied),
        _ => Err("the TV timed out waiting for Allow".into()),
    }
}

/// Is the TV still there? (Its REST endpoint answers while it's on.)
pub async fn ping(session: Session) -> Result<(), String> {
    if !session.client.alive() {
        return Err("connection closed".into());
    }
    info(ip_of(&session.serial).to_owned()).await.map(|_| ())
}

/// Press a remote key, e.g. `KEY_HOME`.
pub fn key(session: &Session, key: &str) -> Result<(), String> {
    session.client.send(&json!({
        "method": "ms.remote.control",
        "params": {"Cmd": "Click", "DataOfCmd": key, "Option": "false", "TypeOfRemote": "SendRemoteKey"}
    }))
}

/// Type text into the focused field on the TV.
pub fn text(session: &Session, text: &str) -> Result<(), String> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    session.client.send(&json!({
        "method": "ms.remote.control",
        "params": {"Cmd": encoded, "DataOfCmd": "base64", "TypeOfRemote": "SendInputString"}
    }))?;
    session.client.send(&json!({"method": "ms.remote.control", "params": {"TypeOfRemote": "SendInputEnd"}}))
}

pub fn parse_apps(reply: &Value) -> Vec<App> {
    let mut apps: Vec<App> = reply["data"]["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            Some(App {
                id: a["appId"].as_str()?.to_owned(),
                name: a["name"].as_str().unwrap_or("").to_owned(),
                app_type: a["app_type"].as_u64().unwrap_or(2),
            })
        })
        .collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

pub async fn apps(session: Session) -> Result<Vec<App>, String> {
    let ask = json!({"method": "ms.channel.emit", "params": {"event": "ed.installedApp.get", "to": "host"}});
    let reply = session.client.request(&ask, |v| v["event"] == "ed.installedApp.get", Duration::from_secs(10)).await?;
    Ok(parse_apps(&reply))
}

pub fn launch(session: &Session, app: &App) -> Result<(), String> {
    let action = if app.app_type == 4 { "NATIVE_LAUNCH" } else { "DEEP_LINK" };
    session.client.send(&json!({
        "method": "ms.channel.emit",
        "params": {"event": "ed.apps.launch", "to": "host", "data": {"appId": app.id, "action_type": action}}
    }))
}

/// Close an app (`DELETE /api/v2/applications/<id>`).
pub async fn close(session: Session, app_id: String) -> Result<(), String> {
    rest("DELETE", format!("http://{}:{REST_PORT}/api/v2/applications/{app_id}", ip_of(&session.serial))).await.map(|_| ())
}

/// Android key codes (the app's remote) → Samsung key names.
pub fn key_name(android_code: u16) -> Option<&'static str> {
    Some(match android_code {
        19 => "KEY_UP",
        20 => "KEY_DOWN",
        21 => "KEY_LEFT",
        22 => "KEY_RIGHT",
        23 => "KEY_ENTER",
        4 => "KEY_RETURN",
        3 => "KEY_HOME",
        82 => "KEY_MENU",
        24 => "KEY_VOLUP",
        25 => "KEY_VOLDOWN",
        164 => "KEY_MUTE",
        88 => "KEY_REWIND",
        85 => "KEY_PLAY_BACK",
        87 => "KEY_FF",
        26 => "KEY_POWER",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_device_info() {
        let body = r#"{"device":{"FrameTVSupport":"false","OS":"Tizen","PowerState":"on","modelName":"QE55Q80BAT",
            "model":"22_PONTUSM_QTV","name":"[TV] Living Room","wifiMac":"a0:b1:c2:d3:e4:f5","resolution":"3840x2160",
            "networkType":"wireless","ip":"192.168.1.50","TokenAuthSupport":"true"},"version":"2.0.25","name":"[TV] Living Room"}"#;
        let info = parse_info(body).unwrap();
        assert_eq!(info.name, "[TV] Living Room");
        assert_eq!(info.model, "QE55Q80BAT");
        assert_eq!(info.wifi_mac.as_deref(), Some("a0:b1:c2:d3:e4:f5"));
        assert!(info.fields.contains(&("Operating system".into(), "Tizen".into())));
        assert!(parse_info(r#"{"error":1}"#).is_err());
    }

    #[test]
    fn builds_the_url_and_reads_apps() {
        let u = url("192.168.1.50", Some("12345678"));
        assert_eq!(u, "wss://192.168.1.50:8002/api/v2/channels/samsung.remote.control?name=U21hcnQgVFYgVG9vbGtpdA==&token=12345678");
        assert!(!url("192.168.1.50", None).contains("token"));
        let reply = json!({"event": "ed.installedApp.get", "data": {"data": [
            {"appId": "3201907018807", "name": "Netflix", "app_type": 2},
            {"appId": "111299001912", "name": "YouTube", "app_type": 2},
            {"appId": "org.tizen.browser", "name": "Internet", "app_type": 4}
        ]}});
        let apps = parse_apps(&reply);
        assert_eq!(apps.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Internet", "Netflix", "YouTube"]);
        assert_eq!(apps[0].app_type, 4);
        assert_eq!(key_name(3), Some("KEY_HOME"));
    }

    /// Talks to a stand-in TV: the Allow handshake hands out a token, then
    /// the app list comes back on request.
    #[test]
    fn handshake_against_a_fake_tv() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                let (tcp, _) = listener.accept().await.unwrap();
                let mut tv = tokio_tungstenite::accept_async(tcp).await.unwrap();
                let hello = json!({"event": "ms.channel.connect", "data": {"token": "99887766", "clients": []}});
                tv.send(Message::Text(hello.to_string().into())).await.unwrap();
                while let Some(Ok(Message::Text(t))) = tv.next().await {
                    let v: Value = serde_json::from_str(&t).unwrap();
                    if v["params"]["event"] == "ed.installedApp.get" {
                        let apps = json!({"event": "ed.installedApp.get", "data": {"data": [{"appId": "1", "name": "Netflix", "app_type": 2}]}});
                        tv.send(Message::Text(apps.to_string().into())).await.unwrap();
                    }
                }
            });
            let (client, mut rx) = Client::connect(&format!("ws://{addr}"), Duration::from_secs(5)).await.unwrap();
            let first = ws::wait_for(&mut rx, |v| v["event"] == "ms.channel.connect", Duration::from_secs(5)).await.unwrap();
            assert_eq!(first["data"]["token"], "99887766");
            let session = Session { serial: format!("{addr}"), info: Info::default(), client };
            let list = apps(session).await.unwrap();
            assert_eq!(list[0].name, "Netflix");
        });
    }
}
