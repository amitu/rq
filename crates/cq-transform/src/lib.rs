// Absorbed verbatim from the app's private `script-transform` crate (ADR/CONTEXT.md §1: the
// compat pillar of cross-q-context, open-sourced here). Kept a faithful copy — clippy style
// lints are allowed so the port doesn't diverge from upstream; a cleanup pass is a follow-up.
#![allow(clippy::all)]

mod platforms;
mod replacer;
mod scope;
pub mod types;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    AssignmentExpression, CallExpression, Expression, ForStatementInit, ForStatementLeft,
    MemberExpression, Program, Statement,
};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};
use wasm_bindgen::prelude::*;

use crate::platforms::postman;
use crate::replacer::{apply_replacements, dedup_overlapping, span_to_info};
use crate::scope::ScopeStack;
use crate::types::{
    Diagnostic, DiagnosticKind, ExtractRequiresResult, Platform, Replacement, RequireInfo, Summary,
    TransformResult,
};

#[wasm_bindgen]
pub fn transform(source: &str, platform: &str) -> String {
    let platform = match serde_json::from_value::<Platform>(serde_json::Value::String(platform.to_string())) {
        Ok(p) => p,
        Err(_) => {
            return format!(
                r#"{{"success":false,"code":"","diagnostics":[{{"kind":"Error","message":"Unknown platform: {platform}"}}],"summary":{{"replacements":0,"warnings":0,"errors":1}}}}"#
            );
        }
    };
    let result = full_transform(source, platform);
    serde_json::to_string(&result).unwrap_or_else(|e| {
        format!(
            r#"{{"success":false,"code":"","diagnostics":[{{"kind":"Error","message":"Serialization error: {e}"}}],"summary":{{"replacements":0,"warnings":0,"errors":1}}}}"#
        )
    })
}

/// WASM entry point: extract static `require()` calls from script source (ADR-084).
/// Returns JSON: `{ "requires": [{ "raw_id": "lodash@4.17.21", "span": { "start": 0, "end": 25, "line": 1, "col": 0 } }] }`
#[wasm_bindgen]
pub fn extract_requires(source: &str) -> String {
    let result = collect_requires(source);
    serde_json::to_string(&result).unwrap_or_else(|_| r#"{"requires":[]}"#.to_string())
}

fn collect_requires(source: &str) -> ExtractRequiresResult {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, source, source_type).parse();

    if !ret.errors.is_empty() {
        return ExtractRequiresResult {
            requires: Vec::new(),
        };
    }

    let mut requires = Vec::new();
    collect_requires_from_program(&ret.program, source, &mut requires);
    ExtractRequiresResult { requires }
}

fn collect_requires_from_program(program: &Program, source: &str, requires: &mut Vec<RequireInfo>) {
    for stmt in &program.body {
        collect_requires_from_stmt(stmt, source, requires);
    }
}

fn collect_requires_from_stmt(stmt: &Statement, source: &str, requires: &mut Vec<RequireInfo>) {
    match stmt {
        Statement::ExpressionStatement(expr_stmt) => {
            collect_requires_from_expr(&expr_stmt.expression, source, requires);
        }
        Statement::VariableDeclaration(decl) => {
            for declarator in &decl.declarations {
                if let Some(init) = &declarator.init {
                    collect_requires_from_expr(init, source, requires);
                }
            }
        }
        Statement::ReturnStatement(ret) => {
            if let Some(arg) = &ret.argument {
                collect_requires_from_expr(arg, source, requires);
            }
        }
        Statement::IfStatement(if_stmt) => {
            collect_requires_from_expr(&if_stmt.test, source, requires);
            collect_requires_from_stmt(&if_stmt.consequent, source, requires);
            if let Some(alt) = &if_stmt.alternate {
                collect_requires_from_stmt(alt, source, requires);
            }
        }
        Statement::BlockStatement(block) => {
            for s in &block.body {
                collect_requires_from_stmt(s, source, requires);
            }
        }
        Statement::TryStatement(try_stmt) => {
            for s in &try_stmt.block.body {
                collect_requires_from_stmt(s, source, requires);
            }
            if let Some(handler) = &try_stmt.handler {
                for s in &handler.body.body {
                    collect_requires_from_stmt(s, source, requires);
                }
            }
            if let Some(finalizer) = &try_stmt.finalizer {
                for s in &finalizer.body {
                    collect_requires_from_stmt(s, source, requires);
                }
            }
        }
        Statement::ForStatement(for_stmt) => {
            if let Some(init) = &for_stmt.init {
                if let Some(expr) = init.as_expression() {
                    collect_requires_from_expr(expr, source, requires);
                }
            }
            if let Some(test) = &for_stmt.test {
                collect_requires_from_expr(test, source, requires);
            }
            if let Some(update) = &for_stmt.update {
                collect_requires_from_expr(update, source, requires);
            }
            collect_requires_from_stmt(&for_stmt.body, source, requires);
        }
        Statement::WhileStatement(while_stmt) => {
            collect_requires_from_expr(&while_stmt.test, source, requires);
            collect_requires_from_stmt(&while_stmt.body, source, requires);
        }
        Statement::SwitchStatement(switch_stmt) => {
            collect_requires_from_expr(&switch_stmt.discriminant, source, requires);
            for case in &switch_stmt.cases {
                if let Some(test) = &case.test {
                    collect_requires_from_expr(test, source, requires);
                }
                for s in &case.consequent {
                    collect_requires_from_stmt(s, source, requires);
                }
            }
        }
        Statement::FunctionDeclaration(func) => {
            if let Some(body) = &func.body {
                for s in &body.statements {
                    collect_requires_from_stmt(s, source, requires);
                }
            }
        }
        _ => {}
    }
}

fn collect_requires_from_expr(expr: &Expression, source: &str, requires: &mut Vec<RequireInfo>) {
    match expr {
        Expression::CallExpression(call) => {
            // Check if callee is `require` identifier
            if let Expression::Identifier(ident) = &call.callee {
                if ident.name == "require" {
                    // Extract first argument if it's a string literal
                    if let Some(first_arg) = call.arguments.first() {
                        if let Some(arg_expr) = first_arg.as_expression() {
                            if let Expression::StringLiteral(s) = arg_expr {
                                requires.push(RequireInfo {
                                    raw_id: s.value.to_string(),
                                    span: crate::replacer::span_to_info(
                                        source,
                                        call.span.start,
                                        call.span.end,
                                    ),
                                });
                            }
                        }
                    }
                }
            }
            // Walk arguments for nested requires
            for arg in &call.arguments {
                if let Some(arg_expr) = arg.as_expression() {
                    collect_requires_from_expr(arg_expr, source, requires);
                }
            }
            // Walk callee for chained calls
            if let Some(member) = call.callee.as_member_expression() {
                collect_requires_from_member(member, source, requires);
            }
        }
        Expression::AssignmentExpression(assign) => {
            collect_requires_from_expr(&assign.right, source, requires);
        }
        Expression::ConditionalExpression(cond) => {
            collect_requires_from_expr(&cond.test, source, requires);
            collect_requires_from_expr(&cond.consequent, source, requires);
            collect_requires_from_expr(&cond.alternate, source, requires);
        }
        Expression::LogicalExpression(logic) => {
            collect_requires_from_expr(&logic.left, source, requires);
            collect_requires_from_expr(&logic.right, source, requires);
        }
        Expression::BinaryExpression(bin) => {
            collect_requires_from_expr(&bin.left, source, requires);
            collect_requires_from_expr(&bin.right, source, requires);
        }
        Expression::UnaryExpression(unary) => {
            collect_requires_from_expr(&unary.argument, source, requires);
        }
        Expression::ArrowFunctionExpression(arrow) => {
            if arrow.expression {
                if let Some(stmt) = arrow.body.statements.first() {
                    if let Statement::ExpressionStatement(expr_stmt) = stmt {
                        collect_requires_from_expr(&expr_stmt.expression, source, requires);
                    }
                }
            } else {
                for s in &arrow.body.statements {
                    collect_requires_from_stmt(s, source, requires);
                }
            }
        }
        Expression::FunctionExpression(func) => {
            if let Some(body) = &func.body {
                for s in &body.statements {
                    collect_requires_from_stmt(s, source, requires);
                }
            }
        }
        Expression::ObjectExpression(obj) => {
            for prop in &obj.properties {
                if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop {
                    collect_requires_from_expr(&p.value, source, requires);
                }
            }
        }
        Expression::ArrayExpression(arr) => {
            for elem in &arr.elements {
                if let oxc_ast::ast::ArrayExpressionElement::SpreadElement(spread) = elem {
                    collect_requires_from_expr(&spread.argument, source, requires);
                } else if let Some(e) = elem.as_expression() {
                    collect_requires_from_expr(e, source, requires);
                }
            }
        }
        Expression::SequenceExpression(seq) => {
            for e in &seq.expressions {
                collect_requires_from_expr(e, source, requires);
            }
        }
        Expression::AwaitExpression(await_expr) => {
            collect_requires_from_expr(&await_expr.argument, source, requires);
        }
        Expression::ParenthesizedExpression(paren) => {
            collect_requires_from_expr(&paren.expression, source, requires);
        }
        Expression::TemplateLiteral(tmpl) => {
            for e in &tmpl.expressions {
                collect_requires_from_expr(e, source, requires);
            }
        }
        expr if expr.as_member_expression().is_some() => {
            collect_requires_from_member(expr.as_member_expression().unwrap(), source, requires);
        }
        _ => {}
    }
}

fn collect_requires_from_member(
    member: &MemberExpression,
    source: &str,
    requires: &mut Vec<RequireInfo>,
) {
    match member {
        MemberExpression::StaticMemberExpression(static_member) => {
            collect_requires_from_expr(&static_member.object, source, requires);
        }
        MemberExpression::ComputedMemberExpression(computed) => {
            collect_requires_from_expr(&computed.object, source, requires);
            collect_requires_from_expr(&computed.expression, source, requires);
        }
        MemberExpression::PrivateFieldExpression(private) => {
            collect_requires_from_expr(&private.object, source, requires);
        }
    }
}

/// Wrapper used to make top-level `return` parseable. Postman executes scripts wrapped
/// in a function, so `return` at script scope is valid input. JavaScript parsers reject it.
/// Parsing inside the wrapper reproduces Postman's runtime shape; spans are shifted back
/// before replacements are applied to the original source.
const WRAPPER_PREFIX: &str = "(function(){\n";
const WRAPPER_SUFFIX: &str = "\n})()";

