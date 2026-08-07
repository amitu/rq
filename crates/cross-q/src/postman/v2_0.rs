//! Postman Collection **v2.0.0** parser. Identical tree shape to v2.1 — so it reuses the
//! whole v2 tree walk ([`super::shared::parse_v2_tree`]) — differing in exactly one place:
//! auth params are an **object** (`{ "bearer": { "token": "…" } }`) rather than v2.1's
//! array of `{key, value}`. That one difference is why version isolation beats an
//! `if version === '2.0'` branch inside a single parser.

use std::collections::BTreeMap;

use serde_json::Value;

use cq_model::{Auth, Workspace};
use cq_report::Report;

use super::shared;

/// v2.0 auth params: `{ "bearer": { "token": "…" } }` → `{token: …}`.
fn params(auth: &Value, ty: &str) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    if let Some(Value::Object(obj)) = auth.get(ty) {
        for (k, v) in obj {
            m.insert(k.clone(), shared::coerce_value(Some(v)));
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
