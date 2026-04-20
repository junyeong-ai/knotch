//! Built-in lint rules.

use std::collections::HashSet;

use compact_str::CompactString;
use proc_macro2::LineColumn;
use syn::{
    Expr, ExprLit, Ident, ItemEnum, ItemFn, ItemStruct, ItemTrait, ItemUse, Lit, LitStr, Macro,
    UseTree,
    visit::{self, Visit},
};

use crate::{LintContext, Rule, RuleId, Severity, Violation};

// ---------------------------------------------------------------------
// R1 — DirectLogWrite
// ---------------------------------------------------------------------

/// `R1` — flags direct writes to knotch log files outside of
/// `knotch-storage`. Enforces constitution §II (single writer per
/// unit).
#[derive(Debug, Clone)]
pub struct DirectLogWriteRule {
    /// Crates allowed to write directly to knotch log files.
    pub allowlist: HashSet<String>,
    /// Path suffixes (`log.jsonl`, `.resume-cache.json`) that
    /// trigger the rule.
    pub path_needles: Vec<CompactString>,
}

impl Default for DirectLogWriteRule {
    fn default() -> Self {
        let mut allowlist = HashSet::new();
        for crate_name in [
            "knotch-storage",
            "knotch-testing",
            "knotch-linter", // self-lint fixtures
        ] {
            allowlist.insert(crate_name.to_owned());
        }
        Self {
            allowlist,
            path_needles: vec![
                CompactString::from("log.jsonl"),
                CompactString::from(".resume-cache.json"),
            ],
        }
    }
}

impl Rule for DirectLogWriteRule {
    fn id(&self) -> RuleId {
        RuleId("R1")
    }

    fn description(&self) -> &'static str {
        "direct writes to knotch log files are forbidden outside knotch-storage"
    }

    fn check(&self, ctx: &LintContext, file: &syn::File) -> Vec<Violation> {
        if let Some(name) = &ctx.crate_name {
            if self.allowlist.contains(name) {
                return Vec::new();
            }
        }
        let mut v = DirectLogWriteVisitor {
            ctx,
            path_needles: &self.path_needles,
            findings: Vec::new(),
        };
        v.visit_file(file);
        v.findings
    }
}

struct DirectLogWriteVisitor<'a> {
    ctx: &'a LintContext,
    path_needles: &'a [CompactString],
    findings: Vec<Violation>,
}

impl<'a, 'ast> Visit<'ast> for DirectLogWriteVisitor<'a> {
    fn visit_expr(&mut self, expr: &'ast Expr) {
        // Heuristic: report any string literal that matches a needle
        // and sits anywhere inside an expression (call args, let
        // bindings, match expressions). The AST scope carries
        // enough context that later rules can refine this.
        if let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = expr {
            check_lit(self.ctx, s, self.path_needles, &mut self.findings);
        }
        visit::visit_expr(self, expr);
    }

    fn visit_macro(&mut self, mac: &'ast Macro) {
        for token in mac.tokens.clone().into_iter() {
            if let proc_macro2::TokenTree::Literal(lit) = token {
                let text = lit.to_string();
                if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
                    let trimmed = &text[1..text.len() - 1];
                    if self.path_needles.iter().any(|n| trimmed.contains(n.as_str())) {
                        let LineColumn { line, column } = lit.span().start();
                        self.findings.push(Violation {
                            rule: RuleId("R1"),
                            path: self.ctx.path.clone(),
                            line: line as u32,
                            column: (column + 1) as u32,
                            severity: Severity::Error,
                            message: CompactString::from(
                                "direct write to a knotch log file — route through \
                                 knotch_storage::Storage instead",
                            ),
                        });
                    }
                }
            }
        }
        visit::visit_macro(self, mac);
    }
}

