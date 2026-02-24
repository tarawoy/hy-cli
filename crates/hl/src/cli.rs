use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "hl", version, about = "Hyperliquid CLI (Rust rewrite)")]
pub struct Root {
    /// Output in JSON format
    #[arg(long)]
    pub json: bool,

    /// Use testnet instead of mainnet
    #[arg(long)]
    pub testnet: bool,

    #[command(subcommand)]
    pub cmd: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    Account {
        #[command(subcommand)]
        cmd: AccountCmd,
    },
    Markets {
        #[command(subcommand)]
        cmd: MarketsCmd,
    },
    Asset {
        #[command(subcommand)]
        cmd: AssetCmd,
    },
    Trade {
        #[command(subcommand)]
        cmd: TradeCmd,
    },
    Referral {
        #[command(subcommand)]
        cmd: ReferralCmd,
    },
    Server {
        #[command(subcommand)]
        cmd: ServerCmd,
    },
}

#[derive(Subcommand, Debug)]
pub enum AccountCmd {
    Add,
    Ls,
    SetDefault,
    Remove,
    Balances { #[arg(long)] user: Option<String>, #[arg(short='w', long)] watch: bool },
    Positions { #[arg(long)] user: Option<String>, #[arg(short='w', long)] watch: bool },
    Orders { #[arg(long)] user: Option<String>, #[arg(short='w', long)] watch: bool },
    Portfolio { #[arg(long)] user: Option<String>, #[arg(short='w', long)] watch: bool },
}

#[derive(Subcommand, Debug)]
pub enum MarketsCmd { Ls, Prices }

#[derive(Subcommand, Debug)]
pub enum AssetCmd {
    Price { coin: String, #[arg(short='w', long)] watch: bool },
    Book { coin: String, #[arg(short='w', long)] watch: bool },
}

#[derive(Subcommand, Debug)]
pub enum TradeCmd {
    #[command(subcommand)]
    Order(OrderCmd),
    Cancel { oid: Option<String> },
    CancelAll { #[arg(long)] coin: Option<String>, #[arg(short='y', long)] yes: bool },
    SetLeverage { coin: String, leverage: u32, #[arg(long)] isolated: bool, #[arg(long)] cross: bool },
}

#[derive(Subcommand, Debug)]
pub enum OrderCmd {
    Ls,
    Limit {
        side: String,
        size: String,
        coin: String,
        price: String,
        #[arg(long, default_value = "Gtc")]
        tif: String,
        #[arg(long)]
        reduce_only: bool,
    },
    Market {
        side: String,
        size: String,
        coin: String,
        #[arg(long, default_value = "1")]
        slippage: String,
        #[arg(long)]
        reduce_only: bool,
    },
    StopLoss { side: String, size: String, coin: String, price: String, trigger: String, #[arg(long)] tpsl: bool },
    TakeProfit { side: String, size: String, coin: String, price: String, trigger: String, #[arg(long)] tpsl: bool },
    Configure { #[arg(long)] slippage: Option<String> },
}

#[derive(Subcommand, Debug)]
pub enum ReferralCmd { Set { code: String }, Status }

#[derive(Subcommand, Debug)]
pub enum ServerCmd {
    Start,
    Stop,
    Status,
    /// Internal: run the background daemon in the foreground (used by `hl server start`).
    #[command(hide = true)]
    Run,
}

pub async fn run() -> Result<()> {
    let args = Root::parse();

    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(false)
        .init();

    let paths = hl_core::paths::Paths::resolve()?;
    paths.ensure_dirs()?;
    let mut db = hl_core::db::Db::open(&paths.accounts_db)?;

    let info = if args.testnet {
        hl_core::info::InfoClient::new_testnet()?
    } else {
        hl_core::info::InfoClient::new_mainnet()?
    };

    let ws = if args.testnet {
        hl_core::ws::WsClient::new_testnet()?
    } else {
        hl_core::ws::WsClient::new_mainnet()?
    };

    match args.cmd {
        Command::Account { cmd } => account(cmd, args.json, args.testnet, &paths, &mut db, &info, &ws).await,
        Command::Markets { cmd } => markets(cmd, args.json, &info).await,
        Command::Asset { cmd } => asset(cmd, args.json, args.testnet, &info, &ws).await,
        Command::Trade { cmd } => crate::trade::trade(cmd, args.json, args.testnet, &mut db, &info).await,
        Command::Server { cmd } => crate::server_cmd::server(cmd, args.json, args.testnet).await,
        _ => {
            if args.json {
                println!("{}", serde_json::json!({"ok": false, "error": "not implemented"}));
            } else {
                println!("not implemented yet");
            }
            Ok(())
        }
    }
}

async fn markets(cmd: MarketsCmd, json: bool, info: &hl_core::info::InfoClient) -> Result<()> {
    match cmd {
        MarketsCmd::Ls => {
            let meta = info.meta().await?;
            // meta.universe is the canonical list of perps.
            let uni = meta
                .get("universe")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if json {
                println!("{}", serde_json::to_string_pretty(&uni)?);
                return Ok(());
            }
            if uni.is_empty() {
                println!("No markets (meta.universe empty)");
                return Ok(());
            }
            for m in uni {
                let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let sd = m
                    .get("szDecimals")
                    .and_then(|v| v.as_i64())
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "?".into());
                let ml = m
                    .get("maxLeverage")
                    .and_then(|v| v.as_i64())
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "?".into());
                println!("{name:>8}  szDecimals={sd}  maxLev={ml}");
            }
            Ok(())
        }
        MarketsCmd::Prices => {
            let mids = info.all_mids().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&mids)?);
                return Ok(());
            }
            let obj = mids
                .as_object()
                .context("allMids: expected object")?;
            // Stable-ish output.
            let mut keys: Vec<_> = obj.keys().cloned().collect();
            keys.sort();
            for k in keys {
                let px = obj.get(&k).and_then(|v| v.as_str()).unwrap_or("?");
                println!("{k:>8}  {px}");
            }
            Ok(())
        }
    }
}

async fn asset(cmd: AssetCmd, json: bool, testnet: bool, info: &hl_core::info::InfoClient, ws: &hl_core::ws::WsClient) -> Result<()> {
    match cmd {
        AssetCmd::Price { coin, watch } => {
            if watch {
                return crate::watch::watch_asset_price(coin, testnet, info, ws.clone(), json).await;
            }
            let mids = info.all_mids().await?;
            let px = mids
                .get(&coin)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "coin": coin,
                        "mid": px,
                    }))?
                );
                return Ok(());
            }
            match px {
                Some(px) => println!("{coin} mid={px}"),
                None => anyhow::bail!("coin not found in allMids: {coin}"),
            }
            Ok(())
        }
        AssetCmd::Book { coin, watch } => {
            if watch {
                return crate::watch::watch_asset_book(coin, testnet, info, ws.clone(), json).await;
            }
            let book = info.l2_book(&coin).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&book)?);
                return Ok(());
            }
            // l2Book response is usually {"coin":..., "levels":[bids, asks], ...}
            let levels = book.get("levels").and_then(|v| v.as_array());
            if let Some(levels) = levels {
                let bids = levels.get(0).and_then(|v| v.as_array()).cloned().unwrap_or_default();
                let asks = levels.get(1).and_then(|v| v.as_array()).cloned().unwrap_or_default();
                println!("{coin} book:");
                println!("  bids:");
                for lvl in bids.iter().take(10) {
                    let px = lvl.get(0).and_then(|v| v.as_str()).unwrap_or("?");
                    let sz = lvl.get(1).and_then(|v| v.as_str()).unwrap_or("?");
                    println!("    {px:>12}  {sz}");
                }
                println!("  asks:");
                for lvl in asks.iter().take(10) {
                    let px = lvl.get(0).and_then(|v| v.as_str()).unwrap_or("?");
                    let sz = lvl.get(1).and_then(|v| v.as_str()).unwrap_or("?");
                    println!("    {px:>12}  {sz}");
                }
            } else {
                // Fallback: dump raw json as compact.
                println!("{}", serde_json::to_string_pretty(&book)?);
            }
            Ok(())
        }
    }
}

