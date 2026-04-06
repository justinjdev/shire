//! JavaScript-specific hooks.
//!
//! JavaScript shares most extraction logic with TypeScript: visibility,
//! parent resolution, signatures, parameters, return types, and post-processing
//! all work identically against the tree-sitter-javascript grammar.
//!
//! What differs:
//!  - `enclosing_ancestors`: the JS grammar has no `method_signature` or
//!    `interface_declaration` node kinds, so we drop them.
//!  - `reference_stoplist`: TS-only type keywords (`any`, `unknown`, `never`,
//!    `void`) and TS primitive type names (`string`, `number`, `boolean`) are
//!    removed since they are ordinary identifiers in JS.

use super::{LanguageHooks, ReferenceHooks};

/// Return the language hooks for JavaScript.
///
/// Delegates all extraction functions to the shared TypeScript hooks and only
/// overrides the two fields that are language-specific.
pub fn hooks() -> LanguageHooks {
    let base = super::typescript::hooks();
    LanguageHooks {
        reference_hooks: Some(ReferenceHooks {
            enclosing_ancestors: &[
                "function_declaration",
                "method_definition",
                "class_declaration",
                "function_expression",
                // arrow_function is a distinct callable kind in JS — without it
                // refs captured inside arrow bodies get no enclosing_symbol,
                // degrading caller/callee attribution.
                "arrow_function",
            ],
            reference_stoplist: &[
                "true", "false", "null", "undefined", "this", "super",
            ],
        }),
        ..base
    }
}
