use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, UnixListener};
use tokio::sync::{RwLock, watch};
use tracing::{info, warn};

use crate::client::{pid_path, socket_path, tcp_addr_path};
use crate::proto::{GetKind, Request, Response};

#[derive(Debug, Default)]
struct Cache {
    all_mids: Option<Value>,
    l2_book: BTreeMap<String, Value>,
    user_state: BTreeMap<String, Value>,
    open_orders: BTreeMap<String, Value>,
}

#[derive(Debug, Clone)]
struct SubInfo {
    key: String,
    subscription: Value,
    connected: bool,
    last_event_ts_ms: Option<i64>,
    last_disconnect: Option<String>,
}

#[derive(Debug)]
struct State {
    start: Instant,
    testnet: bool,
    ws_url: String,
    cache: RwLock<Cache>,
    subs: RwLock<BTreeMap<String, SubInfo>>,
    stop_tx: watch::Sender<bool>,
}

pub async fn run(testnet: bool) -> Result<()> {
    // Best-effort daemonization: create a new session.
    if std::env::var("HL_SERVER_DAEMON").ok().as_deref() == Some("1") {
        unsafe {
            libc::setsid();
            // Ignore SIGHUP.
            libc::signal(libc::SIGHUP, libc::SIG_IGN);
        }
    }

    let paths = hl_core::paths::Paths::resolve()?;
    paths.ensure_dirs()?;

    let ws = if testnet {
        hl_core::ws::WsClient::new_testnet()?
    } else {
        hl_core::ws::WsClient::new_mainnet()?
    };

    let info_http = if testnet {
        hl_core::info::InfoClient::new_testnet()?
    } else {
        hl_core::info::InfoClient::new_mainnet()?
    };

    let (stop_tx, stop_rx) = watch::channel(false);

    let st = Arc::new(State {
        start: Instant::now(),
        testnet,
        ws_url: ws.ws_url().to_string(),
        cache: RwLock::new(Cache::default()),
        subs: RwLock::new(BTreeMap::new()),
        stop_tx,
    });

    // Ensure pidfile exists.
    let pidfile = pid_path(&paths.home, testnet);
    let pid = std::process::id();
    std::fs::write(&pidfile, format!("{pid}\n")).ok();

    // Always subscribe to allMids.
    ensure_sub_all_mids(st.clone(), ws.clone(), stop_rx.clone(), info_http.clone()).await?;

    // Bind IPC.
    let sock_path = socket_path(&paths.home, testnet);
    let tcp_path = tcp_addr_path(&paths.home, testnet);

    // Try unix socket first; if it fails, fall back to TCP.
    let ipc = match bind_unix(&sock_path) {
        Ok(l) => {
            // clear tcp addr file
            let _ = std::fs::remove_file(&tcp_path);
            IpcListener::Unix(l, sock_path)
        }
        Err(e) => {
            warn!("unix socket bind failed ({e:#}); falling back to tcp");
            let (l, addr) = bind_tcp().await?;
            std::fs::write(&tcp_path, format!("{addr}\n")).ok();
            IpcListener::Tcp(l, tcp_path)
        }
    };

    info!(testnet = testnet, "hl-server started");

    tokio::select! {
        r = ipc_loop(ipc, st.clone(), ws.clone(), stop_rx.clone(), info_http.clone()) => { r?; }
        _ = shutdown_signal() => {
            info!("shutdown signal");
        }
    }

    let _ = st.stop_tx.send(true);
    crate::control::cleanup_ipc_files(testnet).ok();
    let _ = std::fs::remove_file(pidfile);
    Ok(())
}

enum IpcListener {
    Unix(UnixListener, PathBuf),
    Tcp(TcpListener, PathBuf),
}

fn bind_unix(p: &PathBuf) -> Result<UnixListener> {
    // Remove stale socket file.
    let _ = std::fs::remove_file(p);
    let l = UnixListener::bind(p).context("bind unix")?;
    Ok(l)
}

