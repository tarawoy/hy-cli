// Server-backed watch implementations (poll local cache instead of opening new WS connections).

use anyhow::Result;
use hl_core::info::InfoClient;
use hl_tui::{TableCell, TableView};
use ratatui::layout::Constraint;
use serde_json::Value;
use tokio::sync::{mpsc, watch};

use crate::watch::{parse_l2_level, view_balances, view_portfolio, view_positions, AccountKind};

pub async fn asset_price(coin: String, sc: hl_server::client::Client, info: &InfoClient) -> Result<()> {
    let (stop_tx, stop_rx) = watch::channel(false);

    // initial
    let mut last_mid: Option<String> = info
        .all_mids()
        .await
        .ok()
        .and_then(|m| m.get(&coin).and_then(|v| v.as_str()).map(|s| s.to_string()));

    let (ui_tx, ui_rx) = mpsc::channel(16);
    tokio::spawn(async move {
        let _ = hl_tui::run_table(ui_rx).await;
        let _ = stop_tx.send(true);
    });

    let mut status = "server".to_string();

    loop {
        if *stop_rx.borrow() {
            break;
        }

        match sc.get_all_mids().await {
            Ok(mids) => {
                if let Some(mid) = mids.get(&coin).and_then(|v| v.as_str()) {
                    last_mid = Some(mid.to_string());
                }
                status = "server (cache)".into();
            }
            Err(e) => {
                status = format!("server error: {e:#}");
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

        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }

    Ok(())
}

pub async fn asset_book(coin: String, sc: hl_server::client::Client, info: &InfoClient) -> Result<()> {
    let (stop_tx, stop_rx) = watch::channel(false);

    let mut last_book: Option<Value> = info.l2_book(&coin).await.ok();

    let (ui_tx, ui_rx) = mpsc::channel(16);
    tokio::spawn(async move {
        let _ = hl_tui::run_table(ui_rx).await;
        let _ = stop_tx.send(true);
    });

    let mut status = "server".to_string();

    loop {
        if *stop_rx.borrow() {
            break;
        }

        match sc.get_l2_book(&coin).await {
            Ok(book) => {
                last_book = Some(book);
                status = "server (cache)".into();
            }
            Err(e) => {
                status = format!("server error: {e:#}");
            }
        }

        let mut rows: Vec<Vec<TableCell>> = Vec::new();
        if let Some(data) = last_book.as_ref() {
            if let Some(levels) = data.get("levels").and_then(|v| v.as_array()) {
                let bids = levels.get(0).and_then(|v| v.as_array()).cloned().unwrap_or_default();
                let asks = levels.get(1).and_then(|v| v.as_array()).cloned().unwrap_or_default();
                rows.push(vec![TableCell::dim("BIDS"), TableCell::dim(""), TableCell::dim("ASKS"), TableCell::dim("")]);
                for i in 0..10 {
                    let (bpx, bsz) = parse_l2_level(bids.get(i));
                    let (apx, asz) = parse_l2_level(asks.get(i));
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

        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }

    Ok(())
}

pub async fn account_webdata(
    title: String,
    user: String,
    kind: AccountKind,
    sc: hl_server::client::Client,
    info: &InfoClient,
) -> Result<()> {
    let (stop_tx, stop_rx) = watch::channel(false);
    let mut last_state = info.user_state_compat(&user).await.ok();

    let (ui_tx, ui_rx) = mpsc::channel(16);
    tokio::spawn(async move {
        let _ = hl_tui::run_table(ui_rx).await;
        let _ = stop_tx.send(true);
    });

    let mut status = "server".to_string();

    loop {
        if *stop_rx.borrow() {
            break;
        }

        match sc.get_user_state(&user).await {
            Ok(st) => {
                last_state = Some(st);
                status = "server (cache)".into();
            }
            Err(e) => {
                status = format!("server error: {e:#}");
            }
        }

        let view = match kind {
            AccountKind::Balances => view_balances(&title, &status, last_state.as_ref()),
            AccountKind::Positions => view_positions(&title, &status, last_state.as_ref()),
            AccountKind::Portfolio => view_portfolio(&title, &status, last_state.as_ref()),
        };
        let _ = ui_tx.send(view).await;

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    Ok(())
}

pub async fn account_orders(user: String, sc: hl_server::client::Client, info: &InfoClient) -> Result<()> {
    let (stop_tx, stop_rx) = watch::channel(false);

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

    let mut status = "server".to_string();

    loop {
        if *stop_rx.borrow() {
            break;
        }

        match sc.get_open_orders(&user).await {
            Ok(v) => {
                if let Some(arr) = v.as_array() {
                    orders = arr.to_vec();
                }
                status = "server (cache)".into();
            }
            Err(e) => {
                status = format!("server error: {e:#}");
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
                let side_cell = if side.to_lowercase().starts_with('b') {
                    TableCell::green(side)
                } else {
                    TableCell::red(side)
                };
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

        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    }

    Ok(())
}
