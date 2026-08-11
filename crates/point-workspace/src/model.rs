use std::fmt;

use point_contracts::{AttributeDataType, AttributeId, ContentHash, SourceId, WorldBounds};
use point_source::Source;
use serde::{Deserialize, Serialize};

use crate::{PointSet, WorkspaceDiagnostic, WorkspaceError};

const WORKSPACE_ID_BYTES: usize = 16;
const OPERATION_ID_BYTES: usize = 16;
const REVISION_ID_BYTES: usize = 32;

/// Stable opaque identity of one Workspace lineage.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "[u8; 16]", into = "[u8; 16]")]
pub struct WorkspaceId([u8; WORKSPACE_ID_BYTES]);

impl WorkspaceId {
    /// Generates a nonzero identity from operating-system randomness.
    ///
    /// # Errors
    ///
    /// Returns [`WorkspaceError::RandomnessUnavailable`] when the operating
    /// system cannot provide cryptographic randomness.
    pub fn generate() -> Result<Self, WorkspaceError> {
        generate_16(Self::from_bytes)
    }

    /// Creates an identity from checked opaque bytes.
    ///
    /// # Errors
    ///
    /// Returns an invalid-argument error for the reserved all-zero value.
    pub fn from_bytes(bytes: [u8; WORKSPACE_ID_BYTES]) -> Result<Self, WorkspaceError> {
        reject_zero_16("WorkspaceId", bytes).map(Self)
    }

    /// Copies a checked identity from one exact-width byte slice.
    ///
    /// # Errors
    ///
    /// Returns an invalid-argument error for the wrong width or all-zero value.
    pub fn try_from_slice(bytes: &[u8]) -> Result<Self, WorkspaceError> {
        Self::from_bytes(copy_16("WorkspaceId", bytes)?)
    }

    /// Borrows the opaque bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; WORKSPACE_ID_BYTES] {
        &self.0
    }

    /// Returns the opaque bytes.
    #[must_use]
    pub const fn into_bytes(self) -> [u8; WORKSPACE_ID_BYTES] {
        self.0
    }
}

impl TryFrom<[u8; WORKSPACE_ID_BYTES]> for WorkspaceId {
    type Error = WorkspaceError;

    fn try_from(bytes: [u8; WORKSPACE_ID_BYTES]) -> Result<Self, Self::Error> {
        Self::from_bytes(bytes)
    }
}

impl From<WorkspaceId> for [u8; WORKSPACE_ID_BYTES] {
    fn from(value: WorkspaceId) -> Self {
        value.into_bytes()
    }
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(formatter, &self.0)
    }
}

/// Caller-owned identity of one canonical commit request.
///
/// An Operation Identity is durable and retryable. It is not a runtime
/// [`JobId`](foundation_runtime::JobId).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "[u8; 16]", into = "[u8; 16]")]
pub struct OperationId([u8; OPERATION_ID_BYTES]);

impl OperationId {
    /// Generates a nonzero identity from operating-system randomness.
    ///
    /// # Errors
    ///
    /// Returns [`WorkspaceError::RandomnessUnavailable`] when the operating
    /// system cannot provide cryptographic randomness.
    pub fn generate() -> Result<Self, WorkspaceError> {
        generate_16(Self::from_bytes)
    }

    /// Creates an identity from checked opaque bytes.
    ///
    /// # Errors
    ///
    /// Returns an invalid-argument error for the reserved all-zero value.
    pub fn from_bytes(bytes: [u8; OPERATION_ID_BYTES]) -> Result<Self, WorkspaceError> {
        reject_zero_16("OperationId", bytes).map(Self)
    }

    /// Copies a checked identity from one exact-width byte slice.
    ///
    /// # Errors
    ///
    /// Returns an invalid-argument error for the wrong width or all-zero value.
    pub fn try_from_slice(bytes: &[u8]) -> Result<Self, WorkspaceError> {
        Self::from_bytes(copy_16("OperationId", bytes)?)
    }

    /// Borrows the opaque bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; OPERATION_ID_BYTES] {
        &self.0
    }

    /// Returns the opaque bytes.
    #[must_use]
    pub const fn into_bytes(self) -> [u8; OPERATION_ID_BYTES] {
        self.0
    }
}