/// Native entry point: rewrite `source` from the given platform dialect to `rq.*`, returning the
/// rewritten code plus diagnostics. The `#[wasm_bindgen] transform()` above wraps this for JS;
/// native hosts (cross-q-context) call this directly.
pub fn full_transform(source: &str, platform: Platform) -> TransformResult {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, source, source_type).parse();

    if ret.errors.is_empty() {
        return build_result(source, &ret.program, platform, 0);
    }

    // Retry with a function wrapper to support Postman scripts with top-level `return`.
    let wrapped = format!("{WRAPPER_PREFIX}{source}{WRAPPER_SUFFIX}");
    let prefix_len = WRAPPER_PREFIX.len() as u32;
    let wrapper_allocator = Allocator::default();
    let wrapped_ret = Parser::new(&wrapper_allocator, &wrapped, source_type).parse();

    if wrapped_ret.errors.is_empty() {
        return build_result(&wrapped, &wrapped_ret.program, platform, prefix_len);
    }

    // Both parses failed — return original parse errors (genuine syntax error).
    let diagnostics: Vec<Diagnostic> = ret
        .errors
        .iter()
        .map(|e| Diagnostic {
            kind: DiagnosticKind::Error,
            message: format!("{e}"),
            span: None,
        })
        .collect();
    let error_count = diagnostics.len() as u32;
    TransformResult {
        success: false,
        code: source.to_string(),
        diagnostics,
        summary: Summary {
            replacements: 0,
            warnings: 0,
            errors: error_count,
        },
    }
}

/// Build a TransformResult from a parsed program. When `prefix_len` is non-zero, the program
/// was parsed from a wrapped source; replacement spans are shifted back to original-source
/// coordinates before application.
fn build_result(
    parse_source: &str,
    program: &Program,
    platform: Platform,
    prefix_len: u32,
) -> TransformResult {
    let mut replacements = Vec::new();
    let mut diagnostics = Vec::new();
    let mut scope = ScopeStack::new();

    collect_from_program(
        program,
        parse_source,
        platform,
        &mut scope,
        &mut replacements,
        &mut diagnostics,
    );

    dedup_overlapping(&mut replacements);

    // Shift spans from wrapped coordinates back to original coordinates when applicable.
    if prefix_len > 0 {
        replacements.retain_mut(|r| {
            if r.start < prefix_len {
                // Replacement targets the wrapper itself — drop it.
                return false;
            }
            r.start -= prefix_len;
            r.end = r.end.saturating_sub(prefix_len);
            true
        });
        for d in &mut diagnostics {
            if let Some(span) = d.span.as_mut() {
                if span.start >= prefix_len {
                    span.start -= prefix_len;
                    span.end = span.end.saturating_sub(prefix_len);
                    if span.line > 1 {
                        span.line -= 1;
                    }
                }
            }
        }
    }

    // Derive the original source slice for applying replacements and computing diagnostic spans.
    let original_source = if prefix_len > 0 {
        let start = prefix_len as usize;
        let end = parse_source.len().saturating_sub(WRAPPER_SUFFIX.len());
        &parse_source[start..end]
    } else {
        parse_source
    };

    let replacement_count = replacements.len() as u32;
    let warning_count = diagnostics
        .iter()
        .filter(|d| matches!(d.kind, DiagnosticKind::Warning))
        .count() as u32;

    let code = apply_replacements(original_source, &mut replacements);

    for r in &replacements {
        diagnostics.push(Diagnostic {
            kind: DiagnosticKind::Replacement,
            message: r.message.clone(),
            span: Some(span_to_info(original_source, r.start, r.end)),
        });
    }

    TransformResult {
        success: true,
        code,
        diagnostics,
        summary: Summary {
            replacements: replacement_count,
            warnings: warning_count,
            errors: 0,
        },
    }
}

fn collect_from_program(
    program: &Program,
    source: &str,
    platform: Platform,
    scope: &mut ScopeStack,
    replacements: &mut Vec<Replacement>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // Pre-collect bindings declared anywhere in this scope (declarations are hoisted, and a
    // reference may textually precede its declaration). Over-collection is safe.
    collect_bindings_from_statements(&program.body, scope);
    for stmt in &program.body {
        collect_from_statement(stmt, source, platform, scope, replacements, diagnostics);
    }
}

/// Collect the names declared by a list of statements into the current scope frame.
/// Covers `var`/`let`/`const` declarators and function declarations (both hoisted). Walks
/// only the immediate statement list — nested function/block scopes get their own frames.
fn collect_bindings_from_statements(statements: &[Statement], scope: &mut ScopeStack) {
    for stmt in statements {
        match stmt {
            Statement::VariableDeclaration(decl) => {
                for declarator in &decl.declarations {
                    scope.bind_pattern(&declarator.id);
                }
            }
            Statement::FunctionDeclaration(func) => {
                if let Some(id) = &func.id {
                    scope.bind(&id.name);
                }
            }
            _ => {}
        }
    }
}

fn collect_from_statement(
    stmt: &Statement,
    source: &str,
    platform: Platform,
    scope: &mut ScopeStack,
    replacements: &mut Vec<Replacement>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match stmt {
        Statement::ExpressionStatement(expr_stmt) => {
            collect_from_expr(&expr_stmt.expression, source, platform, scope, replacements, diagnostics);
        }
        Statement::VariableDeclaration(decl) => {
            for declarator in &decl.declarations {
                if let Some(init) = &declarator.init {
                    collect_from_expr(init, source, platform, scope, replacements, diagnostics);
                }
            }
        }
        Statement::ReturnStatement(ret) => {
            if let Some(arg) = &ret.argument {
                collect_from_expr(arg, source, platform, scope, replacements, diagnostics);
            }
        }
        Statement::IfStatement(if_stmt) => {
            collect_from_expr(&if_stmt.test, source, platform, scope, replacements, diagnostics);
            collect_from_statement(&if_stmt.consequent, source, platform, scope, replacements, diagnostics);
            if let Some(alt) = &if_stmt.alternate {
                collect_from_statement(alt, source, platform, scope, replacements, diagnostics);
            }
        }
        Statement::BlockStatement(block) => {
            // Block-scoped `let`/`const` (and hoisted function declarations) get a frame.
            scope.push();
            collect_bindings_from_statements(&block.body, scope);
            for s in &block.body {
                collect_from_statement(s, source, platform, scope, replacements, diagnostics);
            }
            scope.pop();
        }
        Statement::TryStatement(try_stmt) => {
            scope.push();
            collect_bindings_from_statements(&try_stmt.block.body, scope);
            for s in &try_stmt.block.body {
                collect_from_statement(s, source, platform, scope, replacements, diagnostics);
            }
            scope.pop();
            if let Some(handler) = &try_stmt.handler {
                scope.push();
                if let Some(param) = &handler.param {
                    scope.bind_pattern(&param.pattern);
                }
                collect_bindings_from_statements(&handler.body.body, scope);
                for s in &handler.body.body {
                    collect_from_statement(s, source, platform, scope, replacements, diagnostics);
                }
                scope.pop();
            }
            if let Some(finalizer) = &try_stmt.finalizer {
                scope.push();
                collect_bindings_from_statements(&finalizer.body, scope);
                for s in &finalizer.body {
                    collect_from_statement(s, source, platform, scope, replacements, diagnostics);
                }
                scope.pop();
            }
        }
        Statement::ForStatement(for_stmt) => {
            // The loop header introduces a lexical frame: a `for (let data = 0; ...)` declaration
            // binds `data` for the test/update and the body. Bind it so a loop variable named
            // after a legacy global (e.g. `data`, `globals`) is treated as user-bound and left
            // un-rewritten. Over-collection is safe — it only suppresses rewrites.
            scope.push();
            if let Some(init) = &for_stmt.init {
                if let ForStatementInit::VariableDeclaration(decl) = init {
                    for declarator in &decl.declarations {
                        scope.bind_pattern(&declarator.id);
                    }
                } else if let Some(expr) = init.as_expression() {
                    collect_from_expr(expr, source, platform, scope, replacements, diagnostics);
                }
            }
            if let Some(test) = &for_stmt.test {
                collect_from_expr(test, source, platform, scope, replacements, diagnostics);
            }
            if let Some(update) = &for_stmt.update {
                collect_from_expr(update, source, platform, scope, replacements, diagnostics);
            }
            collect_from_statement(&for_stmt.body, source, platform, scope, replacements, diagnostics);
            scope.pop();
        }
        Statement::WhileStatement(while_stmt) => {
            collect_from_expr(&while_stmt.test, source, platform, scope, replacements, diagnostics);
            collect_from_statement(&while_stmt.body, source, platform, scope, replacements, diagnostics);
        }
        Statement::SwitchStatement(switch_stmt) => {
            collect_from_expr(&switch_stmt.discriminant, source, platform, scope, replacements, diagnostics);
            for case in &switch_stmt.cases {
                if let Some(test) = &case.test {
                    collect_from_expr(test, source, platform, scope, replacements, diagnostics);
                }
                for s in &case.consequent {
                    collect_from_statement(s, source, platform, scope, replacements, diagnostics);
                }
            }
        }
        Statement::FunctionDeclaration(func) => {
            // The function's own name is bound in the enclosing scope (already done by
            // collect_bindings_from_statements). Its body gets a fresh frame with params.
            if let Some(body) = &func.body {
                scope.push();
                scope.bind_params(&func.params);
                collect_bindings_from_statements(&body.statements, scope);
                for s in &body.statements {
                    collect_from_statement(s, source, platform, scope, replacements, diagnostics);
                }
                scope.pop();
            }
        }
        Statement::ThrowStatement(throw_stmt) => {
            collect_from_expr(&throw_stmt.argument, source, platform, scope, replacements, diagnostics);
        }
        Statement::DoWhileStatement(do_while) => {
            collect_from_statement(&do_while.body, source, platform, scope, replacements, diagnostics);
            collect_from_expr(&do_while.test, source, platform, scope, replacements, diagnostics);
        }
        Statement::ForInStatement(for_in) => {
            // `for (let globals in obj)` binds `globals` for the body. Push a frame and bind the
            // loop-variable declaration so a loop var named after a legacy global is not rewritten.
            scope.push();
            if let ForStatementLeft::VariableDeclaration(decl) = &for_in.left {
                for declarator in &decl.declarations {
                    scope.bind_pattern(&declarator.id);
                }
            }
            collect_from_expr(&for_in.right, source, platform, scope, replacements, diagnostics);
            collect_from_statement(&for_in.body, source, platform, scope, replacements, diagnostics);
            scope.pop();
        }
        Statement::ForOfStatement(for_of) => {
            // `for (const request of arr)` binds `request` for the body. Push a frame and bind the
            // loop-variable declaration so a loop var named after a legacy global is not rewritten.
            scope.push();
            if let ForStatementLeft::VariableDeclaration(decl) = &for_of.left {
                for declarator in &decl.declarations {
                    scope.bind_pattern(&declarator.id);
                }
            }
            collect_from_expr(&for_of.right, source, platform, scope, replacements, diagnostics);
            collect_from_statement(&for_of.body, source, platform, scope, replacements, diagnostics);
            scope.pop();
        }
        Statement::LabeledStatement(labeled) => {
            collect_from_statement(&labeled.body, source, platform, scope, replacements, diagnostics);
        }
        _ => {}
    }
}

