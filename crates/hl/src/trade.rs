use anyhow::{Context, Result};

use crate::cli::{OrderCmd, TradeCmd};

#[derive(Debug, Clone, Copy)]
struct MarketMeta {
    asset: u32,
    sz_decimals: i32,
}

fn parse_side(side: &str) -> Result<bool> {
    match side.to_lowercase().as_str() {
        "buy" | "b" | "long" => Ok(true),
        "sell" | "s" | "short" => Ok(false),
        _ => anyhow::bail!("invalid side '{side}' (use buy/sell)"),
    }
}

fn parse_f64(s: &str, name: &str) -> Result<f64> {
    s.parse::<f64>().with_context(|| format!("parse {name}"))
}

fn parse_slippage_pct(s: &str) -> Result<f64> {
    // CLI takes percent, e.g. "1" => 1%.
    let v = parse_f64(s, "slippage")?;
    if !(0.0..=50.0).contains(&v) {
        anyhow::bail!("slippage must be between 0 and 50 (percent)");
    }
    Ok(v / 100.0)
}

async fn meta_for_coin(info: &hl_core::info::InfoClient, coin: &str) -> Result<MarketMeta> {
    let meta = info.meta().await?;
    let uni = meta
        .get("universe")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for (i, m) in uni.iter().enumerate() {
        let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if name.eq_ignore_ascii_case(coin) {
            let sd = m
                .get("szDecimals")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            return Ok(MarketMeta {
                asset: i as u32,
                sz_decimals: sd,
            });
        }
    }

    anyhow::bail!("unknown coin '{coin}'. Try: hl markets ls")
}

async fn mid_price(info: &hl_core::info::InfoClient, coin: &str) -> Result<f64> {
    let mids = info.all_mids().await?;
    let px = mids
        .get(coin)
        .and_then(|v| v.as_str())
        .context("coin not in allMids")?;
    px.parse::<f64>().context("parse mid price")
}

fn default_user(db: &hl_core::db::Db) -> Result<String> {
    if let Some(a) = db.default_account()? {
        return Ok(a.address);
    }
    anyhow::bail!("no default account configured. Run: hl account add (and optionally hl account set-default)");
}

fn parse_pct(s: &str) -> Result<Option<f64>> {
    let st = s.trim();
    if !st.ends_with('%') {
        return Ok(None);
    }
    let inner = st[..st.len() - 1].trim();
    let v: f64 = inner.parse().with_context(|| format!("parse percent '{s}'"))?;
    Ok(Some(v / 100.0))
}

async fn position_for_coin(info: &hl_core::info::InfoClient, user: &str, coin: &str) -> Result<(f64, f64, Option<f64>, Option<f64>)> {
    // Returns: (szi, entryPx, leverage, liquidationPx)
    let st = info.clearinghouse_state(user).await?;
    let positions = st
        .get("assetPositions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for ap in positions {
        let p = ap.get("position").unwrap_or(&ap);
        let c = p.get("coin").and_then(|v| v.as_str()).unwrap_or("");
        if !c.eq_ignore_ascii_case(coin) {
            continue;
        }
        let szi = p.get("szi").and_then(|v| v.as_str()).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        let entry = p.get("entryPx").and_then(|v| v.as_str()).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        let lev = p.get("leverage").and_then(|v| v.get("value")).and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok());
        let liq = p.get("liquidationPx").and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok());
        return Ok((szi, entry, lev, liq));
    }

    anyhow::bail!("no open position for {coin}");
}

