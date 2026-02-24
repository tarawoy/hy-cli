use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use crate::client::{pid_path, socket_path, tcp_addr_path, Client, Endpoint};

pub async fn start(testnet: bool) -> Result<()> {
    let paths = hl_core::paths::Paths::resolve()?;
    paths.ensure_dirs()?;

    // If already running, just return ok.
    if let Some(c) = Client::try_connect(testnet).await {
        let _ = c.status().await?;
        return Ok(());
    }

    // If stale pidfile exists, try to clean it.
    let pidfile = pid_path(&paths.home, testnet);
    if let Ok(pid) = read_pid(&pidfile) {
        if is_pid_alive(pid) {
            anyhow::bail!("hl-server appears to be running already (pid={pid}) but IPC not reachable");
        } else {
            let _ = std::fs::remove_file(&pidfile);
        }
    }

    // Spawn `hl server run` detached.
    let exe = std::env::current_exe().context("current_exe")?;
    let mut cmd = std::process::Command::new(exe);

    if testnet {
        cmd.arg("--testnet");
    }
    cmd.args(["server", "run"]);
    cmd.env("HL_SERVER_DAEMON", "1");
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());

    // On Unix we can also create a new session in the child (done in daemon::run).
    let child = cmd.spawn().context("spawn hl server run")?;

    // Write pid immediately.
    std::fs::write(&pidfile, format!("{}\n", child.id()))?;

    // Wait briefly for socket.
    let sock = socket_path(&paths.home, testnet);
    let tcp = tcp_addr_path(&paths.home, testnet);
    for _ in 0..40 {
        if sock.exists() || tcp.exists() {
            if let Some(c) = Client::try_connect(testnet).await {
                let _ = c.status().await?;
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    anyhow::bail!("server did not become ready (socket not reachable)")
}

pub async fn stop(testnet: bool) -> Result<()> {
    let paths = hl_core::paths::Paths::resolve()?;
    let pidfile = pid_path(&paths.home, testnet);
    let pid = read_pid(&pidfile).context("read pidfile")?;

    if !is_pid_alive(pid) {
        cleanup_ipc_files(testnet)?;
        let _ = std::fs::remove_file(&pidfile);
        return Ok(());
    }

    signal_term(pid).context("signal TERM")?;

    for _ in 0..50 {
        if !is_pid_alive(pid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    cleanup_ipc_files(testnet)?;
    let _ = std::fs::remove_file(&pidfile);

    if is_pid_alive(pid) {
        anyhow::bail!("server did not stop (pid={pid})")
    }

    Ok(())
}

pub async fn status(testnet: bool) -> Result<serde_json::Value> {
    let c = Client::try_connect(testnet)
        .await
        .context("hl-server not running")?;
    c.status().await
}

pub fn cleanup_ipc_files(testnet: bool) -> Result<()> {
    let paths = hl_core::paths::Paths::resolve()?;
    let sock = socket_path(&paths.home, testnet);
    let tcp = tcp_addr_path(&paths.home, testnet);
    let _ = std::fs::remove_file(sock);
    let _ = std::fs::remove_file(tcp);
    Ok(())
}

fn read_pid(p: &PathBuf) -> Result<i32> {
    let s = std::fs::read_to_string(p)?;
    let pid: i32 = s.trim().parse().context("parse pid")?;
    Ok(pid)
}

fn is_pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // kill(pid, 0) checks existence.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn signal_term(pid: i32) -> Result<()> {
    let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
    if rc != 0 {
        anyhow::bail!("kill(SIGTERM) failed")
    }
    Ok(())
}

pub async fn endpoint_for_status(testnet: bool) -> Result<Endpoint> {
    let c = Client::try_connect(testnet)
        .await
        .context("hl-server not running")?;
    Ok(c.endpoint().clone())
}