impl TryFrom<[u8; OPERATION_ID_BYTES]> for OperationId {
    type Error = WorkspaceError;

    fn try_from(bytes: [u8; OPERATION_ID_BYTES]) -> Result<Self, Self::Error> {
        Self::from_bytes(bytes)
    }
}

impl From<OperationId> for [u8; OPERATION_ID_BYTES] {
    fn from(value: OperationId) -> Self {
        value.into_bytes()
    }
}

impl fmt::Display for OperationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(formatter, &self.0)
    }
}

/// Stable opaque identity of one immutable Workspace Revision.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "[u8; 32]", into = "[u8; 32]")]
pub struct RevisionId([u8; REVISION_ID_BYTES]);

impl RevisionId {
    /// Creates an identity from checked opaque bytes.
    ///
    /// # Errors
    ///
    /// Returns an invalid-argument error for the reserved all-zero value.
    pub fn from_bytes(bytes: [u8; REVISION_ID_BYTES]) -> Result<Self, WorkspaceError> {
        if bytes == [0; REVISION_ID_BYTES] {
            return Err(WorkspaceError::invalid(
                "RevisionId",
                "the all-zero identity is reserved",
            ));
        }
        Ok(Self(bytes))
    }

    /// Copies a checked identity from one exact-width byte slice.
    ///
    /// # Errors
    ///
    /// Returns an invalid-argument error for the wrong width or all-zero value.
    pub fn try_from_slice(bytes: &[u8]) -> Result<Self, WorkspaceError> {
        let bytes: [u8; REVISION_ID_BYTES] = bytes.try_into().map_err(|_| {
            WorkspaceError::invalid("RevisionId", "identity must contain exactly 32 bytes")
        })?;
        Self::from_bytes(bytes)
    }

    /// Borrows the opaque bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; REVISION_ID_BYTES] {
        &self.0
    }

    /// Returns the opaque bytes.
    #[must_use]
    pub const fn into_bytes(self) -> [u8; REVISION_ID_BYTES] {
        self.0
    }

    pub(crate) fn from_hash(bytes: [u8; REVISION_ID_BYTES]) -> Result<Self, WorkspaceError> {
        Self::from_bytes(bytes)
    }
}

impl TryFrom<[u8; REVISION_ID_BYTES]> for RevisionId {
    type Error = WorkspaceError;

    fn try_from(bytes: [u8; REVISION_ID_BYTES]) -> Result<Self, Self::Error> {
        Self::from_bytes(bytes)
    }
}

impl From<RevisionId> for [u8; REVISION_ID_BYTES] {
    fn from(value: RevisionId) -> Self {
        value.into_bytes()
    }
}

impl fmt::Display for RevisionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(formatter, &self.0)
    }
}

/// Editable Attribute contract fixed when a Workspace is created.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceSchema {
    classification: AttributeId,
}

impl WorkspaceSchema {
    /// Selects the Source Attribute used for effective classification values.
    #[must_use]
    pub const fn new(classification: AttributeId) -> Self {
        Self { classification }
    }

    /// Returns the caller-selected classification Attribute identity.
    #[must_use]
    pub const fn classification(self) -> AttributeId {
        self.classification
    }

    pub(crate) fn validate_source(self, source: &Source) -> Result<(), WorkspaceError> {
        let Some(definition) = source.metadata().attributes().get(self.classification) else {
            return Err(WorkspaceError::incompatible(format!(
                "Source does not contain classification Attribute {}",
                self.classification.get()
            )));
        };
        if definition.data_type() != AttributeDataType::U8 {
            return Err(WorkspaceError::incompatible(format!(
                "classification Attribute {} is {:?}, expected U8",
                self.classification.get(),
                definition.data_type()
            )));
        }
        Ok(())
    }
}

/// Complete identity chain for one immutable Snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SnapshotProvenance {
    workspace: WorkspaceId,
    source: SourceId,
    revision: RevisionId,
}

impl SnapshotProvenance {
    pub(crate) const fn new(
        workspace: WorkspaceId,
        source: SourceId,
        revision: RevisionId,
    ) -> Self {
        Self {
            workspace,
            source,
            revision,
        }
    }