fn collect_from_expr(
    expr: &Expression,
    source: &str,
    platform: Platform,
    scope: &mut ScopeStack,
    replacements: &mut Vec<Replacement>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match expr {
        Expression::CallExpression(call) => {
            check_call(call, source, platform, scope, replacements, diagnostics);
            // Walk the callee when it's not a member expression (handled inside check_call).
            // Covers IIFEs like `(function(){ ... })()` used by the wrapper retry path.
            if call.callee.as_member_expression().is_none() {
                collect_from_expr(&call.callee, source, platform, scope, replacements, diagnostics);
            }
            // Also walk arguments
            for arg in &call.arguments {
                if let Some(expr) = arg.as_expression() {
                    collect_from_expr(expr, source, platform, scope, replacements, diagnostics);
                }
            }
        }
        expr if expr.as_member_expression().is_some() => {
            let member = expr.as_member_expression().unwrap();
            check_non_call_member(member, source, platform, scope, true, replacements, diagnostics);
        }
        Expression::AssignmentExpression(assign) => {
            // Legacy `tests["label"] = expr` → `rq.test("label", () => rq.expect(expr).to.be.ok)`.
            // Handled before walking children so the assignment can be rewritten as a whole.
            // Returns false when not a `tests[...]` assignment, in which case fall through to
            // the default sub-expression walk.
            if check_global_member_assignment(assign, scope, replacements) {
                // `globals.X = expr` / `environment.X = expr` rewritten as two RHS-bracketing
                // replacements (see check_global_member_assignment). The LHS member is consumed
                // by those brackets, so DON'T re-walk it; only walk the RHS for inner rewrites.
                collect_from_expr(&assign.right, source, platform, scope, replacements, diagnostics);
            } else if !check_tests_assignment(assign, source, scope, replacements) {
                collect_from_expr(&assign.right, source, platform, scope, replacements, diagnostics);
                // Check left side for member expressions.
                if let Some(member) = assign.left.as_member_expression() {
                    // GUARD (RQ-3463): COMPOUND assignment (`+= -= *= /= %= **= <<= >>=
                    // >>>= &= |= ^= &&= ||= ??=`) on a single-hop legacy-global member
                    // (`globals.n += 1`). `check_global_member_assignment` already declined
                    // (only plain `=` is a dictionary write), so without this guard the LHS
                    // member would fall to `check_non_call_member` and rewrite to
                    // `rq.globals.get('n') += 1` — assigning to a CALL expression, which is
                    // INVALID, unparseable JS. Leave the LHS bare (a safe no-op → Slice C
                    // runtime shim). The RHS was already walked above. Asymmetric-safety
                    // bias: a missed rewrite is recoverable; corrupt output is not.
                    if is_compound_assignment_on_legacy_global(assign, scope) {
                        // Intentionally do NOT walk the LHS member — leave it unrewritten.
                    } else {
                        check_non_call_member(member, source, platform, scope, true, replacements, diagnostics);
                    }
                }
            } else {
                // `tests[...]` rewrite is expressed as two non-overlapping replacements that
                // bracket the RHS (see check_tests_assignment), so the RHS is left for the
                // normal walk to rewrite any inner legacy identifiers it contains.
                collect_from_expr(&assign.right, source, platform, scope, replacements, diagnostics);
            }
        }
        Expression::ConditionalExpression(cond) => {
            collect_from_expr(&cond.test, source, platform, scope, replacements, diagnostics);
            collect_from_expr(&cond.consequent, source, platform, scope, replacements, diagnostics);
            collect_from_expr(&cond.alternate, source, platform, scope, replacements, diagnostics);
        }
        Expression::LogicalExpression(logic) => {
            collect_from_expr(&logic.left, source, platform, scope, replacements, diagnostics);
            collect_from_expr(&logic.right, source, platform, scope, replacements, diagnostics);
        }
        Expression::BinaryExpression(bin) => {
            collect_from_expr(&bin.left, source, platform, scope, replacements, diagnostics);
            collect_from_expr(&bin.right, source, platform, scope, replacements, diagnostics);
        }
        Expression::UnaryExpression(unary) => {
            collect_from_expr(&unary.argument, source, platform, scope, replacements, diagnostics);
        }
        Expression::TemplateLiteral(tmpl) => {
            for expr in &tmpl.expressions {
                collect_from_expr(expr, source, platform, scope, replacements, diagnostics);
            }
        }
        Expression::ArrowFunctionExpression(arrow) => {
            // Arrow body gets a fresh frame with its parameters bound.
            scope.push();
            scope.bind_params(&arrow.params);
            if arrow.expression {
                if let Some(stmt) = arrow.body.statements.first() {
                    if let Statement::ExpressionStatement(expr_stmt) = stmt {
                        collect_from_expr(
                            &expr_stmt.expression,
                            source,
                            platform,
                            scope,
                            replacements,
                            diagnostics,
                        );
                    }
                }
            } else {
                collect_bindings_from_statements(&arrow.body.statements, scope);
                for s in &arrow.body.statements {
                    collect_from_statement(s, source, platform, scope, replacements, diagnostics);
                }
            }
            scope.pop();
        }
        Expression::FunctionExpression(func) => {
            if let Some(body) = &func.body {
                scope.push();
                scope.bind_params(&func.params);
                collect_bindings_from_statements(&body.statements, scope);
                for s in &body.statements {
                    collect_from_statement(s, source, platform, scope, replacements, diagnostics);
                }
                scope.pop();
            }
        }
        Expression::ObjectExpression(obj) => {
            for prop in &obj.properties {
                if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop {
                    collect_from_expr(&p.value, source, platform, scope, replacements, diagnostics);
                }
            }
        }
        Expression::ArrayExpression(arr) => {
            for elem in &arr.elements {
                if let oxc_ast::ast::ArrayExpressionElement::SpreadElement(spread) = elem {
                    collect_from_expr(&spread.argument, source, platform, scope, replacements, diagnostics);
                } else if let Some(expr) = elem.as_expression() {
                    collect_from_expr(expr, source, platform, scope, replacements, diagnostics);
                }
            }
        }
        Expression::SequenceExpression(seq) => {
            for e in &seq.expressions {
                collect_from_expr(e, source, platform, scope, replacements, diagnostics);
            }
        }
        Expression::AwaitExpression(await_expr) => {
            collect_from_expr(&await_expr.argument, source, platform, scope, replacements, diagnostics);
        }
        Expression::ParenthesizedExpression(paren) => {
            collect_from_expr(&paren.expression, source, platform, scope, replacements, diagnostics);
        }
        // Identifier — check for standalone legacy globals (scope-gated).
        Expression::Identifier(ident) => {
            if !scope.is_bound(&ident.name) {
                check_postman_global_identifier(
                    &ident.name,
                    ident.span.start,
                    ident.span.end,
                    replacements,
                );
            }
        }
        _ => {}
    }
}

/// Check standalone (bare) Postman global identifiers that map to a simple `rq.*` expression
/// by a whole-identifier span swap. The caller must have already confirmed the name is NOT
/// shadowed by a user binding (scope-gated).
fn check_postman_global_identifier(
    name: &str,
    start: u32,
    end: u32,
    replacements: &mut Vec<Replacement>,
) {
    let (new_text, message) = match name {
        "responseBody" => ("rq.response.text()", "responseBody → rq.response.text()"),
        "responseHeaders" => ("rq.response.headers", "responseHeaders → rq.response.headers"),
        "responseCookies" => ("rq.cookies", "responseCookies → rq.cookies"),
        "responseTime" => ("rq.response.responseTime", "responseTime → rq.response.responseTime"),
        "iteration" => ("rq.info.iteration", "iteration → rq.info.iteration"),
        "request" => ("rq.request", "request → rq.request"),
        _ => return,
    };
    replacements.push(Replacement {
        start,
        end,
        new_text: new_text.to_string(),
        message: message.to_string(),
    });
}

/// The five methods exposed by `rq.globals` / `rq.environment`. A member access whose property
/// is one of these is a method reference (pass-through), not a variable read.
const GLOBAL_METHODS: [&str; 5] = ["get", "set", "unset", "has", "toObject"];

fn is_global_method(name: &str) -> bool {
    GLOBAL_METHODS.contains(&name)
}

fn is_legacy_global_root(name: &str) -> bool {
    name == "globals" || name == "environment"
}

/// If `member` is a single-hop access directly off a bare `globals` / `environment` identifier,
/// return the root identifier and the accessed property name. The property is:
///   - `Some(name)` for `globals.name` (static) or `globals['name']` (computed string literal),
///   - `None` for a computed dynamic key (`globals[k]`) — a root-prefix swap candidate.
/// Returns `None` entirely when the object side is not a bare legacy-global identifier (e.g. the
/// outer node of a multi-hop chain `globals.a.b`, whose object is itself a member expression).
fn legacy_global_member_access<'a>(
    member: &'a MemberExpression<'a>,
) -> Option<(&'a oxc_ast::ast::IdentifierReference<'a>, Option<&'a str>)> {
    match member {
        MemberExpression::StaticMemberExpression(static_member) => {
            if let Expression::Identifier(ident) = &static_member.object {
                if is_legacy_global_root(&ident.name) {
                    return Some((ident, Some(static_member.property.name.as_str())));
                }
            }
            None
        }
        MemberExpression::ComputedMemberExpression(computed) => {
            if let Expression::Identifier(ident) = &computed.object {
                if is_legacy_global_root(&ident.name) {
                    let prop = match &computed.expression {
                        Expression::StringLiteral(s) => Some(s.value.as_str()),
                        _ => None,
                    };
                    return Some((ident, prop));
                }
            }
            None
        }
        MemberExpression::PrivateFieldExpression(_) => None,
    }
}

/// The `rq.*` root that a legacy `globals` / `environment` root maps to.
fn legacy_global_rq_root(name: &str) -> &'static str {
    if name == "globals" {
        "rq.globals"
    } else {
        "rq.environment"
    }
}

/// Diagnostic message for a legacy global root-prefix swap (pass-through forms).
fn legacy_global_message(name: &str) -> &'static str {
    if name == "globals" {
        "globals.* → rq.globals.*"
    } else {
        "environment.* → rq.environment.*"
    }
}

