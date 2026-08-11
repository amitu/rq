use std::collections::HashSet;

use oxc_ast::ast::{BindingPattern, BindingPatternKind, FormalParameters};

/// A lexical scope stack tracking identifier names bound in enclosing scopes.
///
/// Used to make legacy-Postman bare-identifier rewrites scope-aware: a bare reference
/// (e.g. `globals`, `environment`, `request`) is only rewritten to its `rq.*` equivalent
/// when it is NOT shadowed by a user-declared `var`/`let`/`const`, function parameter,
/// function name, or `catch` binding in an enclosing scope.
///
/// The model biases toward over-collection: when uncertain whether a name is bound, it is
/// treated as bound. Over-collection only suppresses a rewrite (a recoverable missed
/// opportunity); it never corrupts a user variable.
#[derive(Default)]
pub struct ScopeStack {
    frames: Vec<HashSet<String>>,
}

impl ScopeStack {
    pub fn new() -> Self {
        ScopeStack {
            frames: vec![HashSet::new()],
        }
    }

    /// Push a new (empty) lexical frame — e.g. on entering a function body or block.
    pub fn push(&mut self) {
        self.frames.push(HashSet::new());
    }

    /// Pop the topmost lexical frame — e.g. on leaving a function body or block.
    pub fn pop(&mut self) {
        // Never pop the root frame.
        if self.frames.len() > 1 {
            self.frames.pop();
        }
    }

    /// Bind a name in the current (topmost) frame.
    pub fn bind(&mut self, name: &str) {
        if let Some(top) = self.frames.last_mut() {
            top.insert(name.to_string());
        }
    }

    /// Returns true if `name` is bound in any enclosing frame.
    pub fn is_bound(&self, name: &str) -> bool {
        self.frames.iter().any(|frame| frame.contains(name))
    }

    /// Collect every identifier bound by a binding pattern (handles destructuring).
    /// Over-collection is safe: extra names only suppress rewrites.
    pub fn bind_pattern(&mut self, pattern: &BindingPattern) {
        match &pattern.kind {
            BindingPatternKind::BindingIdentifier(ident) => {
                self.bind(&ident.name);
            }
            BindingPatternKind::ObjectPattern(obj) => {
                for prop in &obj.properties {
                    self.bind_pattern(&prop.value);
                }
                if let Some(rest) = &obj.rest {
                    self.bind_pattern(&rest.argument);
                }
            }
            BindingPatternKind::ArrayPattern(arr) => {
                for elem in arr.elements.iter().flatten() {
                    self.bind_pattern(elem);
                }
                if let Some(rest) = &arr.rest {
                    self.bind_pattern(&rest.argument);
                }
            }
            BindingPatternKind::AssignmentPattern(assign) => {
                self.bind_pattern(&assign.left);
            }
        }
    }

    /// Bind every formal parameter of a function/arrow into the current frame.
    pub fn bind_params(&mut self, params: &FormalParameters) {
        for param in &params.items {
            self.bind_pattern(&param.pattern);
        }
        if let Some(rest) = &params.rest {
            self.bind_pattern(&rest.argument);
        }
    }
}
