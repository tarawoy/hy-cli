# hy-cli (`hl`) — Hyperliquid CLI (Rust)

Rust rewrite inspired by: https://github.com/chrisling-dev/hyperliquid-cli

## Highlights

- **Account DB (SQLite):** `hl account add/ls/set-default/remove`
- **Market data:** `hl markets ...`, `hl asset ...`
- **Account monitoring:** balances / positions / orders / portfolio
- **Watch mode:** `-w` (TUI on TTY; JSON-lines when `--json` or non‑TTY)
- **Trading:** limit/market/SL/TP + cancel/cancel-all + leverage
- **Background server:** `hl server start/stop/status` (cache + watch auto-uses server when running)
- **OpenWrt-friendly:** supports `HL_HOME` so state can live on `/mnt/...`

---

## Install (Linux)

```bash
git clone https://github.com/tarawoy/hy-cli.git
cd hy-cli
cargo build --release
sudo install -m 0755 target/release/hl /usr/local/bin/hl
hl --help
```

## Install / Run (OpenWrt)

OpenWrt root is often tiny/read-only, so point state to a writable mount:

```sh
export HL_HOME=/mnt/mmcblk2p4/.hyperliquid
mkdir -p "$HL_HOME"
```

Then run normally:
```sh
hl account ls
```

Persist for interactive shells:
```sh
echo 'export HL_HOME=/mnt/mmcblk2p4/.hyperliquid' >> /etc/profile
. /etc/profile
```

---

## Global options

- `--json`  → JSON output (watch mode becomes JSON Lines)
- `--testnet` → use testnet

---

## Account setup

```bash
hl account add
hl account ls
hl account set-default
```

Many commands accept:
- `--user 0x...` (address) OR
- `--user <alias>` (from `hl account ls`)

If omitted, default account is used.

---

## Markets / Asset

```bash
hl markets ls
hl markets prices

hl asset price BTC
hl asset book BTC
```

---

## Account monitoring (one-shot)

```bash
hl account balances
hl account positions
hl account orders
hl account portfolio
```

### Watch mode (`-w`)

```bash
hl account balances -w
hl account positions -w
hl account orders -w
hl account portfolio -w

hl asset price BTC -w
hl asset book BTC -w
```

Exit: `q` / `Esc` / `Ctrl-C`

---

## Background server

```bash
hl server start
hl server status
hl server stop
```

Notes:
- When server is running, `-w` commands will auto-use cached data via IPC.

---

## Trading (real execution)

### Required env

```bash
export HYPERLIQUID_PRIVATE_KEY=0x...
```

### Limit / Market

```bash
hl trade order limit buy 0.001 BTC 50000 --tif Gtc
hl trade order market buy 0.001 BTC --slippage 1
```

### Trigger TP / SL (new helper commands)

These create trigger orders. **If `--size` is omitted, it uses 100% of your current open position size** for that coin.

**Stop-loss:**
```bash
hl trade sltrigger BTC --trigger 63703
hl trade sltrigger BTC --trigger -1.1% --ref entry
```

**Take-profit:**
```bash
hl trade tptrigger BTC --trigger 64881
hl trade tptrigger BTC --trigger +1.7% --ref entry
```

Options:
- `--ref entry|mark` (used only for percent triggers)
- `--size <baseSize>` (override default full-position size)
- `--limit <price>` (defaults to trigger)
- `--reduce-only` (default true)

### Cancel / Leverage

```bash
hl trade cancel <oid>
hl trade cancel-all
hl trade cancel-all --coin BTC -y

hl trade set-leverage BTC 10
hl trade set-leverage BTC 10 --isolated
```

---

## Troubleshooting

### SQLite error code 14 (unable to open database file)
State path not writable (common on OpenWrt). Fix:
```sh
export HL_HOME=/mnt/mmcblk2p4/.hyperliquid
mkdir -p "$HL_HOME"
```

### UNIQUE constraint failed: accounts.address
That address already exists in DB. Use:
```bash
hl account ls
hl account set-default
```

---

## License

MIT
