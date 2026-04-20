//! Declarative skip policies.
//!
//! A `SkipPolicy` enumerates which `SkipKind` variants a phase accepts.
//! Phases whose skip rules are static may embed a `SkipPolicy` constant;
//! phases that compute skippability at runtime can construct one per
//! call.

use compact_str::CompactString;
use knotch_kernel::event::SkipKind;

/// Which skip reasons a phase accepts.
///
/// The `Option<Vec>` shape disambiguates three cases:
///
/// - `None` → reject every variant of this family.
/// - `Some(vec![])` → accept *any* code within this family.
/// - `Some(codes)` → accept only the listed codes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkipPolicy {
    /// Accepts `SkipKind::ScopeTooNarrow`.
    pub scope_too_narrow: bool,
    /// Amnesty policy.
    pub amnesty: Option<Vec<CompactString>>,
    /// Custom-skip policy.
    pub custom: Option<Vec<CompactString>>,
}

impl SkipPolicy {
    /// A policy that rejects every skip reason.
    #[must_use]
    pub fn reject_all() -> Self {
        Self::default()
    }

    /// A policy that accepts every variant.
    #[must_use]
    pub fn accept_all() -> Self {
        Self { scope_too_narrow: true, amnesty: Some(Vec::new()), custom: Some(Vec::new()) }
    }

    /// Accept scope-narrowing skips.
    #[must_use]
    pub fn accept_scope_too_narrow(mut self) -> Self {
        self.scope_too_narrow = true;
        self
    }

    /// Accept amnesty skips with the supplied code. Call repeatedly
    /// to whitelist more codes.
    #[must_use]
    pub fn accept_amnesty(mut self, code: impl Into<CompactString>) -> Self {
        self.amnesty.get_or_insert_with(Vec::new).push(code.into());
        self
    }

    /// Accept *any* amnesty code (wildcard).
    #[must_use]
    pub fn accept_any_amnesty(mut self) -> Self {
        self.amnesty = Some(Vec::new());
        self
    }

    /// Accept a specific custom code.
    #[must_use]
    pub fn accept_custom(mut self, code: impl Into<CompactString>) -> Self {
        self.custom.get_or_insert_with(Vec::new).push(code.into());
        self
    }

    /// Accept *any* custom code (wildcard).
    #[must_use]
    pub fn accept_any_custom(mut self) -> Self {
        self.custom = Some(Vec::new());
        self
    }

    /// Evaluate the policy against a reason.
    #[must_use]
    pub fn is_skippable(&self, reason: &SkipKind) -> bool {
        match reason {
            SkipKind::ScopeTooNarrow => self.scope_too_narrow,
            SkipKind::Amnesty { code } => family_matches(self.amnesty.as_deref(), code),
            SkipKind::Custom { code } => family_matches(self.custom.as_deref(), code),
            _ => false,
        }
    }
}

fn family_matches(family: Option<&[CompactString]>, code: &CompactString) -> bool {
    match family {
        None => false,
        Some([]) => true,
        Some(list) => list.iter().any(|c| c == code),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_all_refuses_every_variant() {
        let policy = SkipPolicy::reject_all();
        assert!(!policy.is_skippable(&SkipKind::ScopeTooNarrow));
        assert!(!policy.is_skippable(&SkipKind::Amnesty { code: "x".into() }));
        assert!(!policy.is_skippable(&SkipKind::Custom { code: "x".into() }));
    }

    #[test]
    fn accept_all_accepts_every_variant() {
        let policy = SkipPolicy::accept_all();
        assert!(policy.is_skippable(&SkipKind::ScopeTooNarrow));
        assert!(policy.is_skippable(&SkipKind::Amnesty { code: "x".into() }));
        assert!(policy.is_skippable(&SkipKind::Custom { code: "anything".into() }));
    }

    #[test]
    fn amnesty_with_codes_filters_exact() {
        let policy = SkipPolicy::reject_all().accept_amnesty("scope-change");
        assert!(policy.is_skippable(&SkipKind::Amnesty { code: "scope-change".into() }));
        assert!(!policy.is_skippable(&SkipKind::Amnesty { code: "other".into() }));
    }

    #[test]
    fn custom_without_wildcard_requires_exact_match() {
        let policy = SkipPolicy::reject_all().accept_custom("manual-review");
        assert!(policy.is_skippable(&SkipKind::Custom { code: "manual-review".into() }));
        assert!(!policy.is_skippable(&SkipKind::Custom { code: "else".into() }));
    }

    #[test]
    fn wildcard_accepts_any_custom_code() {
        let policy = SkipPolicy::reject_all().accept_any_custom();
        assert!(policy.is_skippable(&SkipKind::Custom { code: "whatever".into() }));
    }
}
