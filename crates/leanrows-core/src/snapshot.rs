use crate::PositionedRead;
use std::io;

/// Stable object identity evidence supplied by a positioned source.
///
/// The two values are opaque to the core. File-backed implementations use a
/// volume/device identifier plus a file identifier. Equality is meaningful
/// only between fingerprints produced by the same source implementation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceIdentity {
    namespace: u128,
    object: u128,
}

impl SourceIdentity {
    /// Creates opaque source-identity evidence.
    #[must_use]
    pub const fn new(namespace: u128, object: u128) -> Self {
        Self { namespace, object }
    }
}

/// Observable revision evidence supplied by a positioned source.
///
/// File-backed implementations use write/change timestamps. This is not a
/// content hash: a filesystem or privileged writer that preserves all exposed
/// metadata can still make an undetectable same-size rewrite.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceRevision {
    primary: u128,
    secondary: u128,
}

impl SourceRevision {
    /// Creates opaque source-revision evidence.
    #[must_use]
    pub const fn new(primary: u128, secondary: u128) -> Self {
        Self { primary, secondary }
    }
}

/// Bounded evidence describing one observed source state.
///
/// Size is always present. Identity and revision are optional so in-memory and
/// platform-specific sources can participate without inventing evidence. A
/// file-backed source should expose every reliable field available from its
/// open handle.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceFingerprint {
    size: u64,
    identity: Option<SourceIdentity>,
    revision: Option<SourceRevision>,
}

impl SourceFingerprint {
    /// Creates a portable, size-only fingerprint.
    #[must_use]
    pub const fn new(size: u64) -> Self {
        Self {
            size,
            identity: None,
            revision: None,
        }
    }

    /// Creates a fingerprint with optional identity and revision evidence.
    #[must_use]
    pub const fn with_evidence(
        size: u64,
        identity: Option<SourceIdentity>,
        revision: Option<SourceRevision>,
    ) -> Self {
        Self {
            size,
            identity,
            revision,
        }
    }

    /// Captures the source fingerprint exposed for the current operation.
    ///
    /// # Errors
    /// Returns an I/O error when source evidence cannot be read.
    pub fn capture<R: PositionedRead + ?Sized>(source: &R) -> io::Result<Self> {
        source.fingerprint()
    }

    /// Returns the observed byte length.
    #[must_use]
    pub const fn size(self) -> u64 {
        self.size
    }

    /// Returns the optional stable object identity.
    #[must_use]
    pub const fn identity(self) -> Option<SourceIdentity> {
        self.identity
    }

    /// Returns the optional revision evidence.
    #[must_use]
    pub const fn revision(self) -> Option<SourceRevision> {
        self.revision
    }

    /// Classifies a difference from `self` to `current`.
    ///
    /// Identity is checked before size so a rotated path is not misreported as
    /// an append or truncate. Losing evidence that existed in the baseline is
    /// fail-closed; newly available evidence is accepted because the baseline
    /// cannot prove what its original value would have been.
    #[must_use]
    pub fn compare(self, current: Self) -> SourceChange {
        match (self.identity, current.identity) {
            (Some(expected), Some(actual)) if expected != actual => {
                return SourceChange::IdentityChanged;
            }
            (Some(_), None) => return SourceChange::EvidenceLost,
            _ => {}
        }
        if current.size < self.size {
            return SourceChange::Shrank;
        }
        if current.size > self.size {
            return SourceChange::Grew;
        }
        match (self.revision, current.revision) {
            (Some(expected), Some(actual)) if expected != actual => SourceChange::RevisionChanged,
            (Some(_), None) => SourceChange::EvidenceLost,
            _ => SourceChange::Unchanged,
        }
    }
}

/// The observable relationship between two source fingerprints.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SourceChange {
    /// No difference was observable using the available evidence.
    Unchanged,
    /// The same observable object is longer than the baseline.
    Grew,
    /// The same observable object is shorter than the baseline.
    Shrank,
    /// The stable object identity differs, as with replacement or rotation.
    IdentityChanged,
    /// Size and identity match but write/change evidence differs.
    RevisionChanged,
    /// Evidence present in the baseline is no longer available.
    EvidenceLost,
}

impl SourceChange {
    /// Reports whether the comparison detected a source-integrity failure.
    #[must_use]
    pub const fn is_changed(self) -> bool {
        !matches!(self, Self::Unchanged)
    }
}

#[cfg(test)]
mod tests {
    use super::{SourceChange, SourceFingerprint, SourceIdentity, SourceRevision};

    #[test]
    fn comparisons_prioritize_identity_then_size_then_revision() {
        let identity = SourceIdentity::new(7, 11);
        let revision = SourceRevision::new(13, 17);
        let baseline = SourceFingerprint::with_evidence(100, Some(identity), Some(revision));

        assert_eq!(baseline.compare(baseline), SourceChange::Unchanged);
        assert_eq!(
            baseline.compare(SourceFingerprint::with_evidence(
                100,
                Some(SourceIdentity::new(7, 12)),
                Some(revision),
            )),
            SourceChange::IdentityChanged
        );
        assert_eq!(
            baseline.compare(SourceFingerprint::with_evidence(
                101,
                Some(identity),
                Some(SourceRevision::new(19, 23)),
            )),
            SourceChange::Grew
        );
        assert_eq!(
            baseline.compare(SourceFingerprint::with_evidence(
                99,
                Some(identity),
                Some(SourceRevision::new(19, 23)),
            )),
            SourceChange::Shrank
        );
        assert_eq!(
            baseline.compare(SourceFingerprint::with_evidence(
                100,
                Some(identity),
                Some(SourceRevision::new(19, 23)),
            )),
            SourceChange::RevisionChanged
        );
        assert_eq!(
            baseline.compare(SourceFingerprint::with_evidence(100, None, Some(revision))),
            SourceChange::EvidenceLost
        );
    }

    #[test]
    fn size_only_evidence_does_not_overstate_same_size_stability() {
        let baseline = SourceFingerprint::new(100);
        assert_eq!(
            baseline.compare(SourceFingerprint::new(100)),
            SourceChange::Unchanged
        );
        assert_eq!(
            baseline.compare(SourceFingerprint::new(101)),
            SourceChange::Grew
        );
        assert_eq!(
            baseline.compare(SourceFingerprint::new(99)),
            SourceChange::Shrank
        );
    }
}
