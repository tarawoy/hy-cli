use anyhow::Result;
use hl_core::{info::InfoClient, ws::{WsClient, WsEvent}};
use hl_tui::{TableCell, TableView};
use ratatui::layout::Constraint;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, watch};

pub fn stdout_is_tty() -> bool {
    std::io::stdout().is_terminal()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub async fn stream_json_lines(mut rx: mpsc::Receiver<WsEvent>) -> Result<()> {
    while let Some(ev) = rx.recv().await {
        let out = match ev {
            WsEvent::Connected => serde_json::json!({"ts": now_ms(), "event": "connected"}),
            WsEvent::Disconnected { reason } => {
                serde_json::json!({"ts": now_ms(), "event": "disconnected", "reason": reason})
            }
            WsEvent::Message { msg } => serde_json::json!({"ts": now_ms(), "event": "message", "msg": msg}),
        };
        println!("{}", serde_json::to_string(&out)?);
    }
    Ok(())
}

pub async fn watch_asset_price(coin: String, testnet: bool, info: &InfoClient, ws: WsClient, json: bool) -> Result<()> {
    if !json && stdout_is_tty() {
        if let Some(sc) = hl_server::client::Client::try_connect(testnet).await {
            return crate::watch_server::asset_price(coin, sc, info).await;
        }
    }

    let (stop_tx, stop_rx) = watch::channel(false);
    let subs = vec![serde_json::json!({"type":"allMids"})];
    let rx = ws.spawn(subs, stop_rx);

    if json || !stdout_is_tty() {
        return stream_json_lines(rx).await;
    }

    // initial
    let mut last_mid: Option<String> = None;
    if let Ok(mids) = info.all_mids().await {
        last_mid = mids.get(&coin).and_then(|v| v.as_str()).map(|s| s.to_string());
    }

    let (ui_tx, ui_rx) = mpsc::channel(16);
    tokio::spawn(async move {
        let _ = hl_tui::run_table(ui_rx).await;
        let _ = stop_tx.send(true);
    });

    let mut status = "connecting…".to_string();
    let mut msg_rx = rx;
    // push initial
    let _ = ui_tx
        .send(TableView {
            title: format!("{coin} price (-w)"),
            help: "q/Esc quit".into(),
            status: status.clone(),
            headers: vec!["coin".into(), "mid".into()],
            widths: vec![Constraint::Length(12), Constraint::Min(10)],
            rows: vec![vec![TableCell::plain(coin.clone()), TableCell::yellow(last_mid.clone().unwrap_or("?".into()))]],
        })
        .await;

    while let Some(ev) = msg_rx.recv().await {
        match ev {
            WsEvent::Connected => status = "connected".into(),
            WsEvent::Disconnected { reason } => status = format!("reconnecting ({reason})"),
            WsEvent::Message { msg } => {
                if msg.get("channel").and_then(|v| v.as_str()) == Some("allMids") {
                    if let Some(mid) = msg
                        .get("data")
                        .and_then(|d| d.get("mids"))
                        .and_then(|m| m.get(&coin))
                        .and_then(|v| v.as_str())
                    {
                        last_mid = Some(mid.to_string());
                    }
                }
            }
        }

        let row_mid = last_mid.clone().unwrap_or("?".into());
        let _ = ui_tx
            .send(TableView {
                title: format!("{coin} price (-w)"),
                help: "q/Esc quit".into(),
                status: status.clone(),
                headers: vec!["coin".into(), "mid".into()],
                widths: vec![Constraint::Length(12), Constraint::Min(10)],
                rows: vec![vec![TableCell::plain(coin.clone()), TableCell::yellow(row_mid)]],
            })
            .await;
    }
    Ok(())
}

pub async fn watch_asset_book(coin: String, testnet: bool, info: &InfoClient, ws: WsClient, json: bool) -> Result<()> {
    if !json && stdout_is_tty() {
        if let Some(sc) = hl_server::client::Client::try_connect(testnet).await {
            return crate::watch_server::asset_book(coin, sc, info).await;
        }
    }

    let (stop_tx, stop_rx) = watch::channel(false);
    let subs = vec![serde_json::json!({"type":"l2Book","coin": coin.clone()})];
    let rx = ws.spawn(subs, stop_rx);

    if json || !stdout_is_tty() {
        return stream_json_lines(rx).await;
    }

    // initial snapshot
    let mut last_book: Option<Value> = info.l2_book(&coin).await.ok();

    let (ui_tx, ui_rx) = mpsc::channel(16);
    tokio::spawn(async move {
        let _ = hl_tui::run_table(ui_rx).await;
        let _ = stop_tx.send(true);
    });

    let mut status = "connecting…".to_string();
    let mut msg_rx = rx;

    while let Some(ev) = msg_rx.recv().await {
        match ev {
            WsEvent::Connected => status = "connected".into(),
            WsEvent::Disconnected { reason } => status = format!("reconnecting ({reason})"),
            WsEvent::Message { msg } => {
                if msg.get("channel").and_then(|v| v.as_str()) == Some("l2Book") {
                    last_book = msg.get("data").cloned();
                }
            }
        }

        let mut rows: Vec<Vec<TableCell>> = Vec::new();
        // Expect {coin, levels:[[bids],[asks]], time}
        if let Some(data) = last_book.as_ref() {
            if let Some(levels) = data.get("levels").and_then(|v| v.as_array()) {
                let bids = levels.get(0).and_then(|v| v.as_array()).cloned().unwrap_or_default();
                let asks = levels.get(1).and_then(|v| v.as_array()).cloned().unwrap_or_default();
                rows.push(vec![TableCell::dim("BIDS"), TableCell::dim(""), TableCell::dim("ASKS"), TableCell::dim("")]);
                for i in 0..10 {
                    let b = bids.get(i);
                    let a = asks.get(i);
                    let (bpx, bsz) = parse_l2_level(b);
                    let (apx, asz) = parse_l2_level(a);
                    rows.push(vec![
                        TableCell::green(bpx),
                        TableCell::green(bsz),
                        TableCell::red(apx),
                        TableCell::red(asz),
                    ]);
                }
            }
        }

        if rows.is_empty() {
            rows.push(vec![TableCell::plain("waiting for book…"), TableCell::plain(""), TableCell::plain(""), TableCell::plain("")]);
        }

        let _ = ui_tx
            .send(TableView {
                title: format!("{coin} book (-w)"),
                help: "q/Esc quit".into(),
                status: status.clone(),
                headers: vec!["bidPx".into(), "bidSz".into(), "askPx".into(), "askSz".into()],
                widths: vec![
                    Constraint::Length(14),
                    Constraint::Length(12),
                    Constraint::Length(14),
                    Constraint::Length(12),
                ],
                rows,
            })
            .await;
    }
    Ok(())
}

pub(crate) fn parse_l2_level(v: Option<&Value>) -> (String, String) {
    // WS uses objects: {px, sz, n}. HTTP uses array [px, sz, n]? (older).
    if let Some(v) = v {
        if let Some(px) = v.get("px").and_then(|x| x.as_str()) {
            let sz = v.get("sz").and_then(|x| x.as_str()).unwrap_or("?");
            return (px.into(), sz.into());
        }
        if let Some(arr) = v.as_array() {
            let px = arr.get(0).and_then(|x| x.as_str()).unwrap_or("?");
            let sz = arr.get(1).and_then(|x| x.as_str()).unwrap_or("?");
            return (px.into(), sz.into());
        }
    }
    ("".into(), "".into())
}

pub async fn watch_account_webdata(
    title: String,
    user: String,
    testnet: bool,
    info: &InfoClient,
    ws: WsClient,
    json: bool,
    kind: AccountKind,
) -> Result<()> {
    if !json && stdout_is_tty() {
        if let Some(sc) = hl_server::client::Client::try_connect(testnet).await {
            return crate::watch_server::account_webdata(title, user, kind, sc, info).await;
        }
    }

    let (stop_tx, stop_rx) = watch::channel(false);
    let subs = vec![serde_json::json!({"type":"webData2","user": user.clone()})];
    let rx = ws.spawn(subs, stop_rx);

    if json || !stdout_is_tty() {
        return stream_json_lines(rx).await;
    }

    // initial user state
    let mut last_state = info.user_state(&user).await.ok();

    let (ui_tx, ui_rx) = mpsc::channel(16);
    tokio::spawn(async move {
        let _ = hl_tui::run_table(ui_rx).await;
        let _ = stop_tx.send(true);
    });

    let mut status = "connecting…".to_string();
    let mut msg_rx = rx;

    while let Some(ev) = msg_rx.recv().await {
        match ev {
            WsEvent::Connected => status = "connected".into(),
            WsEvent::Disconnected { reason } => status = format!("reconnecting ({reason})"),
            WsEvent::Message { msg } => {
                if msg.get("channel").and_then(|v| v.as_str()) == Some("webData2") {
                    // take msg.data; but keep compatibility with userState shape.
                    if let Some(d) = msg.get("data") {
                        last_state = Some(d.clone());
                    }
                }
            }
        }

        let view = match kind {
            AccountKind::Balances => view_balances(&title, &status, last_state.as_ref()),
            AccountKind::Positions => view_positions(&title, &status, last_state.as_ref()),
            AccountKind::Portfolio => view_portfolio(&title, &status, last_state.as_ref()),
        };
        let _ = ui_tx.send(view).await;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub enum AccountKind {
    Balances,
    Positions,
    Portfolio,
}

pub(crate) fn view_balances(title: &str, status: &str, st: Option<&Value>) -> TableView {
    let mut rows: Vec<Vec<TableCell>> = Vec::new();

    if let Some(bals) = st
        .and_then(|s| s.get("spotState"))
        .and_then(|s| s.get("balances"))
        .and_then(|v| v.as_array())
    {
        for b in bals {
            let coin = b.get("coin").and_then(|v| v.as_str()).unwrap_or("?");
            let total = b.get("total").and_then(|v| v.as_str()).unwrap_or("?");
            let hold = b.get("hold").and_then(|v| v.as_str()).unwrap_or("?");
            rows.push(vec![
                TableCell::plain(coin),
                TableCell::plain(total),
                TableCell::dim(hold),
            ]);
        }
    }

    if rows.is_empty() {
        rows.push(vec![TableCell::plain("(no balances)"), TableCell::plain(""), TableCell::plain("")]);
    }

    TableView {
        title: title.into(),
        help: "q/Esc quit".into(),
        status: status.into(),
        headers: vec!["coin".into(), "total".into(), "hold".into()],
        widths: vec![Constraint::Length(10), Constraint::Length(18), Constraint::Length(18)],
        rows,
    }
}

pub(crate) fn view_positions(title: &str, status: &str, st: Option<&Value>) -> TableView {
    let mut rows: Vec<Vec<TableCell>> = Vec::new();

    let positions = st
        .and_then(|s| s.get("assetPositions"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for ap in positions {
        let p = ap.get("position").unwrap_or(&ap);
        let coin = p.get("coin").and_then(|v| v.as_str()).unwrap_or("?");
        let szi = p.get("szi").and_then(|v| v.as_str()).unwrap_or("?");
        let entry = p.get("entryPx").and_then(|v| v.as_str()).unwrap_or("?");
        let upnl = p.get("unrealizedPnl").and_then(|v| v.as_str()).unwrap_or("?");
        let upnl_cell = if upnl.starts_with('-') { TableCell::red(upnl) } else { TableCell::green(upnl) };
        rows.push(vec![
            TableCell::plain(coin),
            TableCell::plain(szi),
            TableCell::dim(entry),
            upnl_cell,
        ]);
    }

    if rows.is_empty() {
        rows.push(vec![TableCell::plain("(no positions)"), TableCell::plain(""), TableCell::plain(""), TableCell::plain("")]);
    }

    TableView {
        title: title.into(),
        help: "q/Esc quit".into(),
        status: status.into(),
        headers: vec!["coin".into(), "szi".into(), "entry".into(), "uPnL".into()],
        widths: vec![Constraint::Length(10), Constraint::Length(14), Constraint::Length(14), Constraint::Length(14)],
        rows,
    }
}

pub(crate) fn view_portfolio(title: &str, status: &str, st: Option<&Value>) -> TableView {
    let mut rows: Vec<Vec<TableCell>> = Vec::new();

    if let Some(ms) = st.and_then(|s| s.get("marginSummary")) {
        for k in ["accountValue", "totalMarginUsed", "totalNtlPos", "totalRawUsd"] {
            if let Some(v) = ms.get(k).and_then(|v| v.as_str()) {
                rows.push(vec![TableCell::plain(k), TableCell::yellow(v)]);
            }
        }
    }

    if let Some(ss) = st.and_then(|s| s.get("spotState")) {
        if let Some(eq) = ss.get("equity").and_then(|v| v.as_str()) {
            rows.push(vec![TableCell::plain("spotEquity"), TableCell::yellow(eq)]);
        }
    }

    if let Some(w) = st.and_then(|s| s.get("withdrawable")).and_then(|v| v.as_str()) {
        rows.push(vec![TableCell::plain("withdrawable"), TableCell::yellow(w)]);
    }

    if rows.is_empty() {
        rows.push(vec![TableCell::plain("waiting for state…"), TableCell::plain("")]);
    }

    TableView {
        title: title.into(),
        help: "q/Esc quit".into(),
        status: status.into(),
        headers: vec!["key".into(), "value".into()],
        widths: vec![Constraint::Length(18), Constraint::Min(10)],
        rows,
    }
}

pub async fn watch_account_orders(user: String, testnet: bool, info: &InfoClient, ws: WsClient, json: bool) -> Result<()> {
    if !json && stdout_is_tty() {
        if let Some(sc) = hl_server::client::Client::try_connect(testnet).await {
            return crate::watch_server::account_orders(user, sc, info).await;
        }
    }

    let (stop_tx, stop_rx) = watch::channel(false);
    let subs = vec![serde_json::json!({"type":"orderUpdates","user": user.clone()})];
    let rx = ws.spawn(subs, stop_rx);

    if json || !stdout_is_tty() {
        return stream_json_lines(rx).await;
    }

    // initial
    let mut orders: Vec<Value> = info
        .open_orders(&user)
        .await
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();

    let (ui_tx, ui_rx) = mpsc::channel(16);
    tokio::spawn(async move {
        let _ = hl_tui::run_table(ui_rx).await;
        let _ = stop_tx.send(true);
    });

    let mut status = "connecting…".to_string();
    let mut msg_rx = rx;

    // also maintain by oid for incremental updates when possible
    let mut by_oid: BTreeMap<String, Value> = BTreeMap::new();
    for o in &orders {
        if let Some(oid) = o.get("oid").map(|v| v.to_string()) {
            by_oid.insert(oid, o.clone());
        }
    }

    while let Some(ev) = msg_rx.recv().await {
        match ev {
            WsEvent::Connected => status = "connected".into(),
            WsEvent::Disconnected { reason } => status = format!("reconnecting ({reason})"),
            WsEvent::Message { msg } => {
                if msg.get("channel").and_then(|v| v.as_str()) == Some("orderUpdates") {
                    // best-effort: data could be array of orders, or a single update object.
                    if let Some(arr) = msg.get("data").and_then(|v| v.as_array()) {
                        orders = arr.to_vec();
                        by_oid.clear();
                        for o in &orders {
                            if let Some(oid) = o.get("oid").map(|v| v.to_string()) {
                                by_oid.insert(oid, o.clone());
                            }
                        }
                    } else if let Some(o) = msg.get("data").and_then(|v| v.get("order")) {
                        let oid = o.get("oid").map(|v| v.to_string()).unwrap_or_else(|| "?".into());
                        by_oid.insert(oid, o.clone());
                        orders = by_oid.values().cloned().collect();
                    } else if let Some(o) = msg.get("data").and_then(|v| v.as_object()) {
                        // If it looks like an order itself, insert.
                        let oid = o.get("oid").map(|v| v.to_string()).unwrap_or_else(|| "?".into());
                        by_oid.insert(oid, Value::Object(o.clone()));
                        orders = by_oid.values().cloned().collect();
                    }
                }
            }
        }

        let mut rows: Vec<Vec<TableCell>> = Vec::new();
        if orders.is_empty() {
            rows.push(vec![TableCell::plain("(no open orders)"), TableCell::plain(""), TableCell::plain(""), TableCell::plain(""), TableCell::plain("")]);
        } else {
            for o in &orders {
                let coin = o.get("coin").and_then(|v| v.as_str()).unwrap_or("?");
                let side = o.get("side").and_then(|v| v.as_str()).unwrap_or("?");
                let sz = o.get("sz").and_then(|v| v.as_str()).unwrap_or("?");
                let px = o.get("limitPx").and_then(|v| v.as_str()).unwrap_or("?");
                let oid = o.get("oid").map(|v| v.to_string()).unwrap_or_else(|| "?".into());
                let side_cell = if side.to_lowercase().starts_with('b') { TableCell::green(side) } else { TableCell::red(side) };
                rows.push(vec![
                    TableCell::plain(coin),
                    side_cell,
                    TableCell::plain(sz),
                    TableCell::dim(px),
                    TableCell::dim(oid),
                ]);
            }
        }

        let _ = ui_tx
            .send(TableView {
                title: "orders (-w)".into(),
                help: "q/Esc quit".into(),
                status: status.clone(),
                headers: vec!["coin".into(), "side".into(), "sz".into(), "px".into(), "oid".into()],
                widths: vec![
                    Constraint::Length(10),
                    Constraint::Length(6),
                    Constraint::Length(12),
                    Constraint::Length(12),
                    Constraint::Min(10),
                ],
                rows,
            })
            .await;
    }

    Ok(())
}
