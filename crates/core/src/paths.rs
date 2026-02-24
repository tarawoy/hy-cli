use anyhow::Result;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
    pub accounts_db: PathBuf,
    pub order_config: PathBuf,
}

impl Paths {
    /// Match Project A paths exactly by default.
    ///
    /// Override for devices (e.g. OpenWrt) by setting HL_HOME to a writable mount.
    pub fn resolve() -> Result<Self> {
        let home = if let Ok(v) = std::env::var(crate::env::ENV_HL_HOME) {
            PathBuf::from(v)
        } else {
            let base = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot resolve home dir"))?;
            base.join(".hyperliquid")
        };

        Ok(Self {
            accounts_db: home.join("accounts.db"),
            order_config: home.join("order-config.json"),
            home,
        })
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(&self.home)?;
        Ok(())
    }
}
