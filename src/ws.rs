//! A small JSON WebSocket client for TVs that are controlled over
//! WebSockets (Samsung Tizen, LG webOS).
//!
//! Messages go out through a channel to a writer task; everything that comes
//! back is parsed and broadcast, so a caller subscribes, sends, and waits for
//! the reply it recognises.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{Connector, connect_async_tls_with_config};

#[derive(Clone)]
pub struct Client {
    out: mpsc::UnboundedSender<Message>,
    incoming: broadcast::Sender<Value>,
    alive: Arc<AtomicBool>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ws::Client").field("alive", &self.alive()).finish()
    }
}

/// TVs use self-signed certificates for their local `wss://` endpoints, so
/// there's nothing to verify against (the reference clients do the same).
#[derive(Debug)]
struct AcceptTvCertificate(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AcceptTvCertificate {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn tls() -> Result<Connector, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptTvCertificate(provider)))
        .with_no_client_auth();
    Ok(Connector::Rustls(Arc::new(config)))
}

impl Client {
    /// Connect to `ws://…` or `wss://…` within `timeout`. The receiver
    /// already holds whatever the TV says first (some greet straight away).
    pub async fn connect(url: &str, timeout: Duration) -> Result<(Client, broadcast::Receiver<Value>), String> {
        let connector = if url.starts_with("wss://") { Some(tls()?) } else { None };
        let (stream, _) = tokio::time::timeout(timeout, connect_async_tls_with_config(url, None, true, connector))
            .await
            .map_err(|_| "the TV didn't answer in time".to_owned())?
            .map_err(|e| e.to_string())?;
        let (mut write, mut read) = stream.split();
        let (out, mut outbox) = mpsc::unbounded_channel::<Message>();
        let (incoming, first) = broadcast::channel(256);
        let alive = Arc::new(AtomicBool::new(true));

        let alive_w = alive.clone();
        tokio::spawn(async move {
            while let Some(msg) = outbox.recv().await {
                if write.send(msg).await.is_err() {
                    break;
                }
            }
            alive_w.store(false, Ordering::Relaxed);
        });
        let (alive_r, incoming_r) = (alive.clone(), incoming.clone());
        tokio::spawn(async move {
            while let Some(Ok(msg)) = read.next().await {
                if let Message::Text(text) = msg
                    && let Ok(value) = serde_json::from_str::<Value>(&text)
                {
                    let _ = incoming_r.send(value);
                }
            }
            alive_r.store(false, Ordering::Relaxed);
        });
        Ok((Client { out, incoming, alive }, first))
    }

    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed) && !self.out.is_closed()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.incoming.subscribe()
    }

    pub fn send_text(&self, text: String) -> Result<(), String> {
        self.out.send(Message::Text(text.into())).map_err(|_| "the connection to the TV is closed".to_owned())
    }

    pub fn send(&self, value: &Value) -> Result<(), String> {
        self.send_text(value.to_string())
    }

    /// Send `value`, then wait for the first message `is_reply` accepts.
    pub async fn request(&self, value: &Value, is_reply: impl Fn(&Value) -> bool, timeout: Duration) -> Result<Value, String> {
        let mut rx = self.subscribe();
        self.send(value)?;
        wait_for(&mut rx, is_reply, timeout).await
    }

    pub fn close(&self) {
        let _ = self.out.send(Message::Close(None));
    }
}

/// The first message on `rx` that `is_reply` accepts.
pub async fn wait_for(rx: &mut broadcast::Receiver<Value>, is_reply: impl Fn(&Value) -> bool, timeout: Duration) -> Result<Value, String> {
    tokio::time::timeout(timeout, async {
        loop {
            match rx.recv().await {
                Ok(v) if is_reply(&v) => return Ok(v),
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return Err("the connection to the TV closed".to_owned()),
            }
        }
    })
    .await
    .map_err(|_| "the TV didn't answer in time".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_setup_builds() {
        assert!(matches!(tls(), Ok(Connector::Rustls(_))));
    }

    /// A local echo server stands in for a TV.
    #[test]
    fn requests_get_their_replies() {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                let (tcp, _) = listener.accept().await.unwrap();
                let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
                while let Some(Ok(Message::Text(t))) = ws.next().await {
                    let v: Value = serde_json::from_str(&t).unwrap();
                    // Unrelated chatter first, then the reply.
                    ws.send(Message::Text(r#"{"event":"noise"}"#.into())).await.unwrap();
                    let reply = serde_json::json!({"id": v["id"], "ok": true});
                    ws.send(Message::Text(reply.to_string().into())).await.unwrap();
                }
            });
            let (client, _) = Client::connect(&format!("ws://{addr}"), Duration::from_secs(5)).await.unwrap();
            let reply = client
                .request(&serde_json::json!({"id": "7"}), |v| v["id"] == "7", Duration::from_secs(5))
                .await
                .unwrap();
            assert_eq!(reply["ok"], true);
            assert!(client.alive());
        });
    }
}