async fn trigger_helper(
    info: &hl_core::info::InfoClient,
    ex: &hl_core::exchange::ExchangeClient,
    user: &str,
    testnet: bool,
    json: bool,
    is_stop_loss: bool,
    coin: &str,
    trigger_s: &str,
    ref_s: &str,
    size_opt: Option<&str>,
    limit_opt: Option<&str>,
    reduce_only: bool,
) -> Result<()> {
    let meta = meta_for_coin(info, coin).await?;

    let (pos_szi, entry_px, _lev, _liq) = position_for_coin(info, user, coin).await?;
    let side_is_buy = if pos_szi > 0.0 {
        // long position -> exits are sells
        false
    } else if pos_szi < 0.0 {
        // short position -> exits are buys
        true
    } else {
        anyhow::bail!("position size is 0 for {coin}");
    };

    let sz = if let Some(sz_s) = size_opt {
        parse_f64(sz_s, "size")?
    } else {
        pos_szi.abs()
    };

    // Determine reference price for relative triggers
    let trigger_px = if let Some(pct) = parse_pct(trigger_s)? {
        let base = match ref_s.to_lowercase().as_str() {
            "entry" => entry_px,
            "mark" => mid_price(info, coin).await?,
            other => anyhow::bail!("invalid --ref '{other}' (use entry|mark)"),
        };
        base * (1.0 + pct)
    } else {
        parse_f64(trigger_s, "trigger")?
    };

    let limit_px = if let Some(lim) = limit_opt {
        if let Some(pct) = parse_pct(lim)? {
            let base = match ref_s.to_lowercase().as_str() {
                "entry" => entry_px,
                "mark" => mid_price(info, coin).await?,
                _ => entry_px,
            };
            base * (1.0 + pct)
        } else {
            parse_f64(lim, "limit")?
        }
    } else {
        trigger_px
    };

    let tpsl = if is_stop_loss { "sl" } else { "tp" };

    let order = serde_json::json!({
        "a": meta.asset,
        "b": side_is_buy,
        "p": hl_core::exchange::float_to_wire(limit_px)?,
        "s": hl_core::exchange::float_to_wire(sz)?,
        "r": reduce_only,
        "t": {"trigger": {"isMarket": false, "triggerPx": hl_core::exchange::float_to_wire(trigger_px)?, "tpsl": tpsl}},
    });

    let action = serde_json::json!({
        "type": "order",
        "orders": [order],
        "grouping": "na",
    });

    let nonce = hl_core::exchange::now_ms();
    let resp = ex.post_action(&action, nonce).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    }

    Ok(())
}