/// Legacy Postman `globals.<name> = <rhs>` / `environment.<name> = <rhs>` variable ASSIGNMENT.
///
/// In legacy Postman these are dictionary writes, semantically `globals.set('<name>', <rhs>)`.
/// Rewrites to `rq.globals.set('<name>', <rhs>)` (or `rq.environment.set(...)`). Returns `true`
/// when matched and rewritten (so the caller skips the default LHS member walk), `false`
/// otherwise.
///
/// Only the single-hop static-member or computed-string-literal-key form with a non-method
/// property name is handled. To compose with inner RHS rewrites under the widest-span dedup
/// rule, the rewrite is expressed as two non-overlapping replacements that bracket the RHS:
///   - `[assign.start .. rhs.start)`  →  `rq.globals.set('<name>', `
///   - `[rhs.end .. assign.end)`      →  `)`
fn check_global_member_assignment(
    assign: &AssignmentExpression,
    scope: &ScopeStack,
    replacements: &mut Vec<Replacement>,
) -> bool {
    // Only a plain `=` assignment is a dictionary write; compound assignment (`+=` etc.) is not.
    if assign.operator != oxc_ast::ast::AssignmentOperator::Assign {
        return false;
    }

    let Some(member) = assign.left.as_member_expression() else {
        return false;
    };
    let (root_name, _root_start, _root_end, chain) = extract_member_chain(member);
    if !is_legacy_global_root(&root_name) || scope.is_bound(&root_name) {
        return false;
    }

    // Resolve a single-hop, non-method property name.
    let prop_name: Option<&str> = match member {
        MemberExpression::StaticMemberExpression(_) if chain.len() == 1 => Some(chain[0].as_str()),
        MemberExpression::ComputedMemberExpression(computed) => {
            if let Expression::StringLiteral(s) = &computed.expression {
                Some(s.value.as_str())
            } else {
                None
            }
        }
        _ => None,
    };
    let Some(name) = prop_name else {
        return false;
    };
    if is_global_method(name) {
        return false;
    }

    let rq_root = legacy_global_rq_root(&root_name);
    let assign_span = assign.span();
    let rhs_span = assign.right.span();

    replacements.push(Replacement {
        start: assign_span.start,
        end: rhs_span.start,
        new_text: format!("{rq_root}.set('{name}', "),
        message: format!("{root_name}.X = expr → {rq_root}.set('X', expr)"),
    });
    replacements.push(Replacement {
        start: rhs_span.end,
        end: assign_span.end,
        new_text: ")".to_string(),
        message: format!("{root_name}.X = expr → {rq_root}.set('X', expr)"),
    });

    true
}

/// GUARD predicate (RQ-3463): is `assign` a COMPOUND assignment (`+=`, `-=`, `??=`, …, i.e. any
/// operator other than plain `=`) whose left-hand side is a member access rooted at an unshadowed
/// bare legacy global (`globals` / `environment`)?
///
/// When true, the LHS member must be LEFT UNREWRITTEN: a read-rewrite to `rq.globals.get('n')`
/// on the LHS of `+=` produces `rq.globals.get('n') += 1`, which assigns to a call expression —
/// invalid, unparseable JS. (`check_global_member_assignment` only rewrites plain `=`, so the
/// dictionary-write path never fires here.)
fn is_compound_assignment_on_legacy_global(assign: &AssignmentExpression, scope: &ScopeStack) -> bool {
    // Plain `=` is handled elsewhere (dictionary write); only compound operators are guarded.
    if assign.operator == oxc_ast::ast::AssignmentOperator::Assign {
        return false;
    }
    let Some(member) = assign.left.as_member_expression() else {
        return false;
    };
    // Root must be a bare (unshadowed) legacy global identifier. Covers static (`globals.n`) and
    // computed (`globals['n']`, `globals[k]`) single-hop members via legacy_global_member_access,
    // and deeper static chains via extract_member_chain.
    if let Some((root_ident, _prop)) = legacy_global_member_access(member) {
        return !scope.is_bound(&root_ident.name);
    }
    let (root_name, _root_start, _root_end, _chain) = extract_member_chain(member);
    is_legacy_global_root(&root_name) && !scope.is_bound(&root_name)
}

fn check_call(
    call: &CallExpression,
    source: &str,
    platform: Platform,
    scope: &mut ScopeStack,
    replacements: &mut Vec<Replacement>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(member) = call.callee.as_member_expression() {
        let (root_name, root_start, root_end, chain) = extract_member_chain(member);

        if !root_name.is_empty() {
            // Phase 3: Check for unsupported APIs (diagnostics only)
            let chain_refs: Vec<&str> = chain.iter().map(|s| s.as_str()).collect();
            if let Some(diag) = postman::check_unsupported(
                source,
                &root_name,
                &chain_refs,
                member.span().start,
                call.span.end,
            ) {
                diagnostics.push(diag);
            }

            // Phase 2: Legacy API transforms
            let chain_refs: Vec<&str> = chain.iter().map(|s| s.as_str()).collect();
            if let Some((reps, diags)) = postman::check_legacy_call(
                source,
                &root_name,
                &chain_refs,
                call.span.start,
                call.span.end,
                root_start,
                root_end,
            ) {
                replacements.extend(reps);
                diagnostics.extend(diags);
                collect_from_member_object(member, source, platform, scope, replacements, diagnostics);
                return;
            }

            // Phase 2b: Bare legacy global call roots — `globals.*` / `environment.*` →
            // `rq.globals.*` / `rq.environment.*`. Scope-gated: a user-declared `globals`
            // or `environment` is left untouched. Rewrites only the root identifier span.
            if !scope.is_bound(&root_name) {
                let bare_root = match root_name.as_str() {
                    "globals" => Some(("rq.globals", "globals.* → rq.globals.*")),
                    "environment" => Some(("rq.environment", "environment.* → rq.environment.*")),
                    _ => None,
                };
                if let Some((new_text, message)) = bare_root {
                    replacements.push(Replacement {
                        start: root_start,
                        end: root_end,
                        new_text: new_text.to_string(),
                        message: message.to_string(),
                    });
                }
            }

            // Phase 1: Namespace rename (pm/postman → rq)
            if root_name == "pm" || root_name == "postman" {
                replacements.push(Replacement {
                    start: root_start,
                    end: root_end,
                    new_text: "rq".to_string(),
                    message: format!("{root_name} → rq"),
                });
            }
        }

        // Walk the callee's deeper expressions (handles chained calls like pm.expect(1).to.equal(1))
        collect_from_member_object(member, source, platform, scope, replacements, diagnostics);
    }
}

fn check_non_call_member(
    member: &MemberExpression,
    source: &str,
    platform: Platform,
    scope: &mut ScopeStack,
    is_root_member: bool,
    replacements: &mut Vec<Replacement>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // Phase 2c: Bare legacy global member access — `globals.X` / `environment.X`.
    // Scope-gated: a user-declared `globals` / `environment` is left untouched.
    //
    // In legacy Postman, `globals` / `environment` are dictionaries: `globals.checkErrorWarning`
    // is a VARIABLE READ of the global named `checkErrorWarning`, semantically
    // `globals.get('checkErrorWarning')`. The `rq.globals` object exposes ONLY methods
    // (get/set/unset/has/toObject), so a bare-property read must be translated to a `.get(...)`
    // call — a root-prefix swap alone would produce `rq.globals.checkErrorWarning` (== undefined).
    //
    // Handled directly off the member node (NOT via extract_member_chain, which only models
    // static-member chains and returns an empty root for a computed member like `globals['x']`).
    // Only fires when this member node is the COMPLETE access (`is_root_member`); a deeper chain
    // segment like the inner `globals.a` of `globals.a.b` is left to the root-prefix swap below.
    //   - `globals.<name>`     (static member, single hop, <name> not a method) → `rq.globals.get('<name>')`
    //   - `globals['<name>']`  (computed string-literal key)                    → `rq.globals.get('<name>')`
    //   - `globals.<method>`   (bare method reference, no call)                 → `rq.globals.<method>` (root swap)
    //   - `globals[k]` (dynamic key) / `globals.a.b` (multi-hop)                → `rq.globals...`     (root swap)
    // Method CALLS (`globals.get(...)`) never reach here — they ride check_call Phase 2b.
    if is_root_member {
        if let Some((root_ident, prop)) = legacy_global_member_access(member) {
            let root_name = &root_ident.name;
            if !scope.is_bound(root_name) {
                let rq_root = legacy_global_rq_root(root_name);
                let message = legacy_global_message(root_name);
                let root_start = root_ident.span.start;
                let root_end = root_ident.span.end;
                match prop {
                    // Property READ → `rq.<root>.get('<name>')` over the whole member span.
                    // Single-hop static prop or computed string-literal key, non-method.
                    Some(name) if !is_global_method(name) => {
                        replacements.push(Replacement {
                            start: root_start,
                            end: member.span().end,
                            new_text: format!("{rq_root}.get('{name}')"),
                            message: format!("{root_name}.X read → {rq_root}.get('X')"),
                        });
                        return;
                    }
                    // Bare method REFERENCE (no call) — `globals.get` etc. → root-prefix swap.
                    // Safe: `rq.globals.get` is the live method. Only single-hop static-member
                    // method names reach here (computed keys yield `None`).
                    Some(_method) => {
                        replacements.push(Replacement {
                            start: root_start,
                            end: root_end,
                            new_text: rq_root.to_string(),
                            message: message.to_string(),
                        });
                        // Recurse into the object side for any nested rewrites, then stop:
                        // the root has been rewritten in place.
                        collect_from_member_object(
                            member,
                            source,
                            platform,
                            scope,
                            replacements,
                            diagnostics,
                        );
                        return;
                    }
                    // GUARD (RQ-3463): computed key that is NOT a plain string literal
                    // (Identifier / template literal / any expression) — `globals[k]`,
                    // globals[`a`]. A root-prefix swap would yield `rq.globals[k]`, and
                    // `rq.globals` exposes only methods, so the read is undefined. Leaving
                    // it bare is a safe no-op: it falls through to the Slice C runtime shim.
                    // Asymmetric-safety bias: a missed rewrite is recoverable; corrupt
                    // output is not. Do NOT recurse (no inner pm.* in a single-hop key
                    // worth the corruption risk); leave the member entirely unrewritten.
                    None => {
                        return;
                    }
                }
            }
        }
    }

    let (root_name, root_start, root_end, chain) = extract_member_chain(member);

    if !root_name.is_empty() {
        // Phase 2: responseCode.{code,name,detail} → rq.response.{code,name,detail}.
        // Scope-gated: a user-declared `responseCode` is left untouched.
        if root_name == "responseCode" && !scope.is_bound("responseCode") {
            let chain_refs: Vec<&str> = chain.iter().map(|s| s.as_str()).collect();
            let mapped = match chain_refs.as_slice() {
                ["code"] => Some("rq.response.code"),
                ["name"] => Some("rq.response.name"),
                ["detail"] => Some("rq.response.detail"),
                _ => None,
            };
            if let Some(new_text) = mapped {
                let message = match chain_refs.as_slice() {
                    ["code"] => "responseCode.code → rq.response.code",
                    ["name"] => "responseCode.name → rq.response.name",
                    _ => "responseCode.detail → rq.response.detail",
                };
                replacements.push(Replacement {
                    start: root_start,
                    end: member.span().end,
                    new_text: new_text.to_string(),
                    message: message.to_string(),
                });
                return;
            }
        }

        // Phase 2b: `data.X` (static single-hop) → `rq.iterationData.get('X')`.
        // Scope-gated; only the static-property single-hop form is handled, and only when this
        // member node is the COMPLETE access (`is_root_member`) — `data.a.b` is left untouched
        // because rewriting the inner `data.a` would corrupt the deeper chain.
        if is_root_member && root_name == "data" && !scope.is_bound("data") && chain.len() == 1 {
            if let MemberExpression::StaticMemberExpression(_) = member {
                let prop = &chain[0];
                replacements.push(Replacement {
                    start: root_start,
                    end: member.span().end,
                    new_text: format!("rq.iterationData.get('{prop}')"),
                    message: "data.X → rq.iterationData.get(...)".to_string(),
                });
                return;
            }
        }

        // Phase 2c GUARD (RQ-3463): a legacy global at the root of a DEEP member chain
        // (e.g. the outer `globals.a.b`, which the single-hop `legacy_global_member_access`
        // above does not match because its object is itself a member). A root-prefix swap
        // here yields `rq.globals.a.b`, but `rq.globals` exposes only methods, so the read
        // is undefined — wrong. Leaving the chain bare is a safe no-op: it falls through to
        // the Slice C runtime shim + deprecation warning. Asymmetric-safety bias: a missed
        // rewrite is recoverable; corrupt output is not. The single-hop safe forms are
        // already handled above via `legacy_global_member_access`; this arm intentionally
        // does NOT rewrite, so a deep chain is preserved verbatim.
        //
        // (No replacement emitted for `globals`/`environment` deep chains.)

        // Phase 1: Namespace rename on non-call member expressions
        if root_name == "pm" || root_name == "postman" {
            replacements.push(Replacement {
                start: root_start,
                end: root_end,
                new_text: "rq".to_string(),
                message: format!("{root_name} → rq"),
            });
        }
    }

    // Always recurse into the object side to find deeper expressions
    collect_from_member_object(member, source, platform, scope, replacements, diagnostics);
}