async fn bind_tcp() -> Result<(TcpListener, String)> {
    // Prefer a stable port but allow 0.
    let ports = [32145u16, 0u16];
    for port in ports {
        if let Ok(l) = TcpListener::bind(("127.0.0.1", port)).await {
            let addr = l.local_addr()?;
            return Ok((l, addr.to_string()));
        }
    }
    anyhow::bail!("failed to bind tcp")
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("sigterm");
        let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
            .expect("sigint");
        tokio::select! {
            _ = sigterm.recv() => {}
            _ = sigint.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn ipc_loop(
    ipc: IpcListener,
    st: Arc<State>,
    ws: hl_core::ws::WsClient,
    stop_rx: watch::Receiver<bool>,
    info_http: hl_core::info::InfoClient,
) -> Result<()> {
    match ipc {
        IpcListener::Unix(l, sock_path) => loop {
            let (conn, _) = l.accept().await?;
            let st2 = st.clone();
            let ws2 = ws.clone();
            let stop2 = stop_rx.clone();
            let info2 = info_http.clone();
            tokio::spawn(async move {
                let _ = handle_conn(conn, st2, ws2, stop2, info2).await;
            });
            // keep sock_path alive
            let _ = &sock_path;
        },
        IpcListener::Tcp(l, addr_path) => loop {
            let (conn, _) = l.accept().await?;
            let st2 = st.clone();
            let ws2 = ws.clone();
            let stop2 = stop_rx.clone();
            let info2 = info_http.clone();
            tokio::spawn(async move {
                let _ = handle_conn(conn, st2, ws2, stop2, info2).await;
            });
            let _ = &addr_path;
        },
    }
}

async fn handle_conn<S>(
    conn: S,
    st: Arc<State>,
    ws: hl_core::ws::WsClient,
    stop_rx: watch::Receiver<bool>,
    info_http: hl_core::info::InfoClient,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (r, mut w) = tokio::io::split(conn);
    let mut br = BufReader::new(r);
    let mut line = String::new();
    br.read_line(&mut line).await?;
    if line.trim().is_empty() {
        return Ok(());
    }
    let req: Request = match serde_json::from_str(line.trim()) {
        Ok(v) => v,
        Err(e) => {
            let resp = Response::err(format!("bad request: {e}"));
            w.write_all((serde_json::to_string(&resp)? + "\n").as_bytes())
                .await?;
            return Ok(());
        }
    };

    let resp = handle_req(req, st, ws, stop_rx, info_http).await;
    let out = serde_json::to_string(&resp)? + "\n";
    w.write_all(out.as_bytes()).await?;
    w.flush().await?;
    Ok(())
}

async fn handle_req(
    req: Request,
    st: Arc<State>,
    ws: hl_core::ws::WsClient,
    stop_rx: watch::Receiver<bool>,
    info_http: hl_core::info::InfoClient,
) -> Response {
    match req {
        Request::Status => {
            let meta = build_status_meta(st).await;
            Response::ok(None, Some(meta))
        }
        Request::Get { kind, coin, user } => match kind {
            GetKind::AllMids => {
                // always subscribed
                let mut source = "cache";
                let data = {
                    let c = st.cache.read().await;
                    c.all_mids.clone()
                };
                let data = if data.is_none() {
                    source = "http";
                    info_http.all_mids().await.ok()
                } else {
                    data
                };

                let meta = serde_json::json!({"source": source});
                match data {
                    Some(v) => Response::ok(Some(v), Some(meta)),
                    None => Response::err("allMids unavailable"),
                }
            }
            GetKind::L2Book => {
                let coin = match coin {
                    Some(c) => c,
                    None => return Response::err("missing coin"),
                };
                if let Err(e) = ensure_sub_l2_book(st.clone(), ws, stop_rx, info_http.clone(), &coin).await {
                    return Response::err(format!("ensure l2Book sub failed: {e:#}"));
                }

                let mut source = "cache";
                let data = {
                    let c = st.cache.read().await;
                    c.l2_book.get(&coin).cloned()
                };
                let data = if data.is_none() {
                    source = "http";
                    info_http.l2_book(&coin).await.ok()
                } else {
                    data
                };
                let meta = serde_json::json!({"source": source, "coin": coin});
                match data {
                    Some(v) => Response::ok(Some(v), Some(meta)),
                    None => Response::err("l2Book unavailable"),
                }
            }
            GetKind::UserState => {
                let user = match user {
                    Some(u) => u,
                    None => return Response::err("missing user"),
                };
                if let Err(e) = ensure_sub_user_state(st.clone(), ws, stop_rx, info_http.clone(), &user).await {
                    return Response::err(format!("ensure userState sub failed: {e:#}"));
                }

                let mut source = "cache";
                let data = {
                    let c = st.cache.read().await;
                    c.user_state.get(&user).cloned()
                };
                let data = if data.is_none() {
                    source = "http";
                    info_http.user_state(&user).await.ok()
                } else {
                    data
                };
                let meta = serde_json::json!({"source": source, "user": user});
                match data {
                    Some(v) => Response::ok(Some(v), Some(meta)),
                    None => Response::err("userState unavailable"),
                }
            }
            GetKind::OpenOrders => {
                let user = match user {
                    Some(u) => u,
                    None => return Response::err("missing user"),
                };
                if let Err(e) = ensure_sub_open_orders(st.clone(), ws, stop_rx, info_http.clone(), &user).await {
                    return Response::err(format!("ensure openOrders sub failed: {e:#}"));
                }

                let mut source = "cache";
                let data = {
                    let c = st.cache.read().await;
                    c.open_orders.get(&user).cloned()
                };
                let data = if data.is_none() {
                    source = "http";
                    info_http.open_orders(&user).await.ok()
                } else {
                    data
                };
                let meta = serde_json::json!({"source": source, "user": user});
                match data {
                    Some(v) => Response::ok(Some(v), Some(meta)),
                    None => Response::err("openOrders unavailable"),
                }
            }
        },
    }
}

async fn build_status_meta(st: Arc<State>) -> Value {
    let uptime_ms = st.start.elapsed().as_millis() as u64;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let subs = st.subs.read().await;
    let subs_out: Vec<Value> = subs
        .values()
        .cloned()
        .map(|s| {
            serde_json::json!({
                "key": s.key,
                "subscription": s.subscription,
                "connected": s.connected,
                "lastEventTs": s.last_event_ts_ms,
                "lastDisconnect": s.last_disconnect,
            })
        })
        .collect();

    serde_json::json!({
        "pid": std::process::id(),
        "testnet": st.testnet,
        "wsUrl": st.ws_url,
        "uptimeMs": uptime_ms,
        "nowMs": now_ms,
        "subs": subs_out,
    })
}

fn mark_sub_event(subs: &mut BTreeMap<String, SubInfo>, key: &str, connected: Option<bool>, disconnect: Option<String>) {
    if let Some(s) = subs.get_mut(key) {
        if let Some(c) = connected {
            s.connected = c;
        }
        s.last_event_ts_ms = Some(now_ms());
        if let Some(d) = disconnect {
            s.last_disconnect = Some(d);
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

async fn ensure_sub_all_mids(
    st: Arc<State>,
    ws: hl_core::ws::WsClient,
    stop_rx: watch::Receiver<bool>,
    info_http: hl_core::info::InfoClient,
) -> Result<()> {
    let key = "allMids".to_string();
    {
        let mut subs = st.subs.write().await;
        if subs.contains_key(&key) {
            return Ok(());
        }
        subs.insert(
            key.clone(),
            SubInfo {
                key: key.clone(),
                subscription: serde_json::json!({"type":"allMids"}),
                connected: false,
                last_event_ts_ms: None,
                last_disconnect: None,
            },
        );
    }

    // Seed cache from HTTP.
    if let Ok(m) = info_http.all_mids().await {
        let mut c = st.cache.write().await;
        c.all_mids = Some(m);
    }

    spawn_sub_task(st, ws, stop_rx, key, vec![serde_json::json!({"type":"allMids"})], |st, msg| async move {
        if msg.get("channel").and_then(|v| v.as_str()) == Some("allMids") {
            if let Some(mids) = msg.get("data").and_then(|d| d.get("mids")) {
                let mut c = st.cache.write().await;
                c.all_mids = Some(mids.clone());
            }
        }
    }).await;

    Ok(())
}

async fn ensure_sub_l2_book(
    st: Arc<State>,
    ws: hl_core::ws::WsClient,
    stop_rx: watch::Receiver<bool>,
    info_http: hl_core::info::InfoClient,
    coin: &str,
) -> Result<()> {
    let key = format!("l2Book:{coin}");
    {
        let mut subs = st.subs.write().await;
        if subs.contains_key(&key) {
            return Ok(());
        }
        subs.insert(
            key.clone(),
            SubInfo {
                key: key.clone(),
                subscription: serde_json::json!({"type":"l2Book","coin":coin}),
                connected: false,
                last_event_ts_ms: None,
                last_disconnect: None,
            },
        );
    }

    if let Ok(b) = info_http.l2_book(coin).await {
        let mut c = st.cache.write().await;
        c.l2_book.insert(coin.to_string(), b);
    }

    let coin_s = coin.to_string();
    spawn_sub_task(st, ws, stop_rx, key, vec![serde_json::json!({"type":"l2Book","coin":coin_s.clone()})], move |st, msg| {
        let coin_s = coin_s.clone();
        async move {
            if msg.get("channel").and_then(|v| v.as_str()) == Some("l2Book") {
                if let Some(data) = msg.get("data") {
                    let mut c = st.cache.write().await;
                    c.l2_book.insert(coin_s, data.clone());
                }
            }
        }
    }).await;

    Ok(())
}

async fn ensure_sub_user_state(
    st: Arc<State>,
    ws: hl_core::ws::WsClient,
    stop_rx: watch::Receiver<bool>,
    info_http: hl_core::info::InfoClient,
    user: &str,
) -> Result<()> {
    let key = format!("webData2:{user}");
    {
        let mut subs = st.subs.write().await;
        if subs.contains_key(&key) {
            return Ok(());
        }
        subs.insert(
            key.clone(),
            SubInfo {
                key: key.clone(),
                subscription: serde_json::json!({"type":"webData2","user":user}),
                connected: false,
                last_event_ts_ms: None,
                last_disconnect: None,
            },
        );
    }

    if let Ok(s) = info_http.user_state(user).await {
        let mut c = st.cache.write().await;
        c.user_state.insert(user.to_string(), s);
    }

    let user_s = user.to_string();
    spawn_sub_task(st, ws, stop_rx, key, vec![serde_json::json!({"type":"webData2","user":user_s.clone()})], move |st, msg| {
        let user_s = user_s.clone();
        async move {
            if msg.get("channel").and_then(|v| v.as_str()) == Some("webData2") {
                if let Some(data) = msg.get("data") {
                    let mut c = st.cache.write().await;
                    c.user_state.insert(user_s, data.clone());
                }
            }
        }
    }).await;

    Ok(())
}

async fn ensure_sub_open_orders(
    st: Arc<State>,
    ws: hl_core::ws::WsClient,
    stop_rx: watch::Receiver<bool>,
    info_http: hl_core::info::InfoClient,
    user: &str,
) -> Result<()> {
    let key = format!("orderUpdates:{user}");
    {
        let mut subs = st.subs.write().await;
        if subs.contains_key(&key) {
            return Ok(());
        }
        subs.insert(
            key.clone(),
            SubInfo {
                key: key.clone(),
                subscription: serde_json::json!({"type":"orderUpdates","user":user}),
                connected: false,
                last_event_ts_ms: None,
                last_disconnect: None,
            },
        );
    }

    if let Ok(o) = info_http.open_orders(user).await {
        let mut c = st.cache.write().await;
        c.open_orders.insert(user.to_string(), o);
    }

    let user_s = user.to_string();
    spawn_sub_task(st, ws, stop_rx, key, vec![serde_json::json!({"type":"orderUpdates","user":user_s.clone()})], move |st, msg| {
        let user_s = user_s.clone();
        async move {
            if msg.get("channel").and_then(|v| v.as_str()) == Some("orderUpdates") {
                if let Some(data) = msg.get("data") {
                    let mut c = st.cache.write().await;
                    // Normalize: store either array (snapshot) or update object.
                    c.open_orders.insert(user_s, data.clone());
                }
            }
        }
    }).await;

    Ok(())
}

async fn spawn_sub_task<F, Fut>(
    st: Arc<State>,
    ws: hl_core::ws::WsClient,
    stop_rx: watch::Receiver<bool>,
    key: String,
    subs: Vec<Value>,
    handler: F,
) where
    F: Fn(Arc<State>, Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let mut rx = ws.spawn(subs, stop_rx);
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            match ev {
                hl_core::ws::WsEvent::Connected => {
                    let mut subs = st.subs.write().await;
                    mark_sub_event(&mut subs, &key, Some(true), None);
                }
                hl_core::ws::WsEvent::Disconnected { reason } => {
                    let mut subs = st.subs.write().await;
                    mark_sub_event(&mut subs, &key, Some(false), Some(reason));
                }
                hl_core::ws::WsEvent::Message { msg } => {
                    {
                        let mut subs = st.subs.write().await;
                        mark_sub_event(&mut subs, &key, None, None);
                    }
                    handler(st.clone(), msg).await;
                }
            }
        }
    });
}
