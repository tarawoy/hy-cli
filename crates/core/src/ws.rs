use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};
use url::Url;

#[derive(Debug, Clone)]
pub struct WsClient {
    ws_url: Url,
}

impl WsClient {
    pub fn new_mainnet() -> Result<Self> {
        Self::new_from_http_base("https://api.hyperliquid.xyz")
    }

    pub fn new_testnet() -> Result<Self> {
        Self::new_from_http_base("https://api.hyperliquid-testnet.xyz")
    }

    pub fn new_from_http_base(http_base: &str) -> Result<Self> {
        // Python SDK uses: ws_url = "ws" + base_url[len("http"):] + "/ws"
        // e.g. https://api.hyperliquid.xyz -> wss://api.hyperliquid.xyz/ws
        let http = Url::parse(http_base).context("invalid base url")?;
        let scheme = match http.scheme() {
            "https" => "wss",
            "http" => "ws",
            other => anyhow::bail!("unsupported scheme for ws conversion: {other}"),
        };
        let mut ws_url = http.clone();
        ws_url
            .set_scheme(scheme)
            .map_err(|_| anyhow::anyhow!("failed to set ws scheme"))?;
        ws_url.set_path("/ws");
        Ok(Self { ws_url })
    }

    pub fn ws_url(&self) -> &Url {
        &self.ws_url
    }

    pub fn spawn(
        self,
        subscriptions: Vec<Value>,
        stop_rx: watch::Receiver<bool>,
    ) -> mpsc::Receiver<WsEvent> {
        let (tx, rx) = mpsc::channel(1024);
        tokio::spawn(async move {
            if let Err(e) = ws_task(self.ws_url, subscriptions, stop_rx, tx).await {
                // If the receiver is gone, just end.
                debug!("ws task ended: {e:#}");
            }
        });
        rx
    }
}

#[derive(Debug, Clone)]
pub enum WsEvent {
    Connected,
    Disconnected { reason: String },
    Message { msg: Value },
}

async fn ws_task(
    ws_url: Url,
    subscriptions: Vec<Value>,
    mut stop_rx: watch::Receiver<bool>,
    tx: mpsc::Sender<WsEvent>,
) -> Result<()> {
    let mut backoff = Duration::from_millis(250);

    loop {
        if *stop_rx.borrow() {
            return Ok(());
        }

        debug!(url = %ws_url, "ws connecting");
        let conn = tokio_tungstenite::connect_async(ws_url.clone()).await;
        let (ws, _resp) = match conn {
            Ok(v) => v,
            Err(e) => {
                warn!("ws connect failed: {e:#}; retrying in {:?}", backoff);
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {},
                    _ = stop_rx.changed() => {},
                }
                backoff = (backoff * 2).min(Duration::from_secs(10));
                continue;
            }
        };

        backoff = Duration::from_millis(250);
        let _ = tx.send(WsEvent::Connected).await;

        let (mut write, mut read) = ws.split();
        let (out_tx, mut out_rx) = mpsc::channel::<Message>(256);

        // writer task
        let writer = tokio::spawn(async move {
            while let Some(m) = out_rx.recv().await {
                let _ = write.send(m).await;
            }
        });

        // subscribe
        for sub in &subscriptions {
            let payload = serde_json::json!({"method":"subscribe","subscription": sub});
            let _ = out_tx.send(Message::Text(payload.to_string())).await;
        }

        // ping loop
        let (ping_stop_tx, mut ping_stop_rx) = watch::channel(false);
        let out_tx_ping = out_tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(50));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if *ping_stop_rx.borrow() { break; }
                        let payload = serde_json::json!({"method":"ping"}).to_string();
                        let _ = out_tx_ping.send(Message::Text(payload)).await;
                    }
                    _ = ping_stop_rx.changed() => {
                        if *ping_stop_rx.borrow() { break; }
                    }
                }
            }
        });

        // read loop
        let mut disconnect_reason = "eof".to_string();
        loop {
            tokio::select! {
                _ = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        let _ = ping_stop_tx.send(true);
                        let _ = out_tx.send(Message::Close(None)).await;
                        drop(out_tx);
                        let _ = writer.await;
                        return Ok(());
                    }
                }
                msg = read.next() => {
                    match msg {
                        None => { disconnect_reason = "eof".into(); break; }
                        Some(Err(e)) => { disconnect_reason = format!("read error: {e}"); break; }
                        Some(Ok(Message::Text(t))) => {
                            if t == "Websocket connection established." {
                                continue;
                            }
                            let v: Value = match serde_json::from_str(&t) {
                                Ok(v) => v,
                                Err(_) => continue,
                            };
                            if v.get("channel").and_then(|c| c.as_str()) == Some("pong") {
                                continue;
                            }
                            let _ = tx.send(WsEvent::Message { msg: v }).await;
                        }
                        Some(Ok(Message::Binary(_))) => {}
                        Some(Ok(Message::Ping(_))) => {}
                        Some(Ok(Message::Pong(_))) => {}
                        Some(Ok(Message::Close(_))) => { disconnect_reason = "close".into(); break; }
                        Some(Ok(_)) => {}
                    }
                }
            }
        }

        let _ = ping_stop_tx.send(true);
        drop(out_tx);
        let _ = writer.await;
        let _ = tx
            .send(WsEvent::Disconnected {
                reason: disconnect_reason.clone(),
            })
            .await;

        warn!("ws disconnected: {disconnect_reason}; reconnecting in {:?}", backoff);
        tokio::select! {
            _ = tokio::time::sleep(backoff) => {},
            _ = stop_rx.changed() => {},
        }
        backoff = (backoff * 2).min(Duration::from_secs(10));
    }
}