/// Walk the object side of a member expression to find nested pm.* references.
fn collect_from_member_object(
    member: &MemberExpression,
    source: &str,
    platform: Platform,
    scope: &mut ScopeStack,
    replacements: &mut Vec<Replacement>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match member {
        MemberExpression::StaticMemberExpression(static_member) => {
            walk_member_object(&static_member.object, source, platform, scope, replacements, diagnostics);
        }
        MemberExpression::ComputedMemberExpression(computed) => {
            walk_member_object(&computed.object, source, platform, scope, replacements, diagnostics);
            collect_from_expr(&computed.expression, source, platform, scope, replacements, diagnostics);
        }
        MemberExpression::PrivateFieldExpression(private) => {
            walk_member_object(&private.object, source, platform, scope, replacements, diagnostics);
        }
    }
}

/// Walk an expression that sits on the OBJECT side of an enclosing member expression. When the
/// object is itself a member expression, it is a deeper segment of the same access chain (not a
/// complete access), so it is checked with `is_root_member = false` — this prevents partial
/// rewrites like `data.a` firing inside `data.a.b`. All other expression kinds defer to the
/// normal `collect_from_expr` walk.
fn walk_member_object(
    expr: &Expression,
    source: &str,
    platform: Platform,
    scope: &mut ScopeStack,
    replacements: &mut Vec<Replacement>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(member) = expr.as_member_expression() {
        check_non_call_member(member, source, platform, scope, false, replacements, diagnostics);
    } else {
        collect_from_expr(expr, source, platform, scope, replacements, diagnostics);
    }
}

/// Extract the root identifier and method chain from a member expression.
/// e.g. `pm.environment.get` → ("pm", start, end, ["environment", "get"])
fn extract_member_chain(member: &MemberExpression) -> (String, u32, u32, Vec<String>) {
    let mut chain = Vec::new();
    let mut current = member;

    loop {
        match current {
            MemberExpression::StaticMemberExpression(static_member) => {
                chain.push(static_member.property.name.to_string());
                match &static_member.object {
                    Expression::Identifier(ident) => {
                        return (
                            ident.name.to_string(),
                            ident.span.start,
                            ident.span.end,
                            {
                                chain.reverse();
                                chain
                            },
                        );
                    }
                    expr if expr.as_member_expression().is_some() => {
                        current = expr.as_member_expression().unwrap();
                    }
                    _ => {
                        chain.reverse();
                        return (String::new(), 0, 0, chain);
                    }
                }
            }
            _ => {
                chain.reverse();
                return (String::new(), 0, 0, chain);
            }
        }
    }
}