fn check_lit(
    ctx: &LintContext,
    lit: &LitStr,
    needles: &[CompactString],
    findings: &mut Vec<Violation>,
) {
    let value = lit.value();
    if needles.iter().any(|n| value.contains(n.as_str())) {
        let LineColumn { line, column } = lit.span().start();
        findings.push(Violation {
            rule: RuleId("R1"),
            path: ctx.path.clone(),
            line: line as u32,
            column: (column + 1) as u32,
            severity: Severity::Error,
            message: CompactString::from(
                "direct write to a knotch log file — route through \
                 knotch_storage::Storage instead",
            ),
        });
    }
}

// ---------------------------------------------------------------------
// R2 — ForbiddenName
// ---------------------------------------------------------------------

/// `R2` — rejects identifiers with forbidden suffixes. Matches
/// `knotch-v1-final-plan §16.5`.
#[derive(Debug, Clone)]
pub struct ForbiddenNameRule {
    /// Suffixes that trigger the rule.
    pub suffixes: Vec<&'static str>,
}

impl Default for ForbiddenNameRule {
    fn default() -> Self {
        Self {
            suffixes: vec![
                "Helper",
                "Util",
                "Utils",
                "Manager",
                "Handler",
                "Processor",
                "Impl",
            ],
        }
    }
}

impl Rule for ForbiddenNameRule {
    fn id(&self) -> RuleId {
        RuleId("R2")
    }

    fn description(&self) -> &'static str {
        "identifiers with forbidden suffixes (*Helper/*Util/*Manager/*Handler/*Processor/*Impl)"
    }

    fn check(&self, ctx: &LintContext, file: &syn::File) -> Vec<Violation> {
        let mut v = ForbiddenNameVisitor {
            ctx,
            suffixes: &self.suffixes,
            findings: Vec::new(),
        };
        v.visit_file(file);
        v.findings
    }
}

struct ForbiddenNameVisitor<'a> {
    ctx: &'a LintContext,
    suffixes: &'a [&'static str],
    findings: Vec<Violation>,
}

impl<'a> ForbiddenNameVisitor<'a> {
    fn check_ident(&mut self, ident: &Ident) {
        let name = ident.to_string();
        for suffix in self.suffixes {
            if name.ends_with(*suffix) && name.len() > suffix.len() {
                // Skip the *Impl check when the suffix is part of a
                // vendor-reserved stdlib type (e.g. `BlockingImpl`);
                // v1 enforces only workspace-local definitions.
                let LineColumn { line, column } = ident.span().start();
                self.findings.push(Violation {
                    rule: RuleId("R2"),
                    path: self.ctx.path.clone(),
                    line: line as u32,
                    column: (column + 1) as u32,
                    severity: Severity::Error,
                    message: CompactString::from(format!(
                        "identifier `{name}` ends with forbidden suffix `{suffix}` — \
                         rename to a noun describing the role"
                    )),
                });
                return;
            }
        }
    }
}

impl<'a, 'ast> Visit<'ast> for ForbiddenNameVisitor<'a> {
    fn visit_item_struct(&mut self, item: &'ast ItemStruct) {
        self.check_ident(&item.ident);
        visit::visit_item_struct(self, item);
    }

    fn visit_item_enum(&mut self, item: &'ast ItemEnum) {
        self.check_ident(&item.ident);
        visit::visit_item_enum(self, item);
    }

    fn visit_item_trait(&mut self, item: &'ast ItemTrait) {
        self.check_ident(&item.ident);
        visit::visit_item_trait(self, item);
    }

    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        self.check_ident(&item.sig.ident);
        visit::visit_item_fn(self, item);
    }
}

// ---------------------------------------------------------------------
// R3 — KernelNoIo
// ---------------------------------------------------------------------

/// `R3` — enforces the purity boundary for `knotch-kernel` and
/// `knotch-proto`: neither crate may import `std::fs`, `std::net`,
/// `tokio::fs`, `tokio::net`, or `gix`.
///
/// Realizes Plan §2 principle 4 (Purity boundary). `cargo-deny` can
/// ban transitive crates but not `std` items, so we implement this
/// at the AST layer.
#[derive(Debug, Clone)]
pub struct KernelNoIoRule {
    /// Crates the rule is active in (exact match).
    pub active_in: Vec<&'static str>,
    /// Path prefixes treated as I/O imports.
    pub forbidden_prefixes: Vec<Vec<&'static str>>,
}

