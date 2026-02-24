# hy-cli (hl)

Rust rewrite of https://github.com/chrisling-dev/hyperliquid-cli.

## Goal
Match Project A feature-for-feature:
- Same command tree
- Same defaults and local paths (`~/.hyperliquid/...`)
- Same watch mode UX (`-w`) and terminal UI as closely as practical
- Same JSON mode (`--json`)
- Same background server (`hl server start|stop|status`)

## Status
Milestone 1: non-watch (`--json` + one-shot HTTP) implemented for core read-only commands.

Milestone 2: watch mode (`-w`) implemented for:
- `hl account balances -w` (WS `webData2`)
- `hl account positions -w` (WS `webData2`)
- `hl account orders -w` (WS `orderUpdates`)
- `hl account portfolio -w` (WS `webData2`)
- `hl asset price COIN -w` (WS `allMids`)
- `hl asset book COIN -w` (WS `l2Book`)

Watch UX:
- If stdout is a TTY and `--json` is **not** set, a minimal Ratatui/Crossterm TUI is used.
- If stdout is **not** a TTY (or `--json` is set), watch mode streams **JSON Lines** (one event per line).
- Clean exit: `q` / `Esc` in TUI, or Ctrl-C.
- Reconnect: automatic with exponential backoff; subscriptions are re-sent after reconnect.

Milestone 4: background server parity (`hl server start|stop|status`) implemented.

What it does:
- Runs a local daemon that maintains websocket subscriptions and caches the latest snapshots:
  - `allMids`
  - `l2Book` per coin (on-demand)
  - `userState` per user via WS `webData2` (on-demand)
  - `openOrders` per user via WS `orderUpdates` (on-demand)
- Exposes a local IPC endpoint so `hl` can query the cache instead of hitting the network.
  - Default: Unix socket under `~/.hyperliquid/hl-server.sock` (or `$HL_HOME/...`).
  - Fallback: localhost TCP (auto) if Unix socket bind fails (useful for OpenWrt).

Usage:
```bash
hl server start
hl server status
hl server stop
```

Watch mode auto-detect:
- When running in TTY TUI mode, `-w` commands will use the server cache if the server is running.
- Otherwise they fall back to direct websocket streaming.

## Build
```bash
cargo build --release
```

## OpenWrt build
GitHub Actions workflow builds `hl-openwrt.tar.gz` (aarch64 musl) on push/release.