/// Legacy Postman `tests["label"] = <rhs>` assertion assignment.
///
/// Rewrites it to `rq.test("label", () => rq.expect(<rhs>).to.be.ok)`. Returns `true` when
/// the assignment matched and was rewritten (so the caller skips the default LHS member walk),
/// `false` otherwise.
///
/// Only the computed string-literal-key form is handled (`tests["x"] = e`); `tests.foo = e`
/// (static member) and non-string-literal keys are out of scope and left untouched.
///
/// To compose with inner RHS rewrites under the engine's widest-span dedup rule, the rewrite is
/// expressed as **two non-overlapping replacements that bracket the RHS** rather than one wide
/// whole-assignment span. The RHS source range is left untouched, so any legacy identifiers
/// inside the RHS are rewritten independently by the normal walk:
///   - `[assign.start .. rhs.start)`  →  `rq.test("label", () => rq.expect(`
///   - `[rhs.end .. assign.end)`      →  `).to.be.ok)`
fn check_tests_assignment(
    assign: &AssignmentExpression,
    source: &str,
    scope: &ScopeStack,
    replacements: &mut Vec<Replacement>,
) -> bool {
    // Only a plain `=` assignment defines a test; compound/logical assignment (`+=`, `||=`,
    // etc.) carries semantics that the `rq.test(...)` rewrite would silently discard.
    if assign.operator != oxc_ast::ast::AssignmentOperator::Assign {
        return false;
    }

    // LHS must be `tests[<string-literal>]` — a computed member rooted at bare `tests`.
    let Some(MemberExpression::ComputedMemberExpression(computed)) =
        assign.left.as_member_expression()
    else {
        return false;
    };

    let Expression::Identifier(root) = &computed.object else {
        return false;
    };
    if root.name != "tests" || scope.is_bound("tests") {
        return false;
    }

    // The computed key must be a string literal; capture its verbatim source (preserves quoting).
    let Expression::StringLiteral(key) = &computed.expression else {
        return false;
    };
    let label = &source[key.span.start as usize..key.span.end as usize];

    let assign_span = assign.span();
    let rhs_span = assign.right.span();

    // Left bracket: from the start of the assignment up to (exclusive) the RHS.
    replacements.push(Replacement {
        start: assign_span.start,
        end: rhs_span.start,
        new_text: format!("rq.test({label}, () => rq.expect("),
        message: "tests[...] = expr → rq.test(...)".to_string(),
    });
    // Right bracket: from the end of the RHS to the end of the assignment.
    replacements.push(Replacement {
        start: rhs_span.end,
        end: assign_span.end,
        new_text: ").to.be.ok)".to_string(),
        message: "tests[...] = expr → rq.test(...)".to_string(),
    });

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transform_code(source: &str) -> TransformResult {
        full_transform(source, Platform::Postman)
    }

    // ── extract_requires tests ──────────────────────────────────

    #[test]
    fn extract_string_literal_require() {
        let result = collect_requires("const x = require('lodash')");
        assert_eq!(result.requires.len(), 1);
        assert_eq!(result.requires[0].raw_id, "lodash");
    }

    #[test]
    fn extract_versioned_require() {
        let result = collect_requires("require('lodash@4.17.21')");
        assert_eq!(result.requires.len(), 1);
        assert_eq!(result.requires[0].raw_id, "lodash@4.17.21");
    }

    #[test]
    fn extract_scoped_require() {
        let result = collect_requires("require('@faker-js/faker@9.0.0')");
        assert_eq!(result.requires.len(), 1);
        assert_eq!(result.requires[0].raw_id, "@faker-js/faker@9.0.0");
    }

    #[test]
    fn extract_deep_import() {
        let result = collect_requires("require('lodash/fp')");
        assert_eq!(result.requires.len(), 1);
        assert_eq!(result.requires[0].raw_id, "lodash/fp");
    }

    #[test]
    fn extract_multiple_requires() {
        let result = collect_requires("const a = require('lodash'); const b = require('moment')");
        assert_eq!(result.requires.len(), 2);
        assert_eq!(result.requires[0].raw_id, "lodash");
        assert_eq!(result.requires[1].raw_id, "moment");
    }

    #[test]
    fn skip_dynamic_require() {
        let result = collect_requires("require(varName)");
        assert_eq!(result.requires.len(), 0);
    }

    #[test]
    fn skip_template_literal_require() {
        let result = collect_requires("require(`pkg`)");
        assert_eq!(result.requires.len(), 0);
    }

    #[test]
    fn skip_binary_expression_require() {
        let result = collect_requires("require('pkg' + ver)");
        assert_eq!(result.requires.len(), 0);
    }

    #[test]
    fn skip_comment_require() {
        let result = collect_requires("// require('lodash')");
        assert_eq!(result.requires.len(), 0);
    }

    #[test]
    fn extract_no_requires() {
        let result = collect_requires("console.log('hello')");
        assert_eq!(result.requires.len(), 0);
    }

    #[test]
    fn extract_empty_source() {
        let result = collect_requires("");
        assert_eq!(result.requires.len(), 0);
    }

    #[test]
    fn extract_parse_error() {
        let result = collect_requires("const { = [");
        assert_eq!(result.requires.len(), 0);
    }

    #[test]
    fn extract_require_in_if_block() {
        let result = collect_requires("if (x) { require('lodash') }");
        assert_eq!(result.requires.len(), 1);
        assert_eq!(result.requires[0].raw_id, "lodash");
    }

    #[test]
    fn extract_require_in_function() {
        let result = collect_requires("function f() { require('moment') }");
        assert_eq!(result.requires.len(), 1);
        assert_eq!(result.requires[0].raw_id, "moment");
    }

    #[test]
    fn extract_require_no_args() {
        let result = collect_requires("require()");
        assert_eq!(result.requires.len(), 0);
    }

    // Engine tests
    #[test]
    fn parse_error_returns_failure() {
        let result = transform_code("const { = [");
        assert!(!result.success);
        assert!(result.summary.errors > 0);
    }

    #[test]
    fn empty_string_succeeds() {
        let result = transform_code("");
        assert!(result.success);
        assert_eq!(result.code, "");
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn no_api_calls_unchanged() {
        let result = transform_code("const x = 1 + 2;");
        assert!(result.success);
        assert_eq!(result.code, "const x = 1 + 2;");
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn already_rq_unchanged() {
        let result = transform_code("rq.test(\"x\", fn)");
        assert!(result.success);
        assert_eq!(result.code, "rq.test(\"x\", fn)");
    }

    // Phase 1: Namespace rename
    #[test]
    fn pm_environment_get() {
        let result = transform_code("pm.environment.get(\"key\")");
        assert!(result.success);
        assert_eq!(result.code, "rq.environment.get(\"key\")");
    }

    #[test]
    fn pm_environment_set() {
        let result = transform_code("pm.environment.set(\"k\", \"v\")");
        assert!(result.success);
        assert_eq!(result.code, "rq.environment.set(\"k\", \"v\")");
    }

    #[test]
    fn pm_globals_get() {
        let result = transform_code("pm.globals.get(\"key\")");
        assert!(result.success);
        assert_eq!(result.code, "rq.globals.get(\"key\")");
    }

    #[test]
    fn pm_test() {
        let result = transform_code("pm.test(\"name\", function() {})");
        assert!(result.success);
        assert_eq!(result.code, "rq.test(\"name\", function() {})");
    }

    #[test]
    fn pm_expect() {
        let result = transform_code("pm.expect(value)");
        assert!(result.success);
        assert_eq!(result.code, "rq.expect(value)");
    }

    #[test]
    fn pm_response_json() {
        let result = transform_code("pm.response.json()");
        assert!(result.success);
        assert_eq!(result.code, "rq.response.json()");
    }

    #[test]
    fn pm_response_code() {
        let result = transform_code("pm.response.code");
        assert!(result.success);
        assert_eq!(result.code, "rq.response.code");
    }

    #[test]
    fn pm_send_request() {
        let result = transform_code("pm.sendRequest(url, cb)");
        assert!(result.success);
        assert_eq!(result.code, "rq.sendRequest(url, cb)");
    }

    #[test]
    fn multiple_pm_in_one_line() {
        let result = transform_code("pm.test(\"a\", function() { pm.expect(1).to.equal(1) })");
        assert!(result.success);
        assert!(result.code.contains("rq.test"));
        assert!(result.code.contains("rq.expect"));
        assert!(!result.code.contains("pm."));
    }

    #[test]
    fn nested_pm_expect_pm_response() {
        let result = transform_code("pm.expect(pm.response.json())");
        assert!(result.success);
        assert_eq!(result.code, "rq.expect(rq.response.json())");
    }

    #[test]
    fn deep_chain() {
        let result =
            transform_code("pm.expect(pm.response.responseTime).to.be.below(500)");
        assert!(result.success);
        assert!(result.code.contains("rq.expect(rq.response.responseTime)"));
    }

    // Phase 2: Legacy API transforms
    #[test]
    fn postman_set_environment_variable() {
        let result = transform_code("postman.setEnvironmentVariable(\"k\",\"v\")");
        assert!(result.success);
        assert_eq!(result.code, "rq.environment.set(\"k\",\"v\")");
    }

    #[test]
    fn postman_get_environment_variable() {
        let result = transform_code("postman.getEnvironmentVariable(\"k\")");
        assert!(result.success);
        assert_eq!(result.code, "rq.environment.get(\"k\")");
    }

    #[test]
    fn postman_clear_environment_variable() {
        let result = transform_code("postman.clearEnvironmentVariable(\"k\")");
        assert!(result.success);
        assert_eq!(result.code, "rq.environment.unset(\"k\")");
    }

    #[test]
    fn postman_set_global_variable() {
        let result = transform_code("postman.setGlobalVariable(\"k\",\"v\")");
        assert!(result.success);
        assert_eq!(result.code, "rq.globals.set(\"k\",\"v\")");
    }

    #[test]
    fn postman_get_global_variable() {
        let result = transform_code("postman.getGlobalVariable(\"k\")");
        assert!(result.success);
        assert_eq!(result.code, "rq.globals.get(\"k\")");
    }

    #[test]
    fn postman_clear_global_variable() {
        let result = transform_code("postman.clearGlobalVariable(\"k\")");
        assert!(result.success);
        assert_eq!(result.code, "rq.globals.unset(\"k\")");
    }

    #[test]
    fn postman_get_response_header() {
        let result = transform_code("postman.getResponseHeader(\"X\")");
        assert!(result.success);
        assert_eq!(result.code, "rq.response.headers.get(\"X\")");
    }

    #[test]
    fn postman_set_next_request() {
        let result = transform_code("postman.setNextRequest(\"name\")");
        assert!(result.success);
        assert_eq!(result.code, "rq.execution.setNextRequest(\"name\")");
    }

    #[test]
    fn response_body_standalone() {
        let result = transform_code("console.log(responseBody)");
        assert!(result.success);
        assert!(result.code.contains("rq.response.text()"));
    }

    #[test]
    fn response_body_in_expression() {
        let result = transform_code("pm.test(\"x\", function() { JSON.parse(responseBody) })");
        assert!(result.success);
        assert!(result.code.contains("rq.response.text()"));
    }

    #[test]
    fn response_code_dot_code() {
        let result = transform_code("pm.test(\"x\", function() { responseCode.code })");
        assert!(result.success);
        assert!(result.code.contains("rq.response.code"));
    }

    // ── Slice A: scope-aware bare legacy identifier rewrites (RQ-3463) ──────────

    // Positive: bare value globals (whole-identifier swap)
    #[test]
    fn bare_response_headers() {
        let result = transform_code("console.log(responseHeaders)");
        assert!(result.success);
        assert_eq!(result.code, "console.log(rq.response.headers)");
    }

    #[test]
    fn bare_response_cookies() {
        let result = transform_code("console.log(responseCookies)");
        assert!(result.success);
        assert_eq!(result.code, "console.log(rq.cookies)");
    }

    #[test]
    fn bare_response_time() {
        let result = transform_code("console.log(responseTime)");
        assert!(result.success);
        assert_eq!(result.code, "console.log(rq.response.responseTime)");
    }

    #[test]
    fn bare_iteration() {
        let result = transform_code("console.log(iteration)");
        assert!(result.success);
        assert_eq!(result.code, "console.log(rq.info.iteration)");
    }

    #[test]
    fn bare_request() {
        let result = transform_code("console.log(request)");
        assert!(result.success);
        assert_eq!(result.code, "console.log(rq.request)");
    }

    // Positive: bare global call roots (root-prefix swap)
    #[test]
    fn bare_globals_get() {
        let result = transform_code("globals.get(\"k\")");
        assert!(result.success);
        assert_eq!(result.code, "rq.globals.get(\"k\")");
    }

    #[test]
    fn bare_globals_set() {
        let result = transform_code("globals.set(\"k\", \"v\")");
        assert!(result.success);
        assert_eq!(result.code, "rq.globals.set(\"k\", \"v\")");
    }

    #[test]
    fn bare_environment_get() {
        let result = transform_code("environment.get(\"k\")");
        assert!(result.success);
        assert_eq!(result.code, "rq.environment.get(\"k\")");
    }

    #[test]
    fn bare_environment_set() {
        let result = transform_code("environment.set(\"k\", \"v\")");
        assert!(result.success);
        assert_eq!(result.code, "rq.environment.set(\"k\", \"v\")");
    }

    // Positive: bare global member reads (non-call) — access-semantics translation (RQ-3463).
    // A bare property read is a VARIABLE READ in legacy Postman, so it must become `.get('name')`,
    // NOT a root-prefix swap (which would yield `rq.globals.foo` === undefined).
    #[test]
    fn bare_globals_member_read() {
        let result = transform_code("const x = globals.foo;");
        assert!(result.success);
        assert_eq!(result.code, "const x = rq.globals.get('foo');");
    }

    // The dominant real AirCanada pattern: eval(globals.checkErrorWarning).
    #[test]
    fn bare_globals_read_inside_eval() {
        let result = transform_code("eval(globals.checkErrorWarning)");
        assert!(result.success);
        assert_eq!(result.code, "eval(rq.globals.get('checkErrorWarning'))");
    }

    #[test]
    fn bare_environment_read_inside_eval() {
        let result = transform_code("eval(environment.storeTravelerIds)");
        assert!(result.success);
        assert_eq!(result.code, "eval(rq.environment.get('storeTravelerIds'))");
    }

    // Computed string-literal key read → .get('name').
    #[test]
    fn bare_globals_computed_string_read() {
        let result = transform_code("console.log(globals['checkErrorWarning'])");
        assert!(result.success);
        assert_eq!(
            result.code,
            "console.log(rq.globals.get('checkErrorWarning'))"
        );
    }

    // Method CALLS stay pass-through (root-prefix swap only).
    #[test]
    fn bare_globals_get_method_call_passthrough() {
        let result = transform_code("globals.get('k')");
        assert!(result.success);
        assert_eq!(result.code, "rq.globals.get('k')");
    }

    #[test]
    fn bare_globals_set_method_call_passthrough() {
        let result = transform_code("globals.set('k','v')");
        assert!(result.success);
        assert_eq!(result.code, "rq.globals.set('k','v')");
    }

    #[test]
    fn bare_globals_has_method_call_passthrough() {
        let result = transform_code("globals.has('k')");
        assert!(result.success);
        assert_eq!(result.code, "rq.globals.has('k')");
    }

    // A bare method REFERENCE (no call) also stays a root-prefix swap, not a .get().
    #[test]
    fn bare_globals_method_reference_not_translated_to_get() {
        let result = transform_code("const fn = globals.get;");
        assert!(result.success);
        assert_eq!(result.code, "const fn = rq.globals.get;");
    }

    // Property WRITE → rq.globals.set('name', expr).
    #[test]
    fn bare_globals_member_assignment() {
        let result = transform_code("globals.checkErrorWarning = fn;");
        assert!(result.success);
        assert_eq!(result.code, "rq.globals.set('checkErrorWarning', fn);");
    }

    #[test]
    fn bare_environment_member_assignment() {
        let result = transform_code("environment.storeTravelerIds = ids;");
        assert!(result.success);
        assert_eq!(result.code, "rq.environment.set('storeTravelerIds', ids);");
    }

    // Assignment RHS still gets inner legacy rewrites (bracketing technique).
    #[test]
    fn bare_globals_assignment_rhs_inner_rewrite_survives() {
        let result = transform_code("globals.x = environment.y;");
        assert!(result.success);
        assert_eq!(
            result.code,
            "rq.globals.set('x', rq.environment.get('y'));"
        );
    }

    // GUARD (RQ-3463): computed NON-literal key read (Identifier) — LEFT UNREWRITTEN.
    // A root-prefix swap would yield `rq.globals[k]` (undefined). Safe no-op → Slice C shim.
    #[test]
    fn bare_globals_computed_dynamic_key_left_unrewritten() {
        let result = transform_code("const k = 'x'; console.log(globals[k]);");
        assert!(result.success);
        assert_eq!(result.code, "const k = 'x'; console.log(globals[k]);");
    }

    // GUARD (RQ-3463): template-literal computed key — LEFT UNREWRITTEN (not a plain string lit).
    #[test]
    fn bare_globals_template_literal_key_left_unrewritten() {
        let result = transform_code("console.log(globals[`a`]);");
        assert!(result.success);
        assert_eq!(result.code, "console.log(globals[`a`]);");
    }

    // GUARD (RQ-3463): deep member chain read — LEFT UNREWRITTEN.
    // `rq.globals.a.b` would be undefined; bare `globals.a.b` falls through to the Slice C shim.
    #[test]
    fn bare_globals_multi_hop_left_unrewritten() {
        let result = transform_code("console.log(globals.a.b);");
        assert!(result.success);
        assert_eq!(result.code, "console.log(globals.a.b);");
    }

    // GUARD (RQ-3463): deep three-hop chain — LEFT UNREWRITTEN.
    #[test]
    fn bare_globals_three_hop_left_unrewritten() {
        let result = transform_code("globals.a.b.c;");
        assert!(result.success);
        assert_eq!(result.code, "globals.a.b.c;");
    }

    // GUARD (RQ-3463): but a bare SINGLE-HOP read alongside still rewrites — no over-guarding.
    #[test]
    fn bare_globals_single_hop_still_rewrites_when_deep_chain_guarded() {
        let result = transform_code("globals.a; globals.x.y;");
        assert!(result.success);
        assert_eq!(result.code, "rq.globals.get('a'); globals.x.y;");
    }

    // GUARD (RQ-3463): COMPOUND assignment on a legacy-global member — LEFT UNREWRITTEN.
    // `rq.globals.get('n') += 1` would be invalid JS (assignment to a call expression).
    #[test]
    fn bare_globals_compound_assign_left_unrewritten() {
        let result = transform_code("globals.n += 1;");
        assert!(result.success);
        assert_eq!(result.code, "globals.n += 1;");
    }

    #[test]
    fn bare_globals_compound_assign_variants_left_unrewritten() {
        for op in ["+=", "-=", "*=", "/=", "%=", "**=", "<<=", ">>=", ">>>=", "&=", "|=", "^="] {
            let src = format!("globals.n {op} 1;");
            let result = transform_code(&src);
            assert!(result.success, "op {op} should succeed");
            assert_eq!(result.code, src, "op {op} must be left unrewritten");
        }
    }

    #[test]
    fn bare_globals_logical_assign_left_unrewritten() {
        for op in ["&&=", "||=", "??="] {
            let src = format!("globals.n {op} 1;");
            let result = transform_code(&src);
            assert!(result.success, "op {op} should succeed");
            assert_eq!(result.code, src, "op {op} must be left unrewritten");
        }
    }

    #[test]
    fn bare_environment_compound_assign_left_unrewritten() {
        let result = transform_code("environment.count += 5;");
        assert!(result.success);
        assert_eq!(result.code, "environment.count += 5;");
    }

    // GUARD (RQ-3463): plain `=` assignment MUST still rewrite (no over-guarding the compound path).
    #[test]
    fn bare_globals_plain_assign_still_rewrites() {
        let result = transform_code("globals.n = 1;");
        assert!(result.success);
        assert_eq!(result.code, "rq.globals.set('n', 1);");
    }

    // Scope-gating still works for the new read path.
    #[test]
    fn shadowed_globals_member_read_not_rewritten() {
        let result = transform_code("const globals = {}; globals.foo;");
        assert!(result.success);
        assert_eq!(result.code, "const globals = {}; globals.foo;");
    }

    // Positive: responseCode.name / .detail
    #[test]
    fn response_code_dot_name() {
        let result = transform_code("console.log(responseCode.name)");
        assert!(result.success);
        assert_eq!(result.code, "console.log(rq.response.name)");
    }

    #[test]
    fn response_code_dot_detail() {
        let result = transform_code("console.log(responseCode.detail)");
        assert!(result.success);
        assert_eq!(result.code, "console.log(rq.response.detail)");
    }

    // Positive: data.X → rq.iterationData.get('X')
    #[test]
    fn bare_data_static_property() {
        let result = transform_code("console.log(data.username)");
        assert!(result.success);
        assert_eq!(result.code, "console.log(rq.iterationData.get('username'))");
    }

    // Positive: tests["x"] = expr → rq.test("x", () => rq.expect(expr).to.be.ok)
    #[test]
    fn tests_assignment_simple() {
        let result = transform_code("tests[\"ok\"] = code === 200");
        assert!(result.success);
        assert_eq!(
            result.code,
            "rq.test(\"ok\", () => rq.expect(code === 200).to.be.ok)"
        );
    }

    #[test]
    fn tests_assignment_single_quoted_label_preserved() {
        let result = transform_code("tests['status is ok'] = true");
        assert!(result.success);
        assert_eq!(
            result.code,
            "rq.test('status is ok', () => rq.expect(true).to.be.ok)"
        );
    }

    // tests[...] composition: an inner legacy identifier in the RHS must ALSO be rewritten.
    // This is the dedup-overlap trap (Step 8): if the rewrite were one wide whole-assignment
    // span, the inner rewrite would be silently discarded.
    #[test]
    fn tests_assignment_rhs_inner_rewrite_survives() {
        let result = transform_code("tests[\"code ok\"] = responseCode.code === 200");
        assert!(result.success);
        assert_eq!(
            result.code,
            "rq.test(\"code ok\", () => rq.expect(rq.response.code === 200).to.be.ok)"
        );
    }

    #[test]
    fn tests_assignment_rhs_inner_pm_rewrite_survives() {
        let result = transform_code("tests[\"ok\"] = pm.response.code === 200");
        assert!(result.success);
        assert_eq!(
            result.code,
            "rq.test(\"ok\", () => rq.expect(rq.response.code === 200).to.be.ok)"
        );
    }

    // AirCanada corpus: the customer bug pattern + the data.X rule must coexist correctly.
    // eval(globals.checkErrorWarning) must become a real read; data.seatmaps must stay
    // an iterationData read (data.X rule), not be affected by the globals change.
    #[test]
    fn aircanada_corpus_globals_read_and_data_rule() {
        let result = transform_code(
            "eval(globals.checkErrorWarning); checkErrorWarning(response);\nvar d = data.seatmaps;",
        );
        assert!(result.success);
        assert!(
            result
                .code
                .contains("eval(rq.globals.get('checkErrorWarning'))"),
            "got:\n{}",
            result.code
        );
        // The downstream call to the now-defined helper is untouched.
        assert!(result.code.contains("checkErrorWarning(response);"));
        // data.X rule unchanged.
        assert!(
            result.code.contains("rq.iterationData.get('seatmaps')"),
            "got:\n{}",
            result.code
        );
    }

    // ── Negative / shadowing — the slice's defining requirement ───────────────

    #[test]
    fn shadowed_globals_const_not_rewritten() {
        let result = transform_code("const globals = {}; globals.get(\"k\");");
        assert!(result.success);
        assert_eq!(result.code, "const globals = {}; globals.get(\"k\");");
    }

    #[test]
    fn shadowed_environment_let_not_rewritten() {
        let result = transform_code("let environment = x; environment.set(1);");
        assert!(result.success);
        assert_eq!(result.code, "let environment = x; environment.set(1);");
    }

    #[test]
    fn shadowed_request_param_not_rewritten() {
        let result = transform_code("function f(request) { return request.url; }");
        assert!(result.success);
        assert_eq!(result.code, "function f(request) { return request.url; }");
    }

    #[test]
    fn shadowed_data_const_not_rewritten() {
        let result = transform_code("const data = []; data.username;");
        assert!(result.success);
        assert_eq!(result.code, "const data = []; data.username;");
    }

    #[test]
    fn shadowed_iteration_let_not_rewritten() {
        let result = transform_code("let iteration = 0; iteration;");
        assert!(result.success);
        assert_eq!(result.code, "let iteration = 0; iteration;");
    }

    #[test]
    fn shadowed_response_headers_var_not_rewritten() {
        let result = transform_code("var responseHeaders = {}; console.log(responseHeaders);");
        assert!(result.success);
        assert_eq!(
            result.code,
            "var responseHeaders = {}; console.log(responseHeaders);"
        );
    }

    #[test]
    fn shadowed_tests_param_not_rewritten() {
        let result = transform_code("function run(tests) { tests[\"x\"] = 1; }");
        assert!(result.success);
        assert_eq!(result.code, "function run(tests) { tests[\"x\"] = 1; }");
    }

    #[test]
    fn shadowed_response_code_const_not_rewritten() {
        let result = transform_code("const responseCode = {}; responseCode.name;");
        assert!(result.success);
        assert_eq!(result.code, "const responseCode = {}; responseCode.name;");
    }

    #[test]
    fn shadowed_environment_catch_binding_not_rewritten() {
        // catch (environment) binds `environment` for the handler body.
        let result =
            transform_code("try { foo(); } catch (environment) { environment.set(1); }");
        assert!(result.success);
        assert_eq!(
            result.code,
            "try { foo(); } catch (environment) { environment.set(1); }"
        );
    }

    #[test]
    fn shadowed_globals_arrow_param_not_rewritten() {
        let result = transform_code("const f = (globals) => globals.get(\"k\");");
        assert!(result.success);
        assert_eq!(result.code, "const f = (globals) => globals.get(\"k\");");
    }

    // Bare global is still rewritten when the shadow is in a sibling (non-enclosing) scope.
    #[test]
    fn shadow_in_sibling_scope_does_not_suppress_outer() {
        let result = transform_code("function f(globals) { return globals; } environment.get(\"k\");");
        assert!(result.success);
        assert!(result.code.contains("function f(globals) { return globals; }"));
        assert!(result.code.contains("rq.environment.get(\"k\")"));
    }

    // ── Idempotence and edge cases ────────────────────────────────────────────

    #[test]
    fn bare_globals_idempotent() {
        let once = transform_code("globals.get(\"k\")");
        let twice = transform_code(&once.code);
        assert_eq!(twice.code, "rq.globals.get(\"k\")");
    }

    #[test]
    fn tests_assignment_idempotent() {
        let once = transform_code("tests[\"ok\"] = code === 200");
        let twice = transform_code(&once.code);
        assert_eq!(once.code, twice.code);
    }

    #[test]
    fn bare_identifier_in_string_not_rewritten() {
        let result = transform_code("const s = \"globals.get is legacy\";");
        assert!(result.success);
        assert_eq!(result.code, "const s = \"globals.get is legacy\";");
    }

    #[test]
    fn data_computed_access_not_rewritten() {
        // Computed access `data[k]` is out of scope for v1 — left untouched.
        let result = transform_code("const k = \"u\"; console.log(data[k]);");
        assert!(result.success);
        assert_eq!(result.code, "const k = \"u\"; console.log(data[k]);");
    }

    #[test]
    fn tests_static_member_assignment_not_rewritten() {
        // `tests.foo = x` (static member) is out of scope — left untouched.
        let result = transform_code("tests.foo = 1;");
        assert!(result.success);
        assert_eq!(result.code, "tests.foo = 1;");
    }

    #[test]
    fn tests_non_literal_key_not_rewritten() {
        // Non-string-literal computed key is out of scope — left untouched.
        let result = transform_code("const k = \"x\"; tests[k] = 1;");
        assert!(result.success);
        assert_eq!(result.code, "const k = \"x\"; tests[k] = 1;");
    }

    #[test]
    fn data_multi_hop_not_rewritten() {
        // Deep chain `data.a.b` is out of scope for v1 — left untouched.
        let result = transform_code("console.log(data.a.b);");
        assert!(result.success);
        assert_eq!(result.code, "console.log(data.a.b);");
    }

    // Real-world regression: user script with helpers declared via `function foo() {}`.
    // The full script — test block using pm.* at top level, helper functions using pm.*
    // inside their bodies — must transform every pm.* reference, not just the top-level ones.
    #[test]
    fn real_world_paypal_script_fully_transformed() {
        let source = r#"var successHttpStatuses = [200, 201, 202, 204];
var message = pm.response.code +", Paypal-Debug-Id="+getPayPalDebugId();
if(!isSuccessful()) {
    console.error("Unexpected HTTP Status Code: ", message, pm.response.text());
    message = message + ", "+pm.response.text();
}
pm.test("HTTP Status Code must be one of "+successHttpStatuses+", actual is "+message, function () {
    pm.expect(pm.response.code).to.be.oneOf(successHttpStatuses);
});
function getPayPalDebugId() {
    if(pm && pm.response && pm.response.headers) {
        return pm.response.headers.get('Paypal-Debug-Id');
    }
}
function isSuccessful() {
    return successHttpStatuses.includes(pm.response.code);
}
"#;
        let result = transform_code(source);
        assert!(result.success);
        // Only `pm` as a standalone identifier (truthiness check) is allowed to remain —
        // every member/call starting with `pm.` must have been rewritten to `rq.`.
        assert!(
            !result.code.contains("pm."),
            "pm.* should not remain after transform, got:\n{}",
            result.code
        );
        assert!(result.code.contains("rq.response.headers.get"));
        assert!(result.code.contains("rq.response.code"));
        assert!(result.code.contains("rq.test"));
        assert!(result.code.contains("rq.expect"));
    }

    // Paypal-style script: pm.* calls inside top-level `function foo() {}` declarations
    // must be transformed. The walker was missing Statement::FunctionDeclaration, so
    // function bodies were silently skipped.
    #[test]
    fn pm_inside_function_declaration_is_transformed() {
        let source = r#"function getDebugId() {
    if (pm && pm.response && pm.response.headers) {
        return pm.response.headers.get('Debug-Id');
    }
}
function isSuccessful() {
    return [200, 201].includes(pm.response.code);
}
"#;
        let result = transform_code(source);
        assert!(result.success);
        assert!(
            !result.code.contains("pm."),
            "pm.* should not remain after transform, got:\n{}",
            result.code
        );
        assert!(result.code.contains("rq.response.headers.get"));
        assert!(result.code.contains("rq.response.code"));
    }

    // Postman-runtime scripts: top-level `return` is valid because Postman wraps
    // scripts in a function. Ensure the wrapper-retry path transforms pm.* calls.
    #[test]
    fn top_level_return_transforms_pm_calls() {
        let source = r#"if (pm.variables.get('x')) {
    return;
}
pm.environment.set('k', 'v');
"#;
        let result = transform_code(source);
        assert!(result.success, "expected wrapper retry to succeed");
        assert!(result.code.contains("rq.variables.get"));
        assert!(result.code.contains("rq.environment.set"));
        assert!(!result.code.contains("pm."));
        // `return` must be preserved verbatim.
        assert!(result.code.contains("return;"));
    }

    #[test]
    fn genuine_syntax_error_still_fails() {
        let result = transform_code("const { = [");
        assert!(!result.success);
        assert!(result.summary.errors > 0);
    }

    // Edge cases
    #[test]
    fn pm_in_string_not_transformed() {
        let result = transform_code("const s = \"pm.test is cool\"");
        assert!(result.success);
        // No pm. member expression in AST — string literal is not a member expression
        assert_eq!(result.code, "const s = \"pm.test is cool\"");
    }

    #[test]
    fn pm_in_template_literal_expression_transformed() {
        let result = transform_code("`${pm.test}`");
        assert!(result.success);
        assert!(result.code.contains("rq.test"));
    }

    #[test]
    fn chained_calls_across_lines() {
        let result = transform_code("pm.test(\"a\", () => {\n  pm.expect(1)\n})");
        assert!(result.success);
        assert!(result.code.contains("rq.test"));
        assert!(result.code.contains("rq.expect"));
    }

    #[test]
    fn object_literal_with_pm_values() {
        let result = transform_code("const obj = { value: pm.environment.get(\"k\") }");
        assert!(result.success);
        assert!(result.code.contains("rq.environment.get"));
    }

    #[test]
    fn callback_body_transformed() {
        let result = transform_code("pm.test(\"x\", function() { pm.response.json() })");
        assert!(result.success);
        assert!(!result.code.contains("pm."));
    }

    #[test]
    fn if_block_body() {
        let result = transform_code("if (true) { pm.environment.set(\"k\", \"v\") }");
        assert!(result.success);
        assert!(result.code.contains("rq.environment.set"));
    }

    #[test]
    fn try_catch_body() {
        let result = transform_code("try { pm.test(\"x\", function(){}) } catch(e) {}");
        assert!(result.success);
        assert!(result.code.contains("rq.test"));
    }

    #[test]
    fn summary_counts_correct() {
        let result = transform_code("pm.test(\"a\", function() { pm.expect(1) })");
        assert!(result.success);
        assert!(result.summary.replacements >= 2);
        assert_eq!(result.summary.errors, 0);
    }

    // Full Postman script
    #[test]
    fn full_postman_script() {
        let source = r#"
pm.test("Status code is 200", function () {
    pm.response.to.have.status(200);
});

pm.test("Response has data", function () {
    const jsonData = pm.response.json();
    pm.expect(jsonData).to.have.property("id");
    pm.expect(jsonData.name).to.be.a("string");
});

const token = pm.environment.get("auth_token");
pm.sendRequest({
    url: "https://api.example.com/refresh",
    method: "POST",
    header: { "Authorization": "Bearer " + token }
}, function (err, response) {
    pm.environment.set("auth_token", response.json().token);
});

pm.execution.setNextRequest("Next Test");
"#;
        let result = transform_code(source);
        assert!(result.success);
        assert!(!result.code.contains("pm."));
        assert!(result.code.contains("rq.test"));
        assert!(result.code.contains("rq.response"));
        assert!(result.code.contains("rq.expect"));
        assert!(result.code.contains("rq.environment"));
        assert!(result.code.contains("rq.sendRequest"));
        assert!(result.code.contains("rq.execution.setNextRequest"));
        assert!(result.summary.replacements > 0);
    }

    // ── Finding 1: for-loop loop variables bound into scope ───────────────────
    // A loop variable named after a legacy global must NOT be rewritten inside the loop.

    #[test]
    fn for_statement_loop_var_data_not_rewritten() {
        let result = transform_code("for (let data = 0; data < 10; data++) { console.log(data); }");
        assert!(result.success);
        assert_eq!(
            result.code,
            "for (let data = 0; data < 10; data++) { console.log(data); }"
        );
    }

    #[test]
    fn for_in_loop_var_globals_not_rewritten() {
        let result = transform_code("for (let globals in obj) { console.log(globals); }");
        assert!(result.success);
        assert_eq!(
            result.code,
            "for (let globals in obj) { console.log(globals); }"
        );
    }

    #[test]
    fn for_of_loop_var_request_not_rewritten() {
        let result = transform_code("for (const request of arr) { console.log(request.url); }");
        assert!(result.success);
        assert_eq!(
            result.code,
            "for (const request of arr) { console.log(request.url); }"
        );
    }

    #[test]
    fn for_of_loop_var_environment_not_rewritten() {
        let result = transform_code("for (let environment of xs) { environment.foo; }");
        assert!(result.success);
        assert_eq!(
            result.code,
            "for (let environment of xs) { environment.foo; }"
        );
    }

    // Regression: a bare legacy global NOT shadowed by the loop variable still rewrites.
    #[test]
    fn for_statement_unshadowed_global_inside_loop_still_rewritten() {
        let result = transform_code("for (let i=0;i<3;i++){ globals.set('k', i); }");
        assert!(result.success);
        assert!(
            result.code.contains("rq.globals.set('k', i)"),
            "got:\n{}",
            result.code
        );
        // The loop variable `i` is untouched.
        assert!(result.code.contains("for (let i=0;i<3;i++)"));
    }

    // Regression: a bare `data.X` OUTSIDE any loop still maps to the iterationData read.
    #[test]
    fn bare_data_outside_loop_still_rewritten() {
        let result = transform_code("data.username");
        assert!(result.success);
        assert_eq!(result.code, "rq.iterationData.get('username')");
    }

    // ── Finding 2: tests[...] compound assignment guard ───────────────────────
    // Only a plain `=` defines a test; compound/logical assignment is left unchanged.

    #[test]
    fn tests_compound_plus_assign_not_rewritten() {
        let result = transform_code("tests[\"x\"] += 1");
        assert!(result.success);
        assert_eq!(result.code, "tests[\"x\"] += 1");
    }

    #[test]
    fn tests_compound_minus_assign_not_rewritten() {
        let result = transform_code("tests[\"x\"] -= 1");
        assert!(result.success);
        assert_eq!(result.code, "tests[\"x\"] -= 1");
    }

    #[test]
    fn tests_logical_or_assign_not_rewritten() {
        let result = transform_code("tests[\"x\"] ||= 1");
        assert!(result.success);
        assert_eq!(result.code, "tests[\"x\"] ||= 1");
    }

    #[test]
    fn tests_logical_and_assign_not_rewritten() {
        let result = transform_code("tests[\"x\"] &&= 1");
        assert!(result.success);
        assert_eq!(result.code, "tests[\"x\"] &&= 1");
    }

    #[test]
    fn tests_nullish_assign_not_rewritten() {
        let result = transform_code("tests[\"x\"] ??= 1");
        assert!(result.success);
        assert_eq!(result.code, "tests[\"x\"] ??= 1");
    }

    // Regression: a plain `=` assignment still rewrites.
    #[test]
    fn tests_plain_assign_still_rewritten() {
        let result = transform_code("tests[\"x\"] = code === 200");
        assert!(result.success);
        assert_eq!(
            result.code,
            "rq.test(\"x\", () => rq.expect(code === 200).to.be.ok)"
        );
    }
}
