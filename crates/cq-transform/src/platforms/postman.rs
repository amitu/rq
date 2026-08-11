use crate::replacer::span_to_info;
use crate::types::{Diagnostic, DiagnosticKind, Replacement};

/// Phase 2: Legacy API transforms (postman.setEnvironmentVariable → rq.environment.set, etc.)
/// Returns (replacements, diagnostics) for a call expression.
pub fn check_legacy_call(
    source: &str,
    obj_name: &str,
    method_chain: &[&str],
    call_start: u32,
    call_end: u32,
    obj_start: u32,
    _obj_end: u32,
) -> Option<(Vec<Replacement>, Vec<Diagnostic>)> {
    if obj_name != "postman" {
        return None;
    }

    let mut replacements = Vec::new();
    let mut diagnostics = Vec::new();

    match method_chain {
        ["setEnvironmentVariable"] => {
            // postman.setEnvironmentVariable(k, v) → rq.environment.set(k, v)
            let full_text = &source[call_start as usize..call_end as usize];
            let paren_offset = full_text.find('(');
            if let Some(paren) = paren_offset {
                let method_end = call_start + paren as u32;
                replacements.push(Replacement {
                    start: obj_start,
                    end: method_end,
                    new_text: "rq.environment.set".to_string(),
                    message: "postman.setEnvironmentVariable → rq.environment.set".to_string(),
                });
            }
        }
        ["getEnvironmentVariable"] => {
            let full_text = &source[call_start as usize..call_end as usize];
            let paren_offset = full_text.find('(');
            if let Some(paren) = paren_offset {
                let method_end = call_start + paren as u32;
                replacements.push(Replacement {
                    start: obj_start,
                    end: method_end,
                    new_text: "rq.environment.get".to_string(),
                    message: "postman.getEnvironmentVariable → rq.environment.get".to_string(),
                });
            }
        }
        ["clearEnvironmentVariable"] => {
            let full_text = &source[call_start as usize..call_end as usize];
            let paren_offset = full_text.find('(');
            if let Some(paren) = paren_offset {
                let method_end = call_start + paren as u32;
                replacements.push(Replacement {
                    start: obj_start,
                    end: method_end,
                    new_text: "rq.environment.unset".to_string(),
                    message: "postman.clearEnvironmentVariable → rq.environment.unset".to_string(),
                });
            }
        }
        ["setGlobalVariable"] => {
            let full_text = &source[call_start as usize..call_end as usize];
            let paren_offset = full_text.find('(');
            if let Some(paren) = paren_offset {
                let method_end = call_start + paren as u32;
                replacements.push(Replacement {
                    start: obj_start,
                    end: method_end,
                    new_text: "rq.globals.set".to_string(),
                    message: "postman.setGlobalVariable → rq.globals.set".to_string(),
                });
            }
        }
        ["getGlobalVariable"] => {
            let full_text = &source[call_start as usize..call_end as usize];
            let paren_offset = full_text.find('(');
            if let Some(paren) = paren_offset {
                let method_end = call_start + paren as u32;
                replacements.push(Replacement {
                    start: obj_start,
                    end: method_end,
                    new_text: "rq.globals.get".to_string(),
                    message: "postman.getGlobalVariable → rq.globals.get".to_string(),
                });
            }
        }
        ["clearGlobalVariable"] => {
            let full_text = &source[call_start as usize..call_end as usize];
            let paren_offset = full_text.find('(');
            if let Some(paren) = paren_offset {
                let method_end = call_start + paren as u32;
                replacements.push(Replacement {
                    start: obj_start,
                    end: method_end,
                    new_text: "rq.globals.unset".to_string(),
                    message: "postman.clearGlobalVariable → rq.globals.unset".to_string(),
                });
            }
        }
        ["setNextRequest"] => {
            let full_text = &source[call_start as usize..call_end as usize];
            let paren_offset = full_text.find('(');
            if let Some(paren) = paren_offset {
                let method_end = call_start + paren as u32;
                replacements.push(Replacement {
                    start: obj_start,
                    end: method_end,
                    new_text: "rq.execution.setNextRequest".to_string(),
                    message: "postman.setNextRequest → rq.execution.setNextRequest".to_string(),
                });
            }
        }
        ["getResponseHeader"] => {
            // postman.getResponseHeader(h) → rq.response.headers.get(h)
            let full_text = &source[call_start as usize..call_end as usize];
            let paren_offset = full_text.find('(');
            if let Some(paren) = paren_offset {
                let method_end = call_start + paren as u32;
                replacements.push(Replacement {
                    start: obj_start,
                    end: method_end,
                    new_text: "rq.response.headers.get".to_string(),
                    message: "postman.getResponseHeader → rq.response.headers.get".to_string(),
                });
            }
        }
        _ => return None,
    }

    if replacements.is_empty() && diagnostics.is_empty() {
        return None;
    }

    // Add diagnostic for each replacement
    for r in &replacements {
        diagnostics.push(Diagnostic {
            kind: DiagnosticKind::Replacement,
            message: r.message.clone(),
            span: Some(span_to_info(source, r.start, r.end)),
        });
    }

    Some((replacements, diagnostics))
}