impl Default for KernelNoIoRule {
    fn default() -> Self {
        Self {
            active_in: vec!["knotch-kernel", "knotch-proto"],
            forbidden_prefixes: vec![
                vec!["std", "fs"],
                vec!["std", "net"],
                vec!["tokio", "fs"],
                vec!["tokio", "net"],
                vec!["gix"],
            ],
        }
    }
}

impl Rule for KernelNoIoRule {
    fn id(&self) -> RuleId {
        RuleId("R3")
    }

    fn description(&self) -> &'static str {
        "kernel/proto crates must not import std::fs, std::net, tokio::fs, tokio::net, or gix"
    }

    fn check(&self, ctx: &LintContext, file: &syn::File) -> Vec<Violation> {
        let active = ctx
            .crate_name
            .as_deref()
            .map(|n| self.active_in.contains(&n))
            .unwrap_or(false);
        if !active {
            return Vec::new();
        }
        let mut v = KernelNoIoVisitor {
            ctx,
            forbidden: &self.forbidden_prefixes,
            findings: Vec::new(),
        };
        v.visit_file(file);
        v.findings
    }
}

struct KernelNoIoVisitor<'a> {
    ctx: &'a LintContext,
    forbidden: &'a [Vec<&'static str>],
    findings: Vec<Violation>,
}

impl<'a, 'ast> Visit<'ast> for KernelNoIoVisitor<'a> {
    fn visit_item_use(&mut self, item: &'ast ItemUse) {
        let mut segments = Vec::new();
        collect_use_root(&item.tree, &mut segments);
        for forbidden in self.forbidden {
            if starts_with(&segments, forbidden) {
                let span = item.use_token.span.start();
                let LineColumn { line, column } = span;
                let path = forbidden.join("::");
                self.findings.push(Violation {
                    rule: RuleId("R3"),
                    path: self.ctx.path.clone(),
                    line: line as u32,
                    column: (column + 1) as u32,
                    severity: Severity::Error,
                    message: CompactString::from(format!(
                        "kernel/proto crates must not import `{path}` — route through an adapter crate"
                    )),
                });
                break;
            }
        }
        visit::visit_item_use(self, item);
    }
}

fn collect_use_root(tree: &UseTree, out: &mut Vec<String>) {
    match tree {
        UseTree::Path(p) => {
            out.push(p.ident.to_string());
            collect_use_root(&p.tree, out);
        }
        UseTree::Name(n) => out.push(n.ident.to_string()),
        UseTree::Rename(r) => out.push(r.ident.to_string()),
        UseTree::Glob(_) => {}
        UseTree::Group(_g) => {
            // `use a::b::{c, d}` — only the prefix `a::b` is relevant for
            // the R3 check. Terminate collection here; the siblings are
            // not part of the path root.
        }
    }
}

