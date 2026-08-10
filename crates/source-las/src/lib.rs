//! Verified, bounded LAS and LAZ access through Punctra's canonical Source interface.
//!
//! The public interface deliberately contains only path-based opening. Format
//! parsing, file witnesses, compression, and canonical Attribute mapping stay
//! private to this adapter.
//!
//! Uncompressed LAS point formats 0–10 and `LASzip` LAZ point formats 0–8 are
//! supported. Compressed point formats 9 and 10 are rejected explicitly: the
//! codec boundary cannot currently guarantee exact layered `WavePacket14`
//! values across scanner-channel contexts. The adapter returns
//! [`point_source::SourceError::UnsupportedFormat`] before point verification
//! can publish a Source.
//!
//! Logical ordinals are zero-based point-record order. Source Identity covers
//! the exact file bytes, so equivalent LAS and LAZ encodings intentionally have
//! different identities.
//!
//! # Canonical Attribute mapping
//!
//! Attributes appear only when their point format contains the corresponding
//! field. Projection uses these stable IDs and exact canonical types:
//!
//! | ID | Name | Type |
//! |---:|---|---|
//! | 1 | intensity | `U16` |
//! | 2–5 | return number, number of returns, scan direction, edge of flight line | `U8` |
//! | 6–10 | classification, synthetic, key point, withheld, overlap | `U8` |
//! | 11 | scanner channel (extended formats) | `U8` |
//! | 12 | scan angle | `I8` for formats 0–5; `I16` for formats 6–10 |
//! | 13 | user data | `U8` |
//! | 14 | point source ID | `U16` |
//! | 15 | GPS time | `F64` |
//! | 16–18 | red, green, blue | `U16` |
//! | 19 | waveform descriptor (LAS 4/5/9/10 and LAZ 4/5) | `U8` |
//! | 20 | waveform byte offset | `U64` |
//! | 21 | waveform packet size | `U32` |
//! | 22–25 | waveform location, dx, dy, dz | `F32` |
//! | 26 | near infrared | `U16` |
//! | 4096 | uninterpreted trailing Extra Bytes record slab | `FixedBytes(record width)` |
//!
//! Extra Bytes VLR dimension semantics are preserved in metadata but are not
//! interpreted into separate typed columns in v0.3.
//!
//! # Metadata and Coordinate Reference
//!
//! Regular VLR payloads are preserved in order under namespace `las.vlr` and
//! EVLR payloads under `las.evlr`. Their canonical metadata name is
//! `user_id:record_id:description`; the payload is the exact record data. One
//! non-empty UTF-8 `LASF_Projection` record 2112 is exposed as WKT. Missing,
//! empty, invalid, or ambiguous WKT remains explicitly unknown while the raw
//! metadata records stay available.
//!
//! # Verification
//!
//! Identifying and Full reopening hash every file byte and validate every point
//! record before publishing a [`point_source::Source`]. The verified bytes are
//! retained in a private anonymous snapshot, so later path or in-place changes
//! cannot alter Points published under the established Source Identity. This
//! adapter has no weaker stable file witness: `FastOnly` reopening returns
//! [`point_source::SourceError::VerificationRequired`], while `FastThenFull`
//! falls back to Full verification.
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let source = source_las::open("survey.laz").blocking_wait()?;
//! println!("{} authoritative Points", source.metadata().point_count());
//!
//! let mut batches = source.points()?;
//! while let Some(batch) = batches.next()? {
//!     println!("{} Points from ordinal {}", batch.len(), batch.first_ordinal());
//! }
//! assert!(batches.summary().is_some());
//!
//! let options = point_source::OpenOptions::match_record(
//!     source.record().clone(),
//!     point_source::VerificationPolicy::FastThenFull,
//! );
//! let reopened = source_las::open_with("survey.laz", options).blocking_wait()?;
//! assert_eq!(reopened.identity(), source.identity());
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod decode;
mod format;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use foundation_runtime::OperationReporter;
use point_source::adapter::{AdapterContract, AdapterVerified, CandidateAdapter, FullVerification};
use point_source::{OpenOptions, SourceCandidate, SourceError, SourceJob, SourcePreview};

use crate::decode::LasReadAdapter;
use crate::format::{VerifiedFile, verify_file};

const ADAPTER_NAME: &str = "source-las";
const ADAPTER_VERSION: &str = "1";
const LOGICAL_ORDER: &str = "LAS/LAZ point-record order v1";
const FAST_TOKEN: &[u8] = b"full-only-v1";

/// Opens a supported local LAS or LAZ file through mandatory Full verification.
///
/// LAZ point formats 9 and 10 return [`SourceError::UnsupportedFormat`].
#[must_use]
pub fn open(path: impl AsRef<Path>) -> SourceJob {
    open_with(path, OpenOptions::identify())
}

/// Opens or reopens a local LAS or LAZ file with explicit verification options.
///
/// `FastOnly` returns [`SourceError::VerificationRequired`]. Use
/// [`point_source::VerificationPolicy::FastThenFull`] to request a Fast check
/// with the adapter's required Full fallback.
/// LAZ point formats 9 and 10 return [`SourceError::UnsupportedFormat`].
#[must_use]
pub fn open_with(path: impl AsRef<Path>, options: OpenOptions) -> SourceJob {
    candidate(path.as_ref().to_path_buf()).open(options)
}

fn candidate(path: PathBuf) -> SourceCandidate {
    let display_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    SourceCandidate::new_adapter(LasCandidate {
        path,
        preview: SourcePreview::new("LAS/LAZ", display_name),
    })
}

struct LasCandidate {
    path: PathBuf,
    preview: SourcePreview,
}

impl CandidateAdapter for LasCandidate {
    fn preview(&self) -> &SourcePreview {
        &self.preview
    }

    fn full_verify(
        &self,
        verification: FullVerification,
        reporter: &OperationReporter,
    ) -> Result<AdapterVerified, SourceError> {
        let verified = verify_file(&self.path, verification, reporter)?;
        Ok(publish_verified(verified))
    }

    fn fast_verify(
        &self,
        _expected_fast_token: &[u8],
        reporter: &OperationReporter,
    ) -> Result<AdapterVerified, SourceError> {
        reporter.check_cancelled()?;
        Err(SourceError::VerificationRequired)
    }
}

fn publish_verified(verified: VerifiedFile) -> AdapterVerified {
    let reader = Arc::new(LasReadAdapter::new(
        verified.file,
        verified.source_witness,
        Arc::clone(&verified.layout),
    ));
    AdapterVerified::new(
        AdapterContract::new(ADAPTER_NAME, ADAPTER_VERSION, LOGICAL_ORDER)
            .expect("the static LAS adapter contract is valid"),
        verified.metadata,
        verified.content_hash,
        FAST_TOKEN.to_vec(),
        reader,
    )
}
