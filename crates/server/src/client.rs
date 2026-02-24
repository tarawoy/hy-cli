use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpStream, UnixStream};

use crate::proto::{Request, Response};

#[derive(Debug, Clone)]
pub enum Endpoint {
    Unix(PathBuf),
    Tcp(String),
}

#[derive(Debug, Clone)]
pub struct Client {
    ep: Endpoint,
}

impl Client {
    pub fn endpoint(&self) -> &Endpoint {
        &self.ep
    }

    pub async fn connect(ep: Endpoint) -> Result<Self> {
        // test connection via status
        let c = Self { ep };
        let _ = c.status().await?;
        Ok(c)
    }

    pub async fn try_connect(testnet: bool) -> Option<Self> {
        let ep = resolve_endpoint(testnet).ok()?;
        Self::connect(ep).await.ok()
    }

    pub async fn status(&self) -> Result<Value> {
        let resp = self.request(Request::Status).await?;
        if !resp.ok {
            anyhow::bail!(resp.error.unwrap_or_else(|| "server error".into()));
        }
        Ok(resp.meta.unwrap_or_else(|| serde_json::json!({})))
    }

    pub async fn get_all_mids(&self) -> Result<Value> {
        self.get(crate::proto::GetKind::AllMids, None, None).await
    }

    pub async fn get_l2_book(&self, coin: &str) -> Result<Value> {
        self.get(crate::proto::GetKind::L2Book, Some(coin.into()), None)
            .await
    }

    pub async fn get_user_state(&self, user: &str) -> Result<Value> {
        self.get(crate::proto::GetKind::UserState, None, Some(user.into()))
            .await
    }

    pub async fn get_open_orders(&self, user: &str) -> Result<Value> {
        self.get(crate::proto::GetKind::OpenOrders, None, Some(user.into()))
            .await
    }

    async fn get(
        &self,
        kind: crate::proto::GetKind,
        coin: Option<String>,
        user: Option<String>,
    ) -> Result<Value> {
        let resp = self
            .request(Request::Get { kind, coin, user })
            .await?;
        if !resp.ok {
            anyhow::bail!(resp.error.unwrap_or_else(|| "server error".into()));
        }
        resp.data
            .context("server response missing data")
            .map_err(Into::into)
    }

    pub async fn request(&self, req: Request) -> Result<Response> {
        let line = serde_json::to_string(&req)? + "\n";
        match &self.ep {
            Endpoint::Unix(p) => {
                let st = UnixStream::connect(p).await.context("connect unix")?;
                request_on_stream(st, line).await
            }
            Endpoint::Tcp(addr) => {
                let st = TcpStream::connect(addr).await.context("connect tcp")?;
                request_on_stream(st, line).await
            }
        }
    }
}

async fn request_on_stream<S>(mut st: S, line: String) -> Result<Response>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    st.write_all(line.as_bytes()).await?;
    st.flush().await?;
    let mut rd = BufReader::new(st);
    let mut resp_line = String::new();
    rd.read_line(&mut resp_line).await?;
    let resp: Response = serde_json::from_str(resp_line.trim())?;
    Ok(resp)
}

fn resolve_endpoint(testnet: bool) -> Result<Endpoint> {
    let paths = hl_core::paths::Paths::resolve()?;
    let sock = socket_path(&paths.home, testnet);
    if sock.exists() {
        return Ok(Endpoint::Unix(sock));
    }

    let tcp_path = tcp_addr_path(&paths.home, testnet);
    if tcp_path.exists() {
        let addr = std::fs::read_to_string(tcp_path)?.trim().to_string();
        if !addr.is_empty() {
            return Ok(Endpoint::Tcp(addr));
        }
    }

    anyhow::bail!("no server socket found")
}

pub fn socket_path(home: &Path, testnet: bool) -> PathBuf {
    if testnet {
        home.join("hl-server-testnet.sock")
    } else {
        home.join("hl-server.sock")
    }
}

pub fn pid_path(home: &Path, testnet: bool) -> PathBuf {
    if testnet {
        home.join("hl-server-testnet.pid")
    } else {
        home.join("hl-server.pid")
    }
}

pub fn tcp_addr_path(home: &Path, testnet: bool) -> PathBuf {
    if testnet {
        home.join("hl-server-testnet.tcp")
    } else {
        home.join("hl-server.tcp")
    }
}