async fn account(
    cmd: AccountCmd,
    json: bool,
    testnet: bool,
    paths: &hl_core::paths::Paths,
    db: &mut hl_core::db::Db,
    info: &hl_core::info::InfoClient,
    ws: &hl_core::ws::WsClient,
) -> Result<()> {
    use hl_core::prompt::{prompt, prompt_optional};

    match cmd {
        AccountCmd::Add => {
            if json {
                anyhow::bail!("account add is interactive; json mode not supported yet");
            }

            println!("Accounts DB: {}", paths.accounts_db.display());
            if let Ok(home) = std::env::var(hl_core::env::ENV_HL_HOME) {
                println!("HL_HOME override: {home}");
            }

            let alias = prompt("Alias (e.g. main): ")?;
            if alias.is_empty() {
                anyhow::bail!("alias is required");
            }

            let address = prompt("Address (0x...): ")?;
            if !address.starts_with("0x") || address.len() < 10 {
                anyhow::bail!("address must look like 0x... ");
            }

            let make_default = match prompt_optional("Set as default? (y/N): ")? {
                Some(s) if s.to_lowercase().starts_with('y') => true,
                _ => false,
            };

            // Start read-only until trading implementation lands.
            db.add_account(&alias, &address, true, make_default)?;
            println!("Added account '{alias}' {address}");
            Ok(())
        }
        AccountCmd::Ls => {
            let accts = db.list_accounts()?;
            if json {
                let out: Vec<_> = accts
                    .iter()
                    .map(|a| {
                        serde_json::json!({
                            "id": a.id,
                            "alias": a.alias,
                            "address": a.address,
                            "readOnly": a.read_only,
                            "default": a.is_default,
                            "createdAt": a.created_at,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                if accts.is_empty() {
                    println!("No accounts. Run: hl account add");
                    return Ok(());
                }
                for a in accts {
                    let d = if a.is_default { "*" } else { " " };
                    let ro = if a.read_only { "read-only" } else { "trading" };
                    println!("{d} [{}] {}  {} ({ro})", a.id, a.alias, a.address);
                }
            }
            Ok(())
        }
        AccountCmd::SetDefault => {
            if json {
                anyhow::bail!("set-default is interactive; json mode not supported yet");
            }
            let accts = db.list_accounts()?;
            if accts.is_empty() {
                anyhow::bail!("no accounts found; run hl account add");
            }
            println!("Select default account by id:");
            for a in &accts {
                let d = if a.is_default { "*" } else { " " };
                println!("{d} [{}] {}  {}", a.id, a.alias, a.address);
            }
            let id_s = prompt("id: ")?;
            let id: i64 = id_s.parse().map_err(|_| anyhow::anyhow!("invalid id"))?;
            db.set_default_by_id(id)?;
            println!("Default account set to id={id}");
            Ok(())
        }
        AccountCmd::Remove => {
            if json {
                anyhow::bail!("remove is interactive; json mode not supported yet");
            }
            let accts = db.list_accounts()?;
            if accts.is_empty() {
                anyhow::bail!("no accounts found");
            }
            println!("Select account to remove by id:");
            for a in &accts {
                let d = if a.is_default { "*" } else { " " };
                println!("{d} [{}] {}  {}", a.id, a.alias, a.address);
            }
            let id_s = prompt("id: ")?;
            let id: i64 = id_s.parse().map_err(|_| anyhow::anyhow!("invalid id"))?;
            db.remove_by_id(id)?;
            println!("Removed account id={id}");
            Ok(())
        }
        AccountCmd::Balances { user, watch } => {
            let user = resolve_user(user.as_deref(), db)?;
            if watch {
                return crate::watch::watch_account_webdata(
                    format!("balances {user} (-w)"),
                    user,
                    testnet,
                    info,
                    ws.clone(),
                    json,
                    crate::watch::AccountKind::Balances,
                )
                .await;
            }
            let perp = info.clearinghouse_state(&user).await?;
            let spot = info.spot_clearinghouse_state(&user).await?;

            if json {
                let out = serde_json::json!({
                    "clearinghouseState": perp,
                    "spotClearinghouseState": spot,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }

            // Spot balances
            if let Some(bals) = spot.get("balances").and_then(|v| v.as_array()) {
                println!("Spot balances:");
                for b in bals {
                    let coin = b.get("coin").and_then(|v| v.as_str()).unwrap_or("?");
                    let total = b.get("total").and_then(|v| v.as_str()).unwrap_or("?");
                    let hold = b.get("hold").and_then(|v| v.as_str()).unwrap_or("?");
                    println!("  {coin:>8}  total={total}  hold={hold}");
                }
            } else {
                println!("Spot balances: (none)");
            }

            // Margin summary
            if let Some(ms) = perp.get("marginSummary") {
                let av = ms.get("accountValue").and_then(|v| v.as_str()).unwrap_or("?");
                let tm = ms.get("totalMarginUsed").and_then(|v| v.as_str()).unwrap_or("?");
                let ntl = ms.get("totalNtlPos").and_then(|v| v.as_str()).unwrap_or("?");
                println!("Margin: accountValue={av}  totalMarginUsed={tm}  totalNtlPos={ntl}");
            }
            if let Some(w) = perp.get("withdrawable").and_then(|v| v.as_str()) {
                println!("Withdrawable: {w}");
            }
            Ok(())
        }
        AccountCmd::Positions { user, watch } => {
            let user = resolve_user(user.as_deref(), db)?;
            if watch {
                return crate::watch::watch_account_webdata(
                    format!("positions {user} (-w)"),
                    user,
                    testnet,
                    info,
                    ws.clone(),
                    json,
                    crate::watch::AccountKind::Positions,
                )
                .await;
            }
            let st = info.clearinghouse_state(&user).await?;
            let positions = st
                .get("assetPositions")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            if json {
                println!("{}", serde_json::to_string_pretty(&positions)?);
                return Ok(());
            }

            if positions.is_empty() {
                println!("No positions");
                return Ok(());
            }
            for ap in positions {
                let p = ap.get("position").unwrap_or(&ap);
                let coin = p.get("coin").and_then(|v| v.as_str()).unwrap_or("?");
                let szi = p.get("szi").and_then(|v| v.as_str()).unwrap_or("?");
                let entry = p.get("entryPx").and_then(|v| v.as_str()).unwrap_or("?");
                let upnl = p.get("unrealizedPnl").and_then(|v| v.as_str()).unwrap_or("?");
                let lev = p.get("leverage").and_then(|v| v.get("value")).and_then(|v| v.as_str()).unwrap_or("?");
                println!("{coin:>8}  szi={szi:>10}  entry={entry:>12}  uPnL={upnl:>12}  lev={lev}");
            }
            Ok(())
        }
        AccountCmd::Orders { user, watch } => {
            let user = resolve_user(user.as_deref(), db)?;
            if watch {
                return crate::watch::watch_account_orders(user, testnet, info, ws.clone(), json).await;
            }
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
        AccountCmd::Portfolio { user, watch } => {
            let user = resolve_user(user.as_deref(), db)?;
            if watch {
                return crate::watch::watch_account_webdata(
                    format!("portfolio {user} (-w)"),
                    user,
                    testnet,
                    info,
                    ws.clone(),
                    json,
                    crate::watch::AccountKind::Portfolio,
                )
                .await;
            }
            let perp = info.clearinghouse_state(&user).await?;
            let spot = info.spot_clearinghouse_state(&user).await?;
            let port = info.portfolio(&user).await?;
            if json {
                let out = serde_json::json!({
                    "clearinghouseState": perp,
                    "spotClearinghouseState": spot,
                    "portfolio": port,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }
            // Minimal, non-watch portfolio summary.
            if let Some(ms) = perp.get("marginSummary") {
                println!("Margin summary:");
                for k in ["accountValue", "totalMarginUsed", "totalNtlPos", "totalRawUsd"] {
                    if let Some(v) = ms.get(k).and_then(|v| v.as_str()) {
                        println!("  {k}: {v}");
                    }
                }
            }
            if let Some(bals) = spot.get("balances").and_then(|v| v.as_array()) {
                let coins: Vec<_> = bals
                    .iter()
                    .filter_map(|b| b.get("coin").and_then(|v| v.as_str()).map(|s| s.to_string()))
                    .collect();
                if !coins.is_empty() {
                    println!("Spot coins: {}", coins.join(", "));
                }
            }
            if let Some(w) = perp.get("withdrawable").and_then(|v| v.as_str()) {
                println!("Withdrawable: {w}");
            }
            Ok(())
        }
    }
}

fn resolve_user(user: Option<&str>, db: &hl_core::db::Db) -> Result<String> {
    if let Some(u) = user {
        if u.starts_with("0x") {
            return Ok(u.to_string());
        }
        let accts = db.list_accounts()?;
        if let Some(a) = accts.into_iter().find(|a| a.alias == u) {
            return Ok(a.address);
        }
        anyhow::bail!("unknown account '{u}'. Use 0x... address or an alias from: hl account ls");
    }

    if let Some(a) = db.default_account()? {
        return Ok(a.address);
    }
    anyhow::bail!("no default account configured. Run: hl account add (and optionally hl account set-default)");
}