/// Phase 3: Diagnostics for unsupported Postman APIs.
pub fn check_unsupported(
    source: &str,
    root_name: &str,
    method_chain: &[&str],
    expr_start: u32,
    expr_end: u32,
) -> Option<Diagnostic> {
    if root_name != "pm" {
        return None;
    }

    // NOTE: `pm.vault.*`, `pm.cookies.jar()`, `pm.{scope}.toObject()`, and
    // `pm.visualizer.*` were previously flagged "not supported" here, but all work
    // at runtime now (vault reads via ADR-022; cookies.jar mirrors Postman 1:1 via
    // ADR-105; toObject exists on every variable scope; rq.visualizer.set/clear are
    // implemented in both sandbox engines per ADR-202 / RQ-4994, and the pm.* → rq.*
    // rename makes an imported pm.visualizer.set() run unchanged — RQ-4998). Those
    // stale warnings were a loud FALSE alarm on import (RQ-4058 P0-3 / RQ-4998) and
    // have been removed. Only `variables.replaceIn` (partial — `$` dynamic-var
    // divergence per ADR-055) is genuine and remains.
    let warning = match method_chain {
        ["variables", "replaceIn"] => {
            Some("pm.variables.replaceIn() has partial support in Requestly")
        }
        _ => None,
    };

    warning.map(|msg| Diagnostic {
        kind: DiagnosticKind::Warning,
        message: msg.to_string(),
        span: Some(span_to_info(source, expr_start, expr_end)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // RQ-4058 P0-3: vault reads, cookies.jar(), and {scope}.toObject() work at
    // runtime — they must NOT be flagged unsupported on import (stale false alarm).
    #[test]
    fn supported_vault_get_not_flagged() {
        let d = check_unsupported("pm.vault.get(\"s\")", "pm", &["vault", "get"], 0, 17);
        assert!(d.is_none());
    }

    // RQ-4998: rq.visualizer.set/clear are implemented (ADR-202 / RQ-4994) and the
    // pm.* → rq.* rename makes an imported pm.visualizer.set() run — it must NOT be
    // flagged unsupported on import (was a stale false alarm predating RQ-4994).
    #[test]
    fn supported_visualizer_not_flagged() {
        let d = check_unsupported("pm.visualizer.set(t)", "pm", &["visualizer", "set"], 0, 20);
        assert!(d.is_none());
    }

    #[test]
    fn supported_cookies_jar_not_flagged() {
        let d = check_unsupported("pm.cookies.jar()", "pm", &["cookies", "jar"], 0, 16);
        assert!(d.is_none());
    }

    #[test]
    fn supported_environment_clear() {
        let d = check_unsupported("pm.environment.clear()", "pm", &["environment", "clear"], 0, 22);
        assert!(d.is_none());
    }

    #[test]
    fn supported_globals_clear() {
        let d = check_unsupported("pm.globals.clear()", "pm", &["globals", "clear"], 0, 18);
        assert!(d.is_none());
    }

    #[test]
    fn supported_collection_variables_clear() {
        let d = check_unsupported("pm.collectionVariables.clear()", "pm", &["collectionVariables", "clear"], 0, 30);
        assert!(d.is_none());
    }

    #[test]
    fn supported_globals_to_object_not_flagged() {
        let d = check_unsupported("pm.globals.toObject()", "pm", &["globals", "toObject"], 0, 21);
        assert!(d.is_none());
    }

    #[test]
    fn supported_environment_to_object_not_flagged() {
        let d = check_unsupported("pm.environment.toObject()", "pm", &["environment", "toObject"], 0, 26);
        assert!(d.is_none());
    }

    // replaceIn remains a genuine partial-support divergence (`$` dynamic vars, ADR-055).
    #[test]
    fn partial_support_replace_in_still_flagged() {
        let d = check_unsupported("pm.variables.replaceIn(t)", "pm", &["variables", "replaceIn"], 0, 25);
        assert!(d.is_some());
        assert!(d.unwrap().message.contains("replaceIn"));
    }

    #[test]
    fn legacy_get_response_header_mapped() {
        let result = check_legacy_call(
            "postman.getResponseHeader(\"X\")",
            "postman",
            &["getResponseHeader"],
            0,
            29,
            0,
            7,
        );
        let (reps, _) = result.expect("getResponseHeader should be mapped");
        assert_eq!(reps.len(), 1);
        assert_eq!(reps[0].new_text, "rq.response.headers.get");
    }

    #[test]
    fn legacy_clear_global_variable_still_maps_to_unset() {
        let result = check_legacy_call(
            "postman.clearGlobalVariable(\"k\")",
            "postman",
            &["clearGlobalVariable"],
            0,
            31,
            0,
            7,
        );
        let (reps, _) = result.expect("clearGlobalVariable should be mapped");
        assert_eq!(reps[0].new_text, "rq.globals.unset");
    }

    #[test]
    fn supported_api_not_flagged() {
        let d = check_unsupported("pm.environment.get('k')", "pm", &["environment", "get"], 0, 23);
        assert!(d.is_none());
    }
}
