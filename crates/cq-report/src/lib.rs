//! # cq-report — diagnostics, provenance & fidelity
//!
//! Every `cross-q` conversion emits a [`Report`]: the structured record of what mapped
//! cleanly, what was coerced to fit, and what a target couldn't hold. This is how cross-q
//! keeps its central promise — *lossy is fine, silent is not*. If the tool made a
//! decision about your data, the decision is on the record.
//!
//! A [`Diagnostic`] carries a [`Severity`], the [`Phase`] it occurred in, a
//! [`cq_model::Provenance`] pointing back at the source location, and a human message.

use serde::{Deserialize, Serialize};

use cq_model::{Json, Provenance};

/// How a single decision landed, worst-last so ordering is meaningful.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Mapped cleanly, 1:1.
    Ok,
    /// A value was changed to make it fit (e.g. `key: null` → `""`).
    Coerced,
    /// The target couldn't represent this; it was left out.
    Dropped,
    /// The conversion could not complete this item.
    Error,
}

/// Which stage of the pipeline produced a diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Parse,
    Map,
    Emit,
}

/// The declared fidelity of a `source → target` conversion. Never overstated: a run's
/// summary names this up front.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fidelity {
    /// `A → IR → A` is byte-equivalent.
    RoundTrip,
    /// Everything in the source is representable in the target.
    Lossless,
    /// Some features have no target home; each is a `Dropped` diagnostic.
    Lossy,
    /// Structural downshift (e.g. gRPC → cURL); many `Dropped` diagnostics.
    Degraded,
}

/// One recorded decision about the data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub phase: Phase,
    pub provenance: Provenance,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Json>,
}

impl Diagnostic {
    pub fn new(
        severity: Severity,
        phase: Phase,
        provenance: Provenance,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            phase,
            provenance,
            message: message.into(),
            detail: None,
        }
    }

    /// Attach structured detail (shown in the machine-readable report).
    pub fn with_detail(mut self, detail: Json) -> Self {
        self.detail = Some(detail);
        self
    }
}

/// The full report for one conversion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// Declared fidelity for this conversion (may be tightened as diagnostics accrue —
    /// see [`Report::effective_fidelity`]).
    pub fidelity: Fidelity,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

impl Report {
    pub fn new(fidelity: Fidelity) -> Self {
        Self {
            fidelity,
            diagnostics: Vec::new(),
        }
    }

    /// Record a diagnostic.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    /// Convenience: record a `Coerced` decision.
    pub fn coerced(&mut self, phase: Phase, provenance: Provenance, message: impl Into<String>) {
        self.push(Diagnostic::new(
            Severity::Coerced,
            phase,
            provenance,
            message,
        ));
    }

    /// Convenience: record a `Dropped` decision.
    pub fn dropped(&mut self, phase: Phase, provenance: Provenance, message: impl Into<String>) {
        self.push(Diagnostic::new(
            Severity::Dropped,
            phase,
            provenance,
            message,
        ));
    }

    /// Convenience: record an `Error`.
    pub fn error(&mut self, phase: Phase, provenance: Provenance, message: impl Into<String>) {
        self.push(Diagnostic::new(Severity::Error, phase, provenance, message));
    }