    /// Returns the Workspace lineage.
    #[must_use]
    pub const fn workspace(self) -> WorkspaceId {
        self.workspace
    }

    /// Returns the immutable Source identity.
    #[must_use]
    pub const fn source(self) -> SourceId {
        self.source
    }

    /// Returns the pinned Revision identity.
    #[must_use]
    pub const fn revision(self) -> RevisionId {
        self.revision
    }
}

/// Canonical meaning of one immutable Revision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RevisionKind {
    /// Initial state whose effective values come entirely from the Source.
    Root,
    /// Assigns one classification value to a nonempty Point Set.
    SetClassification {
        /// Assigned effective classification value.
        value: u8,
        /// Points whose effective value actually changed.
        changed_points: u64,
    },
    /// Applies the inverse rows of the immediately preceding Revision.
    Revert {
        /// Immediate-head Revision whose rows were inverted.
        reverted_revision: RevisionId,
        /// Points restored by the inverse Revision.
        changed_points: u64,
    },
}

impl RevisionKind {
    /// Returns the number of changed Points, or zero for the root.
    #[must_use]
    pub const fn changed_points(self) -> u64 {
        match self {
            Self::Root => 0,
            Self::SetClassification { changed_points, .. }
            | Self::Revert { changed_points, .. } => changed_points,
        }
    }
}

/// Addressable facts for one immutable linear Revision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RevisionInfo {
    id: RevisionId,
    parent: Option<RevisionId>,
    sequence: u64,
    operation: Option<OperationId>,
    kind: RevisionKind,
}

impl RevisionInfo {
    pub(crate) const fn new(
        id: RevisionId,
        parent: Option<RevisionId>,
        sequence: u64,
        operation: Option<OperationId>,
        kind: RevisionKind,
    ) -> Self {
        Self {
            id,
            parent,
            sequence,
            operation,
            kind,
        }
    }

    /// Returns this Revision identity.
    #[must_use]
    pub const fn id(self) -> RevisionId {
        self.id
    }

    /// Returns the single predecessor, or `None` for the root.
    #[must_use]
    pub const fn parent(self) -> Option<RevisionId> {
        self.parent
    }

    /// Returns the zero-based linear sequence number.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }

    /// Returns the commit Operation, or `None` for the root.
    #[must_use]
    pub const fn operation(self) -> Option<OperationId> {
        self.operation
    }

    /// Returns the Revision's canonical Edit kind.
    #[must_use]
    pub const fn kind(self) -> RevisionKind {
        self.kind
    }
}

/// Exact Query grammar supported by v0.5.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointQuery {
    bounds: Option<WorldBounds>,
    classification_eq: Option<u8>,
}

impl PointQuery {
    /// Selects all Source Points before the optional Attribute predicate.
    #[must_use]
    pub const fn all() -> Self {
        Self {
            bounds: None,
            classification_eq: None,
        }
    }

    /// Selects Points inside one inclusive finite world box.
    #[must_use]
    pub const fn within(bounds: WorldBounds) -> Self {
        Self {
            bounds: Some(bounds),
            classification_eq: None,
        }
    }

    /// Adds equality against the Snapshot's effective classification value.
    #[must_use]
    pub const fn classification_is(mut self, value: u8) -> Self {
        self.classification_eq = Some(value);
        self
    }

    pub(crate) const fn bounds(self) -> Option<WorldBounds> {
        self.bounds
    }

    pub(crate) const fn classification_eq(self) -> Option<u8> {
        self.classification_eq
    }
}

impl Default for PointQuery {
    fn default() -> Self {
        Self::all()
    }
}

/// Exact immutable identity and provenance of one process-scoped Point Set.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PointSetMetadata {
    provenance: SnapshotProvenance,
    exact_count: u64,
    point_id_hash: ContentHash,
    content_hash: ContentHash,
}

impl PointSetMetadata {
    pub(crate) const fn new(
        provenance: SnapshotProvenance,
        exact_count: u64,
        point_id_hash: ContentHash,
        content_hash: ContentHash,
    ) -> Self {
        Self {
            provenance,
            exact_count,
            point_id_hash,
            content_hash,
        }
    }

