//! Lock-holder metadata sidecar.
//!
//! Every held lock writes a JSON sidecar alongside the `.lock` file
//! containing the holder's PID, hostname, acquisition time, and lease
//! duration. When a contender attempts to acquire, it reads this
//! sidecar to decide whether the current holder is still alive — and
//! if not, whether the lease has expired and the lock can be
//! reclaimed.

use std::time::Duration;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// Lock-holder identity fields stored alongside a held lock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockOwner {
    /// Process id of the holder.
    pub pid: u32,
    /// Hostname of the holder, or `None` when unresolvable.
    pub hostname: Option<String>,
}

impl LockOwner {
    /// Identify the current process.
    #[must_use]
    pub fn current() -> Self {
        let hostname = hostname_best_effort();
        Self { pid: std::process::id(), hostname }
    }
}

/// Full lock metadata — written on acquire, consulted on contention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockMetadata {
    /// Who holds the lock.
    pub owner: LockOwner,
    /// When the lock was acquired.
    pub acquired_at: Timestamp,
    /// Maximum lifetime; `now > acquired_at + lease` implies staleness.
    #[serde(with = "serde_duration")]
    pub lease: Duration,
}

impl LockMetadata {
    /// Has the lease expired relative to the supplied `now`?
    #[must_use]
    pub fn is_expired(&self, now: Timestamp) -> bool {
        let lease_span = jiff::Span::try_from(self.lease).unwrap_or_default();
        match self.acquired_at.checked_add(lease_span) {
            Ok(expires) => now > expires,
            Err(_) => true,
        }
    }
}

fn hostname_best_effort() -> Option<String> {
    // Avoid pulling an extra hostname crate; Env var fallback is
    // sufficient for observability. Callers who need a resolved
    // hostname set `KNOTCH_HOSTNAME`.
    std::env::var("KNOTCH_HOSTNAME")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
}

mod serde_duration {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S: Serializer>(d: &Duration, ser: S) -> Result<S::Ok, S::Error> {
        d.as_secs_f64().serialize(ser)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Duration, D::Error> {
        let secs = f64::deserialize(de)?;
        if !secs.is_finite() || secs < 0.0 {
            return Err(serde::de::Error::custom("lease must be finite and >= 0"));
        }
        Ok(Duration::from_secs_f64(secs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_roundtrips_through_json() {
        let meta = LockMetadata {
            owner: LockOwner { pid: 42, hostname: Some("devbox".into()) },
            acquired_at: Timestamp::from_second(1_700_000_000).expect("ts"),
            lease: Duration::from_secs(60),
        };
        let json = serde_json::to_string(&meta).expect("serialize");
        let back: LockMetadata = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(meta, back);
    }

    #[test]
    fn lease_expiry_reports_correctly() {
        let meta = LockMetadata {
            owner: LockOwner { pid: 42, hostname: None },
            acquired_at: Timestamp::from_second(1_000).expect("ts"),
            lease: Duration::from_secs(60),
        };
        assert!(!meta.is_expired(Timestamp::from_second(1_030).expect("ts")));
        assert!(meta.is_expired(Timestamp::from_second(1_100).expect("ts")));
    }
}
