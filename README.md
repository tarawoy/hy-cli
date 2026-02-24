# Hyperliquid CLI (Rust) — `hl`

A command-line interface for Hyperliquid DEX.

This is a Rust rewrite inspired by:
https://github.com/chrisling-dev/hyperliquid-cli

## Installation

### Build from source (Linux)
```bash
git clone https://github.com/tarawoy/hy-cli.git
cd hy-cli
cargo build --release
sudo install -m 0755 target/release/hl /usr/local/bin/hl
hl --help
```

### OpenWrt / low-storage devices (important)
OpenWrt often has a tiny or read-only `/root`. SQLite state must go on a writable mount.

Example:
```sh
export HL_HOME=/mnt/mmcblk2p4/.hyperliquid
mkdir -p "$HL_HOME"
hl account ls
```

Make it persistent:
```sh
echo 'export HL_HOME=/mnt/mmcblk2p4/.hyperliquid' >> /etc/profile
. /etc/profile
```

## Features

- Multi-account management (SQLite)
- Real-time monitoring (WebSocket watch mode `-w`)
- Terminal UI (TUI) for watch mode (TTY) + JSON-lines fallback (non-TTY or `--json`)
- Background server cache: `hl server start|stop|status`
- Trading: limit/market/stop-loss/take-profit, cancel, cancel-all, leverage
- Mainnet + testnet

## Global Options

| Option | Description |
|---|---|
| `--json` | Output JSON (or JSON Lines for watch mode) |
| `--testnet` | Use testnet instead of mainnet |
| `-h, --help` | Help |
| `-V, --version` | Version |

## State / Storage

Defaults (Project A compatible):
- `~/.hyperliquid/accounts.db`
- `~/.hyperliquid/order-config.json`
- server socket: `~/.hyperliquid/hl-server.sock`

Override the state directory with:
- `HL_HOME=/path/to/.hyperliquid`

On OpenWrt you almost always want `HL_HOME` pointing at `/mnt/...`.

## Account Management

Accounts are stored locally in SQLite.

### Add Account
```bash
hl account add
```

### List Accounts
```bash
hl account ls
```

### Set Default Account
```bash
hl account set-default
```

### Remove Account
```bash
hl account remove
```

### Using `--user`
Many `hl account ...` commands accept:
- `--user 0x...` (address), OR
- `--user <alias>` (from `hl account ls`)

If `--user` is omitted, the default account is used.

## Balance & Portfolio Monitoring

### Get Balances
```bash
hl account balances
hl account balances --user 0x...
```

### Get Positions
```bash
hl account positions
hl account positions --user main
```

### Get Open Orders
```bash
hl account orders
hl account orders --user main
```

### Get Portfolio
```bash
hl account portfolio
hl account portfolio --user main
```

## Market Information

### List Markets
```bash
hl markets ls
```

### Get Prices
```bash
hl markets prices
```

## Asset Information

### Get Price
```bash
hl asset price BTC
```

### Get Order Book
```bash
hl asset book BTC
```

## Watch Mode (`-w`)

Watch mode provides live updates. If you run in a real terminal (TTY), you’ll get a TUI.
If you use `--json` or stdout isn’t a TTY, it streams JSON Lines (one event per line).

Exit keys: `q`, `Esc`, `Ctrl-C`

Examples:
```bash
hl account balances -w
hl account positions -w
hl account orders -w
hl account portfolio -w

hl asset price BTC -w
hl asset book BTC -w
```

## Background Server

Optional cache server for faster queries + server-backed watch mode.

### Start
```bash
hl server start
hl server status
```

### Stop
```bash
hl server stop
```

Notes:
- When the server is running, `-w` commands will auto-use the server cache when possible.
- If the server is not running, `-w` falls back to direct WebSocket streaming.

## Trading

### Required (for trade commands)
Set a private key:

```bash
export HYPERLIQUID_PRIVATE_KEY=0x...
```

> Do NOT put your private key in crontab or shell history. Prefer an environment file sourced by your shell/service.

### Limit Orders
```bash
hl trade order limit buy 0.001 BTC 50000
hl trade order limit sell 0.1 ETH 3500 --tif Gtc
hl trade order limit long 1 SOL 100 --reduce-only --tif Alo
```

TIF values:
- `Gtc` (default), `Ioc`, `Alo` (post-only)

### Market Orders
Market orders are implemented as aggressive IOC using mid price + slippage.

```bash
hl trade order market buy 0.001 BTC --slippage 1
hl trade order market sell 0.1 ETH --slippage 0.5 --reduce-only
```

### Stop-Loss / Take-Profit (Trigger Orders)
```bash
hl trade order stop-loss sell 0.001 BTC 48000 49000
hl trade order take-profit sell 0.001 BTC 55000 54000
```

### Cancel Orders
```bash
hl trade cancel <oid>
hl trade cancel-all
hl trade cancel-all --coin BTC -y
```

### Set Leverage
```bash
hl trade set-leverage BTC 10
hl trade set-leverage BTC 10 --isolated
hl trade set-leverage BTC 10 --cross
```

## Scripting with JSON

```bash
hl markets prices --json
hl account positions --json
hl asset book BTC --json
```

Watch mode JSON lines:
```bash
hl account positions -w --json
```

## Troubleshooting

### SQLite error code 14 (“unable to open database file”)
Your state directory is not writable (common on OpenWrt).

Fix:
```sh
export HL_HOME=/mnt/mmcblk2p4/.hyperliquid
mkdir -p "$HL_HOME"
```

### UNIQUE constraint failed (accounts.address)
That address already exists in the DB.
Use:
```bash
hl account ls
hl account set-default
```
or remove then re-add:
```bash
hl account remove
hl account add
```

## License
MIT