    /// Returns the Snapshot against which membership and before-values are exact.
    #[must_use]
    pub const fn provenance(self) -> SnapshotProvenance {
        self.provenance
    }

    /// Returns the exact number of ordered unique Point Identities.
    #[must_use]
    pub const fn exact_count(self) -> u64 {
        self.exact_count
    }

    /// Returns the canonical hash of ordered Point Identities.
    #[must_use]
    pub const fn point_id_hash(self) -> ContentHash {
        self.point_id_hash
    }

    /// Returns the canonical hash of identities and private effective before-values.
    #[must_use]
    pub const fn content_hash(self) -> ContentHash {
        self.content_hash
    }
}

/// Caller request for one durable classification commit.
pub struct CommitRequest {
    operation: OperationId,
    kind: CommitRequestKind,
}

pub(crate) enum CommitRequestKind {
    SetClassification { points: PointSet, value: u8 },
    Revert { expected_head: RevisionId },
}

impl CommitRequest {
    /// Requests assignment of one classification value to an exact Point Set.
    #[must_use]
    pub fn set_classification(operation: OperationId, points: PointSet, value: u8) -> Self {
        Self {
            operation,
            kind: CommitRequestKind::SetClassification { points, value },
        }
    }

    /// Requests an inverse Edit for the immediate head Revision.
    #[must_use]
    pub fn revert_head(operation: OperationId, expected_head: RevisionId) -> Self {
        Self {
            operation,
            kind: CommitRequestKind::Revert { expected_head },
        }
    }

    /// Returns the caller-owned durable Operation Identity.
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        self.operation
    }

    pub(crate) fn into_parts(self) -> (OperationId, CommitRequestKind) {
        (self.operation, self.kind)
    }
}

/// Definitive reason that one commit request did not create a Revision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CommitRejection {
    /// The Point Set belongs to another Workspace or Source.
    ForeignPointSet,
    /// The request was pinned to a Revision that is no longer head.
    StaleHead {
        /// Revision required by the request.
        expected: RevisionId,
        /// Head observed while serializing the commit.
        actual: RevisionId,
    },
    /// Every selected Point already had the requested effective value.
    NoChanges,
    /// The root has no Edit rows that can be inverted.
    RootCannotBeReverted,
    /// The Operation Identity is already bound to different canonical intent.
    OperationConflict,
}

impl CommitRejection {
    pub(crate) const fn code(self) -> u16 {
        match self {
            Self::ForeignPointSet => 1,
            Self::StaleHead { .. } => 2,
            Self::NoChanges => 3,
            Self::RootCannotBeReverted => 4,
            Self::OperationConflict => 5,
        }
    }

    pub(crate) fn from_code(
        code: u16,
        expected: Option<RevisionId>,
        actual: Option<RevisionId>,
    ) -> Result<Self, WorkspaceError> {
        match code {
            1 => Ok(Self::ForeignPointSet),
            2 => Ok(Self::StaleHead {
                expected: expected.ok_or_else(|| {
                    WorkspaceError::corrupt("stale-head rejection has no expected Revision")
                })?,
                actual: actual.ok_or_else(|| {
                    WorkspaceError::corrupt("stale-head rejection has no actual Revision")
                })?,
            }),
            3 => Ok(Self::NoChanges),
            4 => Ok(Self::RootCannotBeReverted),
            5 => Ok(Self::OperationConflict),
            _ => Err(WorkspaceError::corrupt(
                "recorded rejection has an unknown reason code",
            )),
        }
    }
}

/// Persistence phase after which a commit acknowledgement became uncertain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitPhase {
    /// Publishing an immutable operation intent or rejection.
    OperationPublication,
    /// Publishing the immutable Revision file by no-replace hard link.
    RevisionPublication,
    /// Synchronizing the Revision directory after a visible hard link.
    RevisionDirectorySync,
}

/// Explicit uncertain result that must be reconciled by Operation Identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitUncertainty {
    operation: OperationId,
    phase: CommitPhase,
    reason: WorkspaceDiagnostic,
}

impl CommitUncertainty {
    pub(crate) fn new(operation: OperationId, phase: CommitPhase, reason: impl AsRef<str>) -> Self {
        Self {
            operation,
            phase,
            reason: WorkspaceDiagnostic::new(reason),
        }
    }