    /// Count diagnostics of a given severity.
    pub fn count(&self, severity: Severity) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == severity)
            .count()
    }

    /// The most severe diagnostic recorded, if any.
    pub fn worst(&self) -> Option<Severity> {
        self.diagnostics.iter().map(|d| d.severity).max()
    }

    /// True if any diagnostic is an [`Severity::Error`].
    pub fn has_errors(&self) -> bool {
        self.count(Severity::Error) > 0
    }

    /// The fidelity implied by the diagnostics actually recorded — never *better* than the
    /// declared fidelity, and downgraded when data was dropped. This is what makes "never
    /// overstate fidelity" mechanical rather than a promise.
    pub fn effective_fidelity(&self) -> Fidelity {
        let declared = self.fidelity;
        let dropped = self.count(Severity::Dropped);
        let coerced = self.count(Severity::Coerced);
        let by_diag = if dropped >= 5 {
            Fidelity::Degraded
        } else if dropped > 0 {
            Fidelity::Lossy
        } else if coerced > 0 {
            Fidelity::Lossless
        } else {
            Fidelity::RoundTrip
        };
        // Take the *worse* (numerically larger) of declared vs implied.
        declared.max(by_diag)
    }

    /// A one-line human summary, e.g. `"lossy — 12 ok, 2 coerced, 1 dropped"`.
    pub fn summary(&self) -> String {
        format!(
            "{:?} — {} ok, {} coerced, {} dropped, {} error(s)",
            self.effective_fidelity(),
            self.count(Severity::Ok),
            self.count(Severity::Coerced),
            self.count(Severity::Dropped),
            self.count(Severity::Error),
        )
        .to_lowercase()
    }
}

// Ordering so `Fidelity::max` yields the *worse* fidelity. RoundTrip is best (smallest),
// Degraded is worst (largest).
impl PartialOrd for Fidelity {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Fidelity {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        fn rank(f: Fidelity) -> u8 {
            match f {
                Fidelity::RoundTrip => 0,
                Fidelity::Lossless => 1,
                Fidelity::Lossy => 2,
                Fidelity::Degraded => 3,
            }
        }
        rank(*self).cmp(&rank(*other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cq_model::SourceFormat;

    fn prov() -> Provenance {
        Provenance {
            format: SourceFormat::Postman,
            locator: "item[0].request.header[2].key".into(),
        }
    }

    #[test]
    fn severity_orders_worst_last() {
        assert!(Severity::Ok < Severity::Coerced);
        assert!(Severity::Coerced < Severity::Dropped);
        assert!(Severity::Dropped < Severity::Error);
    }

    #[test]
    fn counts_and_worst() {
        let mut r = Report::new(Fidelity::Lossless);
        r.coerced(Phase::Parse, prov(), "key: null coerced to empty string");
        r.dropped(
            Phase::Emit,
            prov(),
            "mqtt request has no Postman equivalent",
        );
        assert_eq!(r.count(Severity::Coerced), 1);
        assert_eq!(r.count(Severity::Dropped), 1);
        assert_eq!(r.worst(), Some(Severity::Dropped));
        assert!(!r.has_errors());
    }

    #[test]
    fn effective_fidelity_never_overstates() {
        // Declared lossless, but a drop happened => effective is at least lossy.
        let mut r = Report::new(Fidelity::Lossless);
        r.dropped(Phase::Emit, prov(), "dropped something");
        assert_eq!(r.effective_fidelity(), Fidelity::Lossy);

        // Declared degraded stays degraded even with no diagnostics.
        let r2 = Report::new(Fidelity::Degraded);
        assert_eq!(r2.effective_fidelity(), Fidelity::Degraded);

        // Many drops => degraded.
        let mut r3 = Report::new(Fidelity::RoundTrip);
        for _ in 0..5 {
            r3.dropped(Phase::Emit, prov(), "drop");
        }
        assert_eq!(r3.effective_fidelity(), Fidelity::Degraded);
    }

    #[test]
    fn report_round_trips_through_json() {
        let mut r = Report::new(Fidelity::Lossy);
        r.push(
            Diagnostic::new(
                Severity::Coerced,
                Phase::Parse,
                prov(),
                "numeric key -> string",
            )
            .with_detail(serde_json::json!({ "from": 42, "to": "42" })),
        );
        let json = serde_json::to_string_pretty(&r).unwrap();
        let back: Report = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn summary_is_readable() {
        let mut r = Report::new(Fidelity::Lossless);
        r.coerced(Phase::Parse, prov(), "x");
        let s = r.summary();
        assert!(s.contains("coerced"), "summary was: {s}");
    }
}
