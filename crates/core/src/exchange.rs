use anyhow::{Context, Result};
use ethers_core::types::transaction::eip712::{EIP712Domain, TypedData, Types};
use ethers_core::types::{Address, Signature, U256};
use reqwest::Url;
use serde_json::Value;
use tiny_keccak::{Hasher, Keccak};

use crate::msgpack::{encode_msgpack, json_to_msgpack_canonical};
use crate::signer::HlSigner;

#[derive(Debug, Clone)]
pub struct ExchangeClient {
    base_url: Url,
    http: reqwest::Client,
    signer: HlSigner,
    /// If set, actions are signed as vault actions (subaccount).
    pub vault_address: Option<Address>,
    /// If set, included in /exchange payload and action hash.
    pub expires_after: Option<u64>,
    /// Mainnet vs testnet affects the EIP712 `Agent.source` field.
    pub is_mainnet: bool,
}

impl ExchangeClient {
    pub fn new_mainnet(signer: HlSigner) -> Result<Self> {
        Self::new("https://api.hyperliquid.xyz", true, signer)
    }

    pub fn new_testnet(signer: HlSigner) -> Result<Self> {
        Self::new("https://api.hyperliquid-testnet.xyz", false, signer)
    }

    pub fn new(base: &str, is_mainnet: bool, signer: HlSigner) -> Result<Self> {
        let base_url = Url::parse(base).context("invalid base url")?;
        let http = reqwest::Client::builder()
            .user_agent("hy-cli/0.1 (+https://github.com/tarawoy/hy-cli)")
            .build()
            .context("failed to build http client")?;
        Ok(Self {
            base_url,
            http,
            signer,
            vault_address: None,
            expires_after: None,
            is_mainnet,
        })
    }

    pub fn signer_address(&self) -> Address {
        self.signer.address()
    }

    pub async fn post_action(&self, action: &Value, nonce_ms: u64) -> Result<Value> {
        let sig = self
            .sign_l1_action(action, self.vault_address, nonce_ms, self.expires_after)
            .await?;

        let payload = serde_json::json!({
            "action": action,
            "nonce": nonce_ms,
            "signature": sig,
            "vaultAddress": self.vault_address.map(|a| format!("{a:#x}")),
            "expiresAfter": self.expires_after,
        });

        let url = self.base_url.join("/exchange").context("join /exchange")?;
        let resp = self
            .http
            .post(url)
            .json(&payload)
            .send()
            .await
            .context("POST /exchange failed")?;

        let status = resp.status();
        let text = resp.text().await.context("read response body")?;
        if !status.is_success() {
            anyhow::bail!("/exchange returned {status}: {text}");
        }
        let v: Value = serde_json::from_str(&text).context("decode json")?;
        Ok(v)
    }

    async fn sign_l1_action(
        &self,
        action: &Value,
        vault_address: Option<Address>,
        nonce: u64,
        expires_after: Option<u64>,
    ) -> Result<Value> {
        let hash = action_hash(action, vault_address, nonce, expires_after)?;
        let phantom_agent = serde_json::json!({
            "source": if self.is_mainnet {"a"} else {"b"},
            "connectionId": format!("0x{}", hex::encode(hash)),
        });

        let td = l1_typed_data(phantom_agent)?;
        let sig: Signature = self.signer.sign_typed_data(&td).await?;

        fn u256_32be_hex(x: ethers_core::types::U256) -> String {
            let mut buf = [0u8; 32];
            x.to_big_endian(&mut buf);
            format!("0x{}", hex::encode(buf))
        }

        Ok(serde_json::json!({
            "r": u256_32be_hex(sig.r),
            "s": u256_32be_hex(sig.s),
            "v": sig.v,
        }))
    }
}

fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut k = Keccak::v256();
    k.update(data);
    k.finalize(&mut out);
    out
}

fn address_to_bytes(a: Address) -> [u8; 20] {
    a.to_fixed_bytes()
}

