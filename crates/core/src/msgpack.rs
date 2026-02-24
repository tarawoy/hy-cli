use anyhow::{Context, Result};
use rmpv::Value as MVal;
use serde_json::Value as JVal;

/// Convert JSON value to msgpack value with deterministic map key ordering.
///
/// Hyperliquid's action hash is computed over msgpack(action) bytes. Different map key
/// orderings would change the bytes, so we canonicalize by sorting keys lexicographically
/// at every object level.
pub fn json_to_msgpack_canonical(v: &JVal) -> Result<MVal> {
    Ok(match v {
        JVal::Null => MVal::Nil,
        JVal::Bool(b) => MVal::Boolean(*b),
        JVal::Number(n) => {
            if let Some(i) = n.as_i64() {
                MVal::from(i)
            } else if let Some(u) = n.as_u64() {
                MVal::from(u)
            } else if let Some(f) = n.as_f64() {
                // Avoid floats in actions when possible; HL usually uses strings.
                // If it happens, encode as f64.
                MVal::F64(f)
            } else {
                anyhow::bail!("unsupported json number: {n}");
            }
        }
        JVal::String(s) => MVal::String(s.clone().into()),
        JVal::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for x in arr {
                out.push(json_to_msgpack_canonical(x)?);
            }
            MVal::Array(out)
        }
        JVal::Object(map) => {
            let mut keys: Vec<_> = map.keys().cloned().collect();
            keys.sort();
            let mut pairs = Vec::with_capacity(keys.len());
            for k in keys {
                let vv = map.get(&k).context("map key disappeared")?;
                pairs.push((MVal::String(k.into()), json_to_msgpack_canonical(vv)?));
            }
            MVal::Map(pairs)
        }
    })
}

pub fn encode_msgpack(v: &MVal) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    rmpv::encode::write_value(&mut buf, v).context("encode msgpack")?;
    Ok(buf)
}