pub async fn trade(cmd: TradeCmd, json: bool, testnet: bool, db: &mut hl_core::db::Db, info: &hl_core::info::InfoClient) -> Result<()> {
    // Trading always uses the default account (or error).
    let user = default_user(db)?;

    let signer = hl_core::signer::HlSigner::load_from_env()?;
    let signer_addr = format!("{:#x}", signer.address());
    if !signer_addr.eq_ignore_ascii_case(&user) {
        anyhow::bail!(
            "signer address mismatch: default account is {user} but signer is {signer_addr}.\n\
             Fix: hl account set-default to the signer address, or set a different HYPERLIQUID_PRIVATE_KEY."
        );
    }

    let ex = if testnet {
        hl_core::exchange::ExchangeClient::new_testnet(signer)?
    } else {
        hl_core::exchange::ExchangeClient::new_mainnet(signer)?
    };

    match cmd {
        TradeCmd::SlTrigger { coin, trigger, r#ref, size, limit, reduce_only } => {
            return trigger_helper(
                info,
                &ex,
                &user,
                testnet,
                json,
                true,
                &coin,
                &trigger,
                &r#ref,
                size.as_deref(),
                limit.as_deref(),
                reduce_only,
            )
            .await;
        }
        TradeCmd::TpTrigger { coin, trigger, r#ref, size, limit, reduce_only } => {
            return trigger_helper(
                info,
                &ex,
                &user,
                testnet,
                json,
                false,
                &coin,
                &trigger,
                &r#ref,
                size.as_deref(),
                limit.as_deref(),
                reduce_only,
            )
            .await;
        }
        TradeCmd::Order(order_cmd) => match order_cmd {
            OrderCmd::Ls => {
                let orders = info.open_orders(&user).await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&orders)?);
                    return Ok(());
                }
                let arr = orders.as_array().cloned().unwrap_or_default();
                if arr.is_empty() {
                    println!("No open orders");
                    return Ok(());
                }
                for o in arr {
                    let coin = o.get("coin").and_then(|v| v.as_str()).unwrap_or("?");
                    let side = o.get("side").and_then(|v| v.as_str()).unwrap_or("?");
                    let sz = o.get("sz").and_then(|v| v.as_str()).unwrap_or("?");
                    let px = o.get("limitPx").and_then(|v| v.as_str()).unwrap_or("?");
                    let oid = o.get("oid").map(|v| v.to_string()).unwrap_or_else(|| "?".into());
                    println!("{coin:>8}  {side:>4}  sz={sz:>10}  px={px:>12}  oid={oid}");
                }
                Ok(())
            }
            OrderCmd::Limit {
                side,
                size,
                coin,
                price,
                tif,
                reduce_only,
            } => {
                let is_buy = parse_side(&side)?;
                let sz = parse_f64(&size, "size")?;
                let px = parse_f64(&price, "price")?;
                let meta = meta_for_coin(info, &coin).await?;

                let order = serde_json::json!({
                    "a": meta.asset,
                    "b": is_buy,
                    "p": hl_core::exchange::float_to_wire(px)?,
                    "s": hl_core::exchange::float_to_wire(sz)?,
                    "r": reduce_only,
                    "t": {"limit": {"tif": tif}},
                });
                let action = serde_json::json!({
                    "type": "order",
                    "orders": [order],
                    "grouping": "na",
                });

                let nonce = hl_core::exchange::now_ms();
                let resp = ex.post_action(&action, nonce).await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&resp)?);
                } else {
                    println!("{}", serde_json::to_string_pretty(&resp)?);
                }
                Ok(())
            }
            OrderCmd::Market {
                side,
                size,
                coin,
                slippage,
                reduce_only,
            } => {
                let is_buy = parse_side(&side)?;
                let sz = parse_f64(&size, "size")?;
                let slip = parse_slippage_pct(&slippage)?;
                let meta = meta_for_coin(info, &coin).await?;

                let mid = mid_price(info, &coin).await?;
                let mut px = if is_buy { mid * (1.0 + slip) } else { mid * (1.0 - slip) };
                // Mirror hlpy: round to 5 sig figs then to decimals=(6 - szDecimals) for perps.
                px = hl_core::exchange::round_5_sigfig(px);
                let decimals = 6 - meta.sz_decimals;
                px = hl_core::exchange::round_to_decimals(px, decimals);

                let order = serde_json::json!({
                    "a": meta.asset,
                    "b": is_buy,
                    "p": hl_core::exchange::float_to_wire(px)?,
                    "s": hl_core::exchange::float_to_wire(sz)?,
                    "r": reduce_only,
                    "t": {"limit": {"tif": "Ioc"}},
                });
                let action = serde_json::json!({
                    "type": "order",
                    "orders": [order],
                    "grouping": "na",
                });

                let nonce = hl_core::exchange::now_ms();
                let resp = ex.post_action(&action, nonce).await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&resp)?);
                } else {
                    println!("{}", serde_json::to_string_pretty(&resp)?);
                }
                Ok(())
            }
            OrderCmd::StopLoss {
                side,
                size,
                coin,
                price,
                trigger,
                tpsl,
            } => {
                let is_buy = parse_side(&side)?;
                let sz = parse_f64(&size, "size")?;
                let px = parse_f64(&price, "price")?;
                let trig = parse_f64(&trigger, "trigger")?;
                let meta = meta_for_coin(info, &coin).await?;

                let grouping = if tpsl { "normalTpsl" } else { "na" };
                let order = serde_json::json!({
                    "a": meta.asset,
                    "b": is_buy,
                    "p": hl_core::exchange::float_to_wire(px)?,
                    "s": hl_core::exchange::float_to_wire(sz)?,
                    "r": false,
                    "t": {"trigger": {"isMarket": false, "triggerPx": hl_core::exchange::float_to_wire(trig)?, "tpsl": "sl"}},
                });

                let action = serde_json::json!({
                    "type": "order",
                    "orders": [order],
                    "grouping": grouping,
                });

                let nonce = hl_core::exchange::now_ms();
                let resp = ex.post_action(&action, nonce).await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&resp)?);
                } else {
                    println!("{}", serde_json::to_string_pretty(&resp)?);
                }
                Ok(())
            }
            OrderCmd::TakeProfit {
                side,
                size,
                coin,
                price,
                trigger,
                tpsl,
            } => {
                let is_buy = parse_side(&side)?;
                let sz = parse_f64(&size, "size")?;
                let px = parse_f64(&price, "price")?;
                let trig = parse_f64(&trigger, "trigger")?;
                let meta = meta_for_coin(info, &coin).await?;

                let grouping = if tpsl { "normalTpsl" } else { "na" };
                let order = serde_json::json!({
                    "a": meta.asset,
                    "b": is_buy,
                    "p": hl_core::exchange::float_to_wire(px)?,
                    "s": hl_core::exchange::float_to_wire(sz)?,
                    "r": false,
                    "t": {"trigger": {"isMarket": false, "triggerPx": hl_core::exchange::float_to_wire(trig)?, "tpsl": "tp"}},
                });

                let action = serde_json::json!({
                    "type": "order",
                    "orders": [order],
                    "grouping": grouping,
                });

                let nonce = hl_core::exchange::now_ms();
                let resp = ex.post_action(&action, nonce).await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&resp)?);
                } else {
                    println!("{}", serde_json::to_string_pretty(&resp)?);
                }
                Ok(())
            }
            OrderCmd::Configure { .. } => anyhow::bail!("order configure not implemented"),
        },

        TradeCmd::Cancel { oid } => {
            let oid = oid.context("oid is required")?;
            let oid_u64: u64 = oid.parse().context("parse oid")?;
            let orders = info.open_orders(&user).await?;
            let arr = orders.as_array().cloned().unwrap_or_default();
            let mut coin: Option<String> = None;
            for o in arr {
                if o.get("oid").and_then(|v| v.as_u64()) == Some(oid_u64) {
                    coin = o.get("coin").and_then(|v| v.as_str()).map(|s| s.to_string());
                    break;
                }
            }
            let coin = coin.context("order not found in open orders (need coin to cancel)")?;
            let meta = meta_for_coin(info, &coin).await?;

            let action = serde_json::json!({
                "type": "cancel",
                "cancels": [{"a": meta.asset, "o": oid_u64}],
            });
            let nonce = hl_core::exchange::now_ms();
            let resp = ex.post_action(&action, nonce).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            } else {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            }
            Ok(())
        }

        TradeCmd::CancelAll { coin, yes } => {
            let orders = info.open_orders(&user).await?;
            let arr = orders.as_array().cloned().unwrap_or_default();

            let mut cancels = Vec::new();
            for o in arr {
                let ocoin = o.get("coin").and_then(|v| v.as_str()).unwrap_or("");
                if let Some(ref want) = coin {
                    if !ocoin.eq_ignore_ascii_case(want) {
                        continue;
                    }
                }
                let oid_u64 = match o.get("oid").and_then(|v| v.as_u64()) {
                    Some(x) => x,
                    None => continue,
                };
                let meta = meta_for_coin(info, ocoin).await?;
                cancels.push(serde_json::json!({"a": meta.asset, "o": oid_u64}));
            }

            if cancels.is_empty() {
                if json {
                    println!("{}", serde_json::json!({"ok": true, "canceled": 0}));
                } else {
                    println!("No matching open orders");
                }
                return Ok(());
            }

            if !json && !yes {
                let msg = if let Some(c) = &coin {
                    format!("Cancel {} open orders for {c}? (y/N): ", cancels.len())
                } else {
                    format!("Cancel {} open orders? (y/N): ", cancels.len())
                };
                let s = hl_core::prompt::prompt(&msg)?;
                if !s.to_lowercase().starts_with('y') {
                    println!("Aborted");
                    return Ok(());
                }
            }

            let action = serde_json::json!({
                "type": "cancel",
                "cancels": cancels,
            });
            let nonce = hl_core::exchange::now_ms();
            let resp = ex.post_action(&action, nonce).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            } else {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            }
            Ok(())
        }

        TradeCmd::SetLeverage {
            coin,
            leverage,
            isolated,
            cross,
        } => {
            if isolated && cross {
                anyhow::bail!("use only one of --isolated or --cross");
            }
            let is_cross = if isolated { false } else { true };
            let meta = meta_for_coin(info, &coin).await?;

            let action = serde_json::json!({
                "type": "updateLeverage",
                "asset": meta.asset,
                "isCross": is_cross,
                "leverage": leverage,
            });
            let nonce = hl_core::exchange::now_ms();
            let resp = ex.post_action(&action, nonce).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            } else {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            }
            Ok(())
        }
    }
}
