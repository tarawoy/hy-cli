use anyhow::{Context, Result};
use ethers_signers::{LocalWallet, Signer};
use std::path::Path;

use crate::env::{
    ENV_HYPERLIQUID_KEYSTORE, ENV_HYPERLIQUID_KEYSTORE_PASSWORD, ENV_HYPERLIQUID_PRIVATE_KEY,
};

#[derive(Clone)]
pub struct HlSigner {
    wallet: LocalWallet,
}

impl std::fmt::Debug for HlSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HlSigner")
            .field("address", &self.wallet.address())
            .finish()
    }
}

impl HlSigner {
    pub fn address(&self) -> ethers_core::types::Address {
        self.wallet.address()
    }

    pub fn load_from_env() -> Result<Self> {
        if let Ok(pk) = std::env::var(ENV_HYPERLIQUID_PRIVATE_KEY) {
            let pk = pk.trim();
            let pk = pk.strip_prefix("0x").unwrap_or(pk);
            let bytes = hex::decode(pk).context("decode HYPERLIQUID_PRIVATE_KEY hex")?;
            let wallet: LocalWallet = LocalWallet::from_bytes(&bytes)
                .context("parse HYPERLIQUID_PRIVATE_KEY as 32-byte secp256k1 key")?;
            return Ok(Self { wallet });
        }

        if let Ok(ks_path) = std::env::var(ENV_HYPERLIQUID_KEYSTORE) {
            let ks_path = ks_path.trim();
            if ks_path.is_empty() {
                anyhow::bail!("{ENV_HYPERLIQUID_KEYSTORE} is set but empty");
            }
            return Self::load_from_keystore(Path::new(ks_path));
        }

        anyhow::bail!(
            "no signer configured. Set {ENV_HYPERLIQUID_PRIVATE_KEY} (recommended) or {ENV_HYPERLIQUID_KEYSTORE}"
        );
    }

    pub fn load_from_keystore(path: &Path) -> Result<Self> {
        let password = if let Ok(pw) = std::env::var(ENV_HYPERLIQUID_KEYSTORE_PASSWORD) {
            pw
        } else {
            // Interactive prompt. This is best-effort; if stdin isn't a tty, rpassword will error.
            rpassword::prompt_password("Keystore password: ")
                .context("prompt keystore password (set HYPERLIQUID_KEYSTORE_PASSWORD for non-interactive)")?
        };

        let wallet = LocalWallet::decrypt_keystore(path, &password)
            .with_context(|| format!("decrypt keystore {}", path.display()))?;
        Ok(Self { wallet })
    }

    pub async fn sign_typed_data(&self, td: &ethers_core::types::transaction::eip712::TypedData) -> Result<ethers_core::types::Signature> {
        let sig = self
            .wallet
            .sign_typed_data(td)
            .await
            .context("sign typed data")?;
        Ok(sig)
    }
}
