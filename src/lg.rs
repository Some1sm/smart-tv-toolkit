//! LG webOS TVs: the SSAP WebSocket API the LG phone remote uses.
//!
//! - `wss://<ip>:3001` (newer firmware) or `ws://<ip>:3000`.
//! - The first connection shows a pairing prompt on the TV; once accepted
//!   the TV returns a client key so later connections need no prompt.
//! - Requests are `{"type":"request","id":…,"uri":"ssap://…","payload":…}`.
//! - Remote buttons go over a second "pointer input" socket as text:
//!   `type:button\nname:HOME\n\n`.
//!
//! No developer mode needed. Protocol and manifest as used by PyWebOSTV.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::ws::{self, Client};

pub const PORT: u16 = 3001;
pub const PLAIN_PORT: u16 = 3000;
/// The TV waits this long for the user to accept the pairing prompt.
const PROMPT_TIMEOUT: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct Session {
    /// `ip:3001` (or `ip:3000`)
    pub serial: String,
    pub info: Info,
    client: Client,
    next_id: Arc<AtomicU64>,
    /// The pointer socket for remote buttons, opened on first use.
    pointer: Arc<Mutex<Option<Client>>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Info {
    pub model: String,
    pub fields: Vec<(String, String)>,
}

impl Session {
    pub fn close(&self) {
        self.client.close();
    }

    pub fn label(&self) -> String {
        if self.info.model.is_empty() { "LG webOS TV".into() } else { format!("LG {}", self.info.model) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub id: String,
    pub name: String,
}

/// The registration manifest. The `signed` part and its signature come from
/// LG's own remote app and must stay byte-for-byte as they are.
fn manifest() -> Value {
    json!({
        "manifestVersion": 1,
        "appVersion": "1.1",
        "signed": {
            "created": "20140509",
            "appId": "com.lge.test",
            "vendorId": "com.lge",
            "localizedAppNames": {"": "LG Remote App", "ko-KR": "리모컨 앱", "zxx-XX": "ЛГ Rэмotэ AПП"},
            "localizedVendorNames": {"": "LG Electronics"},
            "permissions": [
                "TEST_SECURE", "CONTROL_INPUT_TEXT", "CONTROL_MOUSE_AND_KEYBOARD", "READ_INSTALLED_APPS",
                "READ_LGE_SDX", "READ_NOTIFICATIONS", "SEARCH", "WRITE_SETTINGS", "WRITE_NOTIFICATION_ALERT",
                "CONTROL_POWER", "READ_CURRENT_CHANNEL", "READ_RUNNING_APPS", "READ_UPDATE_INFO",
                "UPDATE_FROM_REMOTE_APP", "READ_LGE_TV_INPUT_EVENTS", "READ_TV_CURRENT_TIME"
            ],
            "serial": "2f930e2d2cfe083771f68e4fe7bb07"
        },
        "permissions": [
            "LAUNCH", "LAUNCH_WEBAPP", "APP_TO_APP", "CLOSE", "TEST_OPEN", "TEST_PROTECTED", "CONTROL_AUDIO",
            "CONTROL_DISPLAY", "CONTROL_INPUT_JOYSTICK", "CONTROL_INPUT_MEDIA_RECORDING",
            "CONTROL_INPUT_MEDIA_PLAYBACK", "CONTROL_INPUT_TV", "CONTROL_POWER", "READ_APP_STATUS",
            "READ_CURRENT_CHANNEL", "READ_INPUT_DEVICE_LIST", "READ_NETWORK_STATE", "READ_RUNNING_APPS",
            "READ_TV_CHANNEL_LIST", "WRITE_NOTIFICATION_TOAST", "READ_POWER_STATE", "READ_COUNTRY_INFO",
            "READ_SETTINGS", "CONTROL_TV_SCREEN", "CONTROL_TV_STANBY", "CONTROL_FAVORITE_GROUP",
            "CONTROL_USER_INFO", "CHECK_BLUETOOTH_DEVICE", "CONTROL_BLUETOOTH", "CONTROL_TIMER_INFO",
            "STB_INTERNAL_CONNECTION", "CONTROL_RECORDING", "READ_RECORDING_STATE", "WRITE_RECORDING_LIST",
            "READ_RECORDING_LIST", "READ_RECORDING_SCHEDULE", "WRITE_RECORDING_SCHEDULE",
            "READ_STORAGE_DEVICE_LIST", "READ_TV_PROGRAM_INFO", "CONTROL_BOX_CHANNEL", "READ_TV_ACR_AUTH_TOKEN",
            "READ_TV_CONTENT_STATE", "READ_TV_CURRENT_TIME", "ADD_LAUNCHER_CHANNEL", "SET_CHANNEL_SKIP",
            "RELEASE_CHANNEL_SKIP", "CONTROL_CHANNEL_BLOCK", "DELETE_SELECT_CHANNEL", "CONTROL_CHANNEL_GROUP",
            "SCAN_TV_CHANNELS", "CONTROL_TV_POWER", "CONTROL_WOL"
        ],
        "signatures": [{
            "signatureVersion": 1,
            "signature": "eyJhbGdvcml0aG0iOiJSU0EtU0hBMjU2Iiwia2V5SWQiOiJ0ZXN0LXNpZ25pbmctY2VydCIsInNpZ25hdHVyZVZlcnNpb24iOjF9.hrVRgjCwXVvE2OOSpDZ58hR+59aFNwYDyjQgKk3auukd7pcegmE2CzPCa0bJ0ZsRAcKkCTJrWo5iDzNhMBWRyaMOv5zWSrthlf7G128qvIlpMT0YNY+n/FaOHE73uLrS/g7swl3/qH/BGFG2Hu4RlL48eb3lLKqTt2xKHdCs6Cd4RMfJPYnzgvI4BNrFUKsjkcu+WD4OO2A27Pq1n50cMchmcaXadJhGrOqH5YmHdOCj5NSHzJYrsW0HPlpuAx/ECMeIZYDh6RMqaFM2DXzdKX9NmmyqzJ3o/0lkk/N97gfVRLW5hA29yeAwaCViZNCP8iC9aO0q9fQojoa7NQnAtw=="
        }]
    })
}

pub fn register_message(client_key: Option<&str>) -> Value {
    let mut payload = json!({"forcePairing": false, "pairingType": "PROMPT", "manifest": manifest()});
    if let Some(key) = client_key.filter(|k| !k.is_empty()) {
        payload["client-key"] = json!(key);
    }
    json!({"type": "register", "id": "register_0", "payload": payload})
}

fn ip_of(serial: &str) -> &str {
    serial.rsplit_once(':').map_or(serial, |(ip, _)| ip)
}

pub enum Connected {
    /// The session and the client key to keep for next time.
    Ready(Session, String),
    Denied,
}

/// Connect (secure port first), waiting for the pairing prompt if needed.
pub async fn connect(serial: String, client_key: Option<String>) -> Result<Connected, String> {
    let ip = ip_of(&serial).to_owned();
    let mut last_error = String::new();
    for (scheme, port) in [("wss", PORT), ("ws", PLAIN_PORT)] {
        match Client::connect(&format!("{scheme}://{ip}:{port}"), Duration::from_secs(5)).await {
            Ok((client, rx)) => return register(format!("{ip}:{port}"), client, rx, client_key).await,
            Err(e) => last_error = e,
        }
    }
    Err(format!("couldn't reach the TV's remote API: {last_error}"))
}

async fn register(
    serial: String,
    client: Client,
    mut rx: tokio::sync::broadcast::Receiver<Value>,
    client_key: Option<String>,
) -> Result<Connected, String> {
    client.send(&register_message(client_key.as_deref()))?;
    let key = loop {
        let msg = ws::wait_for(&mut rx, |v| v["id"] == "register_0", PROMPT_TIMEOUT)
            .await
            .map_err(|_| "no answer: accept the prompt on the TV's screen, then Connect again".to_owned())?;
        match msg["type"].as_str() {
            Some("registered") => break msg["payload"]["client-key"].as_str().unwrap_or("").to_owned(),
            // The TV is showing the prompt; keep waiting for the answer.
            Some("response") if msg["payload"]["pairingType"] == "PROMPT" => continue,
            Some("error") => {
                let error = msg["error"].as_str().unwrap_or("");
                if error.contains("rejected") || error.contains("denied") || error.contains("cancel") {
                    return Ok(Connected::Denied);
                }
                return Err(format!("the TV refused the connection: {error}"));
            }
            _ => continue,
        }
    };
    let mut session = Session {
        serial,
        info: Info::default(),
        client,
        next_id: Arc::new(AtomicU64::new(1)),
        pointer: Arc::new(Mutex::new(None)),
    };
    session.info = info(&session).await;
    Ok(Connected::Ready(session, key))
}

/// One SSAP request; returns its payload.
pub async fn request(session: &Session, uri: &str, payload: Value) -> Result<Value, String> {
    let id = format!("req_{}", session.next_id.fetch_add(1, Ordering::Relaxed));
    let msg = json!({"type": "request", "id": id, "uri": uri, "payload": payload});
    let reply = session.client.request(&msg, |v| v["id"] == id.as_str(), REQUEST_TIMEOUT).await?;
    if reply["type"] == "error" || reply["payload"]["returnValue"] == false {
        let why = reply["error"].as_str().or(reply["payload"]["errorText"].as_str()).unwrap_or("request failed");
        return Err(why.to_owned());
    }
    Ok(reply["payload"].clone())
}

pub fn parse_info(software: &Value, system: &Value) -> Info {
    let mut fields = Vec::new();
    let s = |v: &Value, key: &str| v[key].as_str().map(str::to_owned).or_else(|| v[key].as_i64().map(|n| n.to_string()));
    for (label, key) in [("Product", "product_name"), ("Model", "model_name"), ("Software", "sw_type"), ("Country", "country")] {
        if let Some(v) = s(software, key).filter(|v| !v.is_empty()) {
            fields.push((label.to_owned(), v));
        }
    }
    if let (Some(major), Some(minor)) = (s(software, "major_ver"), s(software, "minor_ver")) {
        fields.push(("webOS firmware".into(), format!("{major}.{minor}")));
    }
    for (label, key) in [("Model name", "modelName"), ("Receiver", "receiverType"), ("Program mode", "programMode")] {
        if let Some(v) = s(system, key).filter(|v| !v.is_empty()) {
            fields.push((label.to_owned(), v));
        }
    }
    let model = s(system, "modelName").or_else(|| s(software, "model_name")).unwrap_or_default();
    Info { model, fields }
}

async fn info(session: &Session) -> Info {
    let software = request(session, "ssap://com.webos.service.update/getCurrentSWInformation", json!({})).await.unwrap_or_default();
    let system = request(session, "ssap://system/getSystemInfo", json!({})).await.unwrap_or_default();
    parse_info(&software, &system)
}

pub async fn ping(session: Session) -> Result<(), String> {
    request(&session, "ssap://com.webos.applicationManager/getForegroundAppInfo", json!({})).await.map(|_| ())
}

/// Press a remote button (`HOME`, `UP`, `ENTER`, `VOLUMEUP`…).
pub async fn button(session: Session, name: &'static str) -> Result<(), String> {
    let mut pointer = session.pointer.lock().await;
    if pointer.as_ref().is_none_or(|p| !p.alive()) {
        let reply = request(&session, "ssap://com.webos.service.networkinput/getPointerInputSocket", json!({})).await?;
        let path = reply["socketPath"].as_str().ok_or("the TV gave no input socket")?;
        *pointer = Some(Client::connect(path, Duration::from_secs(5)).await?.0);
    }
    let socket = pointer.as_ref().ok_or("no input socket")?;
    socket.send_text(format!("type:button\nname:{name}\n\n"))
}

pub async fn power_off(session: Session) -> Result<(), String> {
    request(&session, "ssap://system/turnOff", json!({})).await.map(|_| ())
}

pub async fn text(session: Session, text: String) -> Result<(), String> {
    request(&session, "ssap://com.webos.service.ime/insertText", json!({"text": text, "replace": 0})).await.map(|_| ())
}

pub fn parse_apps(payload: &Value) -> Vec<App> {
    let mut apps: Vec<App> = payload["launchPoints"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| Some(App { id: a["id"].as_str()?.to_owned(), name: a["title"].as_str().unwrap_or("").to_owned() }))
        .collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

pub async fn apps(session: Session) -> Result<Vec<App>, String> {
    Ok(parse_apps(&request(&session, "ssap://com.webos.applicationManager/listLaunchPoints", json!({})).await?))
}

pub async fn launch(session: Session, app_id: String) -> Result<(), String> {
    request(&session, "ssap://system.launcher/launch", json!({"id": app_id})).await.map(|_| ())
}

pub async fn close(session: Session, app_id: String) -> Result<(), String> {
    request(&session, "ssap://system.launcher/close", json!({"id": app_id})).await.map(|_| ())
}

/// Android key codes (the app's remote) → webOS button names. Power has its
/// own request ([`power_off`]).
pub fn button_name(android_code: u16) -> Option<&'static str> {
    Some(match android_code {
        19 => "UP",
        20 => "DOWN",
        21 => "LEFT",
        22 => "RIGHT",
        23 => "ENTER",
        4 => "BACK",
        3 => "HOME",
        82 => "MENU",
        24 => "VOLUMEUP",
        25 => "VOLUMEDOWN",
        164 => "MUTE",
        88 => "REWIND",
        85 => "PLAY",
        87 => "FASTFORWARD",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_carries_the_key() {
        let fresh = register_message(None);
        assert_eq!(fresh["type"], "register");
        assert!(fresh["payload"].get("client-key").is_none());
        assert_eq!(fresh["payload"]["manifest"]["signed"]["appId"], "com.lge.test");
        let again = register_message(Some("abc123"));
        assert_eq!(again["payload"]["client-key"], "abc123");
    }

    #[test]
    fn reads_info_and_apps() {
        let sw = json!({"product_name": "webOSTV 6.0", "model_name": "HE_DTV_W21P_AFADATAA", "major_ver": "03", "minor_ver": "20.40", "country": "ES"});
        let sys = json!({"modelName": "OLED55C14LB", "receiverType": "dvb"});
        let info = parse_info(&sw, &sys);
        assert_eq!(info.model, "OLED55C14LB");
        assert!(info.fields.contains(&("webOS firmware".into(), "03.20.40".into())));
        let apps = parse_apps(&json!({"launchPoints": [{"id": "youtube.leanback.v4", "title": "YouTube"}, {"id": "netflix", "title": "Netflix"}]}));
        assert_eq!(apps[0], App { id: "netflix".into(), name: "Netflix".into() });
        assert_eq!(button_name(3), Some("HOME"));
    }

    /// The pairing handshake against a stand-in TV: prompt, then registered.
    #[test]
    fn pairs_with_a_fake_tv() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                let (tcp, _) = listener.accept().await.unwrap();
                let mut tv = tokio_tungstenite::accept_async(tcp).await.unwrap();
                while let Some(Ok(Message::Text(t))) = tv.next().await {
                    let v: Value = serde_json::from_str(&t).unwrap();
                    let replies = if v["type"] == "register" {
                        vec![
                            json!({"type": "response", "id": "register_0", "payload": {"pairingType": "PROMPT", "returnValue": true}}),
                            json!({"type": "registered", "id": "register_0", "payload": {"client-key": "k-42"}}),
                        ]
                    } else if v["uri"] == "ssap://system/getSystemInfo" {
                        vec![json!({"type": "response", "id": v["id"], "payload": {"returnValue": true, "modelName": "OLED55C14LB"}})]
                    } else {
                        vec![json!({"type": "response", "id": v["id"], "payload": {"returnValue": true}})]
                    };
                    for r in replies {
                        tv.send(Message::Text(r.to_string().into())).await.unwrap();
                    }
                }
            });
            let (client, rx) = Client::connect(&format!("ws://{addr}"), Duration::from_secs(5)).await.unwrap();
            let Connected::Ready(session, key) = register(format!("{addr}"), client, rx, None).await.unwrap() else {
                panic!("denied")
            };
            assert_eq!(key, "k-42");
            assert_eq!(session.label(), "LG OLED55C14LB");
            ping(session).await.unwrap();
        });
    }
}