    /// Returns the Operation that must be resolved after reopen.
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        self.operation
    }

    /// Returns the publication phase whose outcome is uncertain.
    #[must_use]
    pub const fn phase(&self) -> CommitPhase {
        self.phase
    }

    /// Returns a bounded failure diagnostic.
    #[must_use]
    pub fn reason(&self) -> &WorkspaceDiagnostic {
        &self.reason
    }
}

/// Durable acknowledgement for one committed Revision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommitReceipt {
    operation: OperationId,
    revision: RevisionInfo,
}

impl CommitReceipt {
    pub(crate) const fn new(operation: OperationId, revision: RevisionInfo) -> Self {
        Self {
            operation,
            revision,
        }
    }

    /// Returns the caller's Operation Identity.
    #[must_use]
    pub const fn operation(self) -> OperationId {
        self.operation
    }

    /// Returns the newly committed immutable Revision identity.
    #[must_use]
    pub const fn revision(self) -> RevisionId {
        self.revision.id()
    }

    /// Returns all newly committed immutable Revision facts.
    #[must_use]
    pub const fn revision_info(self) -> RevisionInfo {
        self.revision
    }
}

/// Terminal result returned by a custom commit Job.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommitOutcome {
    /// Immutable Revision publication and directory synchronization succeeded.
    Committed(CommitReceipt),
    /// No Revision was created for a definitive reason.
    Rejected(CommitRejection),
    /// Publication may have become visible and requires reopen plus resolution.
    Indeterminate(CommitUncertainty),
}

/// Durable canonical intent that can safely resume without a live Point Set.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecordedIntent {
    operation: OperationId,
    request_hash: ContentHash,
    parent: SnapshotProvenance,
    revision: RevisionId,
    sequence: u64,
    kind: RevisionKind,
    point_set: Option<PointSetMetadata>,
}

impl RecordedIntent {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn new(
        operation: OperationId,
        request_hash: ContentHash,
        parent: SnapshotProvenance,
        revision: RevisionId,
        sequence: u64,
        kind: RevisionKind,
        point_set: Option<PointSetMetadata>,
    ) -> Self {
        Self {
            operation,
            request_hash,
            parent,
            revision,
            sequence,
            kind,
            point_set,
        }
    }

    /// Returns the durable Operation Identity.
    #[must_use]
    pub const fn operation(self) -> OperationId {
        self.operation
    }

    /// Returns the hash of the canonical commit request.
    #[must_use]
    pub const fn request_hash(self) -> ContentHash {
        self.request_hash
    }

    /// Returns the exact parent Snapshot provenance.
    #[must_use]
    pub const fn parent(self) -> SnapshotProvenance {
        self.parent
    }

    /// Returns the proposed Revision identity.
    #[must_use]
    pub const fn revision(self) -> RevisionId {
        self.revision
    }

    /// Returns the proposed linear sequence number.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }

    /// Returns the canonical Edit kind.
    #[must_use]
    pub const fn kind(self) -> RevisionKind {
        self.kind
    }

    /// Returns exact Point Set facts for assignment, or `None` for Revert.
    #[must_use]
    pub const fn point_set(self) -> Option<PointSetMetadata> {
        self.point_set
    }
}

/// Durable definitive rejection for one canonical Operation request.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecordedRejection {
    operation: OperationId,
    request_hash: ContentHash,
    reason: CommitRejection,
}

impl RecordedRejection {
    pub(crate) const fn new(
        operation: OperationId,
        request_hash: ContentHash,
        reason: CommitRejection,
    ) -> Self {
        Self {
            operation,
            request_hash,
            reason,
        }
    }

    /// Returns the rejected Operation Identity.
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        self.operation
    }

    /// Returns the canonical request hash.
    #[must_use]
    pub const fn request_hash(&self) -> ContentHash {
        self.request_hash
    }

    /// Returns the definitive rejection reason.
    #[must_use]
    pub const fn reason(self) -> CommitRejection {
        self.reason
    }
}