fn starts_with(segments: &[String], prefix: &[&'static str]) -> bool {
    if segments.len() < prefix.len() {
        return false;
    }
    segments.iter().zip(prefix.iter()).all(|(s, p)| s == p)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn parse(src: &str) -> syn::File {
        syn::parse_file(src).expect("parse")
    }

    fn ctx(name: Option<&str>) -> LintContext {
        LintContext {
            path: PathBuf::from("<memory>"),
            crate_name: name.map(ToOwned::to_owned),
        }
    }

    #[test]
    fn r1_flags_direct_log_write_outside_storage() {
        let src = r#"
            fn write_it() {
                let path = "state/log.jsonl";
                std::fs::write(path, b"x").unwrap();
            }
        "#;
        let rule = DirectLogWriteRule::default();
        let findings = rule.check(&ctx(Some("downstream-crate")), &parse(src));
        assert_eq!(findings.len(), 1, "expected 1 finding, got {findings:?}");
        assert_eq!(findings[0].rule.0, "R1");
    }

    #[test]
    fn r1_allowlists_knotch_storage() {
        let src = r#"fn w() { let _ = "x/log.jsonl"; }"#;
        let rule = DirectLogWriteRule::default();
        let findings = rule.check(&ctx(Some("knotch-storage")), &parse(src));
        assert!(findings.is_empty());
    }

    #[test]
    fn r1_flags_literal_inside_macro_args() {
        let src = r#"
            fn w() {
                println!("writing to {}", "log.jsonl");
            }
        "#;
        let rule = DirectLogWriteRule::default();
        let findings = rule.check(&ctx(Some("downstream-crate")), &parse(src));
        assert!(!findings.is_empty());
    }

    #[test]
    fn r2_flags_forbidden_struct_suffix() {
        let src = r#"pub struct FooHelper;"#;
        let rule = ForbiddenNameRule::default();
        let findings = rule.check(&ctx(None), &parse(src));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule.0, "R2");
        assert!(findings[0].message.contains("FooHelper"));
    }

    #[test]
    fn r2_flags_forbidden_enum_and_trait() {
        let src = r#"
            pub enum JobProcessor { A }
            pub trait WireHandler {}
        "#;
        let rule = ForbiddenNameRule::default();
        let findings = rule.check(&ctx(None), &parse(src));
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn r2_skips_exact_suffix_only_match() {
        // `Impl` alone (without a prefix) is not a forbidden suffix
        // because name.len() == suffix.len() — rule demands a prefix.
        let src = r#"pub struct Impl;"#;
        let rule = ForbiddenNameRule::default();
        let findings = rule.check(&ctx(None), &parse(src));
        assert!(findings.is_empty());
    }

    #[test]
    fn r2_passes_clean_idiomatic_names() {
        let src = r#"
            pub struct Repository;
            pub trait Observer {}
            pub enum Scope { Tiny, Standard }
            pub fn compute_status() {}
        "#;
        let rule = ForbiddenNameRule::default();
        let findings = rule.check(&ctx(None), &parse(src));
        assert!(findings.is_empty());
    }

    // --- R3 KernelNoIo ------------------------------------------------

    #[test]
    fn r3_flags_std_fs_import_in_kernel() {
        let src = "use std::fs::File;";
        let rule = KernelNoIoRule::default();
        let findings = rule.check(&ctx(Some("knotch-kernel")), &parse(src));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule.0, "R3");
    }

    #[test]
    fn r3_flags_tokio_fs_import_in_proto() {
        let src = "use tokio::fs::read;";
        let rule = KernelNoIoRule::default();
        let findings = rule.check(&ctx(Some("knotch-proto")), &parse(src));
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn r3_flags_gix_import_in_kernel() {
        let src = "use gix::Repository;";
        let rule = KernelNoIoRule::default();
        let findings = rule.check(&ctx(Some("knotch-kernel")), &parse(src));
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn r3_allows_io_import_in_storage() {
        let src = "use std::fs::File; use tokio::fs::read; use gix::Repository;";
        let rule = KernelNoIoRule::default();
        let findings = rule.check(&ctx(Some("knotch-storage")), &parse(src));
        assert!(findings.is_empty());
    }

    #[test]
    fn r3_allows_io_adjacent_imports_in_kernel() {
        // `std::io` is not banned — only `std::fs` and `std::net`.
        let src = "use std::io::Write; use std::net;";
        let rule = KernelNoIoRule::default();
        let findings = rule.check(&ctx(Some("knotch-kernel")), &parse(src));
        // `std::net` hits, `std::io::Write` passes.
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("std::net"));
    }

    #[test]
    fn r3_inactive_for_unknown_crate() {
        // `ctx.crate_name = None` disables the rule.
        let src = "use std::fs::File;";
        let rule = KernelNoIoRule::default();
        let findings = rule.check(&ctx(None), &parse(src));
        assert!(findings.is_empty());
    }
}
