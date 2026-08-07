//! Postman Collection **v2.1.0** parser. Shares the whole v2 tree walk with v2.0
//! ([`super::shared::parse_v2_tree`]); the only v2.1-specific piece is auth, where params
//! are an **array** of `{key, value}` under a key named after the auth type.

use std::collections::BTreeMap;

use serde_json::Value;

use cq_model::{Auth, Workspace};
use cq_report::Report;

use super::shared;

/// v2.1 auth params: `{ "bearer": [ { "key": "token", "value": "…" } ] }` → `{token: …}`.
fn params(auth: &Value, ty: &str) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    if let Some(Value::Array(items)) = auth.get(ty) {
        for it in items {
            if let Some(k) = it.get("key").and_then(Value::as_str) {
                m.insert(k.to_string(), shared::coerce_value(it.get("value")));
            }
        }
    }
    m
}

pub(super) fn parse_auth(v: Option<&Value>, report: &mut Report, locator: &str) -> Option<Auth> {
    let auth = v?;
    let ty = auth.get("type").and_then(Value::as_str)?;
    shared::build_auth(ty, params(auth, ty), auth, report, locator)
}

pub(super) fn parse(root: &Value, report: &mut Report) -> Workspace {
    shared::parse_v2_tree(root, report, parse_auth)
}