/// Recovered durable state for one caller-owned Operation Identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationResolution {
    /// The Operation published exactly one immutable Revision.
    Committed(CommitReceipt),
    /// The Operation has a durable definitive rejection.
    Rejected(RecordedRejection),
    /// A complete durable intent exists and can safely resume.
    Retryable(Box<RecordedIntent>),
    /// No durable record exists for this Operation Identity.
    NotRecorded,
    /// Recovery could not prove a safe terminal state.
    Indeterminate(CommitUncertainty),
}

fn generate_16<T>(
    constructor: fn([u8; 16]) -> Result<T, WorkspaceError>,
) -> Result<T, WorkspaceError> {
    loop {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(WorkspaceError::random)?;
        if let Ok(identity) = constructor(bytes) {
            return Ok(identity);
        }
    }
}

fn reject_zero_16(name: &'static str, bytes: [u8; 16]) -> Result<[u8; 16], WorkspaceError> {
    if bytes == [0; 16] {
        return Err(WorkspaceError::invalid(
            name,
            "the all-zero identity is reserved",
        ));
    }
    Ok(bytes)
}

fn copy_16(name: &'static str, bytes: &[u8]) -> Result<[u8; 16], WorkspaceError> {
    bytes
        .try_into()
        .map_err(|_| WorkspaceError::invalid(name, "identity must contain exactly 16 bytes"))
}

fn write_hex(formatter: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    for byte in bytes {
        write!(formatter, "{byte:02x}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use point_contracts::WorldBounds;

    use super::{
        CommitReceipt, CommitRejection, OperationId, PointQuery, RevisionId, RevisionInfo,
        RevisionKind, WorkspaceId,
    };

    #[test]
    fn opaque_identities_reject_reserved_zero_and_round_trip_bytes() {
        assert!(WorkspaceId::from_bytes([0; 16]).is_err());
        assert!(OperationId::from_bytes([0; 16]).is_err());
        assert!(RevisionId::from_bytes([0; 32]).is_err());

        let workspace = WorkspaceId::from_bytes([1; 16]).expect("nonzero Workspace ID");
        let operation = OperationId::from_bytes([2; 16]).expect("nonzero Operation ID");
        let revision = RevisionId::from_bytes([3; 32]).expect("nonzero Revision ID");
        assert_eq!(workspace.into_bytes(), [1; 16]);
        assert_eq!(operation.into_bytes(), [2; 16]);
        assert_eq!(revision.into_bytes(), [3; 32]);
    }

    #[test]
    fn query_grammar_is_only_all_or_inclusive_bounds_plus_classification() {
        let bounds = WorldBounds::new([1.0, 2.0, 3.0], [1.0, 4.0, 5.0])
            .expect("inclusive degenerate axis is valid");
        let query = PointQuery::within(bounds).classification_is(7);
        assert_eq!(query.bounds(), Some(bounds));
        assert_eq!(query.classification_eq(), Some(7));
        assert_eq!(PointQuery::all().bounds(), None);
    }

    #[test]
    fn commit_receipt_exposes_identity_and_full_revision_facts() {
        let operation = OperationId::from_bytes([4; 16]).expect("nonzero Operation ID");
        let revision = RevisionId::from_bytes([5; 32]).expect("nonzero Revision ID");
        let info = RevisionInfo::new(
            revision,
            None,
            0,
            Some(operation),
            RevisionKind::SetClassification {
                value: 2,
                changed_points: 3,
            },
        );
        let receipt = CommitReceipt::new(operation, info);
        assert_eq!(receipt.revision(), revision);
        assert_eq!(receipt.revision_info(), info);
    }

    #[test]
    fn persisted_rejection_codes_are_stable_and_checked() {
        let expected = RevisionId::from_bytes([6; 32]).expect("nonzero Revision ID");
        let actual = RevisionId::from_bytes([7; 32]).expect("nonzero Revision ID");
        let rejection = CommitRejection::StaleHead { expected, actual };
        assert_eq!(rejection.code(), 2);
        assert_eq!(
            CommitRejection::from_code(2, Some(expected), Some(actual))
                .expect("known rejection code"),
            rejection
        );
        assert!(CommitRejection::from_code(u16::MAX, None, None).is_err());
    }
}
