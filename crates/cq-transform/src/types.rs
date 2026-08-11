use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Replacement {
    pub start: u32,
    pub end: u32,
    pub new_text: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<SpanInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub enum DiagnosticKind {
    Replacement,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpanInfo {
    pub start: u32,
    pub end: u32,
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub replacements: u32,
    pub warnings: u32,
    pub errors: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransformResult {
    pub success: bool,
    pub code: String,
    pub diagnostics: Vec<Diagnostic>,
    pub summary: Summary,
}

/// A static `require()` call extracted from script source (ADR-084).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequireInfo {
    /// The raw string argument (e.g., `"lodash@4.17.21"`, `"@faker-js/faker"`).
    pub raw_id: String,
    /// Byte offset span of the entire `require('...')` call expression.
    pub span: SpanInfo,
}

/// Result of `extract_requires()`.
#[derive(Debug, Clone, Serialize)]
pub struct ExtractRequiresResult {
    pub requires: Vec<RequireInfo>,
}

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Postman,
}
