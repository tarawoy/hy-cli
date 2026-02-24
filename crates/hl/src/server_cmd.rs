use anyhow::Result;

use crate::cli::ServerCmd;

pub async fn server(cmd: ServerCmd, json: bool, testnet: bool) -> Result<()> {
    match cmd {
        ServerCmd::Start => {
            hl_server::control::start(testnet).await?;
            if json {
                println!("{}", serde_json::json!({"ok": true}));
            } else {
                println!("hl-server started{}", if testnet {" (testnet)"} else {""});
            }
            Ok(())
        }
        ServerCmd::Stop => {
            hl_server::control::stop(testnet).await?;
            if json {
                println!("{}", serde_json::json!({"ok": true}));
            } else {
                println!("hl-server stopped{}", if testnet {" (testnet)"} else {""});
            }
            Ok(())
        }
        ServerCmd::Status => {
            let st = hl_server::control::status(testnet).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&st)?);
            } else {
                let pid = st.get("pid").and_then(|v| v.as_u64()).unwrap_or(0);
                let uptime = st.get("uptimeMs").and_then(|v| v.as_u64()).unwrap_or(0);
                let ws = st.get("wsUrl").and_then(|v| v.as_str()).unwrap_or("?");
                println!("hl-server pid={pid} uptimeMs={uptime} ws={ws}");
                if let Some(subs) = st.get("subs").and_then(|v| v.as_array()) {
                    for s in subs {
                        let key = s.get("key").and_then(|v| v.as_str()).unwrap_or("?");
                        let connected = s.get("connected").and_then(|v| v.as_bool()).unwrap_or(false);
                        println!("  {key}  connected={connected}");
                    }
                }
            }
            Ok(())
        }
        ServerCmd::Run => {
            // Foreground daemon runner.
            hl_server::daemon::run(testnet).await
        }
    }
}
