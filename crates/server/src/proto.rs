use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "camelCase")]
pub enum Request {
    Status,
    Get {
        kind: GetKind,
        #[serde(default)]
        coin: Option<String>,
        #[serde(default)]
        user: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GetKind {
    AllMids,
    L2Book,
    UserState,
    OpenOrders,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

impl Response {
    pub fn ok(data: Option<Value>, meta: Option<Value>) -> Self {
        Self {
            ok: true,
            error: None,
            data,
            meta,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(msg.into()),
            data: None,
            meta: None,
        }
    }
}