/// Port of hlpy.hyperliquid.utils.signing.action_hash.
fn action_hash(action: &Value, vault_address: Option<Address>, nonce: u64, expires_after: Option<u64>) -> Result<[u8; 32]> {
    // msgpack(action)
    let mval = json_to_msgpack_canonical(action)?;
    let mut data = encode_msgpack(&mval)?;

    // nonce uint64 big-endian
    data.extend_from_slice(&nonce.to_be_bytes());

    // vault flag + bytes
    match vault_address {
        None => data.push(0u8),
        Some(addr) => {
            data.push(1u8);
            data.extend_from_slice(&address_to_bytes(addr));
        }
    }

    // expiresAfter
    if let Some(ea) = expires_after {
        data.push(0u8);
        data.extend_from_slice(&ea.to_be_bytes());
    }

    Ok(keccak256(&data))
}

fn l1_typed_data(message: Value) -> Result<TypedData> {
    // Matches hlpy: domain(chainId=1337,name=Exchange,verifyingContract=0x0,version=1)
    let domain = EIP712Domain {
        name: Some("Exchange".to_string()),
        version: Some("1".to_string()),
        chain_id: Some(U256::from(1337u64)),
        verifying_contract: Some(Address::zero()),
        salt: None,
    };

    let mut types: Types = Types::new();
    // Ethers typically auto-adds this, but we include it explicitly for parity with hypersdk/hlpy.
    types.insert(
        "EIP712Domain".to_string(),
        vec![
            ethers_core::types::transaction::eip712::Field {
                name: "name".to_string(),
                r#type: "string".to_string(),
            },
            ethers_core::types::transaction::eip712::Field {
                name: "version".to_string(),
                r#type: "string".to_string(),
            },
            ethers_core::types::transaction::eip712::Field {
                name: "chainId".to_string(),
                r#type: "uint256".to_string(),
            },
            ethers_core::types::transaction::eip712::Field {
                name: "verifyingContract".to_string(),
                r#type: "address".to_string(),
            },
        ],
    );

    types.insert(
        "Agent".to_string(),
        vec![
            ethers_core::types::transaction::eip712::Field {
                name: "source".to_string(),
                r#type: "string".to_string(),
            },
            ethers_core::types::transaction::eip712::Field {
                name: "connectionId".to_string(),
                r#type: "bytes32".to_string(),
            },
        ],
    );

    Ok(TypedData {
        types,
        primary_type: "Agent".to_string(),
        domain,
        message,
    })
}

/// Utility: current unix timestamp in ms.
pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    dur.as_millis() as u64
}

/// Convert a decimal string/number to Hyperliquid wire format: up to 8 decimals,
/// trimmed trailing zeros.
pub fn float_to_wire(x: f64) -> Result<String> {
    let rounded = format!("{x:.8}");
    let back: f64 = rounded.parse().context("parse rounded")?;
    if (back - x).abs() >= 1e-12 {
        anyhow::bail!("float_to_wire causes rounding: {x} -> {rounded}");
    }

    let mut s = rounded;
    if s.starts_with("-0") {
        // -0.00000000 etc.
        let f: f64 = s.parse().unwrap_or(0.0);
        if f == 0.0 {
            s = "0".to_string();
        }
    }
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".to_string();
    }
    Ok(s)
}

/// Round to N decimals.
pub fn round_to_decimals(x: f64, decimals: i32) -> f64 {
    if decimals <= 0 {
        return x.round();
    }
    let p = 10_f64.powi(decimals);
    (x * p).round() / p
}

/// Round to 5 significant figures (like Python's f"{px:.5g}").
pub fn round_5_sigfig(x: f64) -> f64 {
    if x == 0.0 {
        return 0.0;
    }
    let absx = x.abs();
    let log10 = absx.log10().floor();
    let scale = 10_f64.powf(4.0 - log10);
    (x * scale).round() / scale
}
