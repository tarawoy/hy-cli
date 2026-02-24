use anyhow::{Context, Result};
use reqwest::Url;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct InfoClient {
    base_url: Url,
    http: reqwest::Client,
}

impl InfoClient {
    pub fn new_mainnet() -> Result<Self> {
        Self::new("https://api.hyperliquid.xyz")
    }

    pub fn new_testnet() -> Result<Self> {
        // Hyperliquid testnet API base.
        Self::new("https://api.hyperliquid-testnet.xyz")
    }

    pub fn new(base: &str) -> Result<Self> {
        let base_url = Url::parse(base).context("invalid base url")?;
        let http = reqwest::Client::builder()
            .user_agent("hy-cli/0.1 (+https://github.com/tarawoy/hy-cli)")
            .build()
            .context("failed to build http client")?;
        Ok(Self { base_url, http })
    }

    pub async fn info<T: Serialize + ?Sized>(&self, body: &T) -> Result<Value> {
        let url = self.base_url.join("/info").context("join /info")?;
        let resp = self
            .http
            .post(url)
            .json(body)
            .send()
            .await
            .context("POST /info failed")?;

        let status = resp.status();
        let text = resp.text().await.context("read response body")?;
        if !status.is_success() {
            anyhow::bail!("/info returned {status}: {text}");
        }
        let v: Value = serde_json::from_str(&text).context("decode json")?;
        Ok(v)
    }

    pub async fn meta(&self) -> Result<Value> {
        self.info(&serde_json::json!({"type": "meta"})).await
    }

    pub async fn all_mids(&self) -> Result<Value> {
        self.info(&serde_json::json!({"type": "allMids"})).await
    }

    pub async fn l2_book(&self, coin: &str) -> Result<Value> {
        self.info(&serde_json::json!({"type": "l2Book", "coin": coin}))
            .await
    }

    pub async fn clearinghouse_state(&self, user: &str) -> Result<Value> {
        self.info(&serde_json::json!({"type": "clearinghouseState", "user": user}))
            .await
    }

    pub async fn spot_clearinghouse_state(&self, user: &str) -> Result<Value> {
        self.info(&serde_json::json!({"type": "spotClearinghouseState", "user": user}))
            .await
    }

    pub async fn portfolio(&self, user: &str) -> Result<Value> {
        self.info(&serde_json::json!({"type": "portfolio", "user": user}))
            .await
    }

    pub async fn open_orders(&self, user: &str) -> Result<Value> {
        self.info(&serde_json::json!({"type": "openOrders", "user": user}))
            .await
    }

    /// Compatibility helper: build a Project-A-like `userState` object by combining
    /// `clearinghouseState` + `spotClearinghouseState`.
    ///
    /// Note: Hyperliquid no longer accepts {type:"userState"} on /info.
    pub async fn user_state_compat(&self, user: &str) -> Result<Value> {
        let perp = self.clearinghouse_state(user).await?;
        let spot = self.spot_clearinghouse_state(user).await?;

        let spot_state = serde_json::json!({
            "balances": spot.get("balances").cloned().unwrap_or(Value::Null),
            "equity": spot.get("equity").cloned().unwrap_or(Value::Null),
        });

        Ok(serde_json::json!({
            "marginSummary": perp.get("marginSummary").cloned().unwrap_or(Value::Null),
            "assetPositions": perp.get("assetPositions").cloned().unwrap_or(Value::Null),
            "withdrawable": perp.get("withdrawable").cloned().unwrap_or(Value::Null),
            "spotState": spot_state,
        }))
    }
}
