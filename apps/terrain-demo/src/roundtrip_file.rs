//! Stable bounded capture for round-trip inputs.

use std::{
    fs::File,
    io::{Read as _, Seek as _, SeekFrom},
    path::Path,
};

use crate::{
    roundtrip::{InputSide, RoundTripFailure},
    stable_file::StableFile,
};

pub(crate) struct FileSnapshot {
    pub(crate) bytes: Vec<u8>,
}

pub(crate) struct CapturedRoundTripFile {
    pub(crate) bytes: Vec<u8>,
    witness: StableFile,
}

impl CapturedRoundTripFile {
    pub(crate) fn verify(&self) -> Result<(), RoundTripFailure> {
        self.witness.verify().map_err(|error| {
            RoundTripFailure::invalid(format_args!(
                "{} changed after it was captured: {error}",
                InputSide::Returned
            ))
        })
    }
}

pub(crate) fn capture_round_trip_file(
    path: &Path,
    max_file_bytes: u64,
) -> Result<CapturedRoundTripFile, RoundTripFailure> {
    let witness = capture_regular_file(InputSide::Returned, path, max_file_bytes)?;
    let (snapshot, witness) =
        read_regular_file_retained(InputSide::Returned, witness, max_file_bytes)?;
    Ok(CapturedRoundTripFile {
        bytes: snapshot.bytes,
        witness,
    })
}

pub(crate) fn capture_file_pair(
    reference_path: &Path,
    returned_path: &Path,
    max_file_bytes: u64,
) -> Result<(StableFile, StableFile), RoundTripFailure> {
    let reference = capture_regular_file(InputSide::Reference, reference_path, max_file_bytes)?;
    let returned = capture_regular_file(InputSide::Returned, returned_path, max_file_bytes)?;
    Ok((reference, returned))
}

fn capture_regular_file(
    side: InputSide,
    path: &Path,
    max_file_bytes: u64,
) -> Result<StableFile, RoundTripFailure> {
    let witness = StableFile::capture(path).map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} cannot be captured: {error}"))
    })?;
    require_file_bytes(side, witness.byte_length(), max_file_bytes)?;
    Ok(witness)
}

#[cfg(test)]
pub(crate) fn capture_inspected_regular_file(
    side: InputSide,
    path: &Path,
    path_metadata: &std::fs::Metadata,
    max_file_bytes: u64,
) -> Result<StableFile, RoundTripFailure> {
    require_file_bytes(side, path_metadata.len(), max_file_bytes)?;
    StableFile::capture_from_metadata(path, path_metadata).map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} cannot be captured: {error}"))
    })
}

pub(crate) fn read_regular_file(
    side: InputSide,
    witness: StableFile,
    max_file_bytes: u64,
) -> Result<FileSnapshot, RoundTripFailure> {
    read_regular_file_retained(side, witness, max_file_bytes).map(|(snapshot, _)| snapshot)
}

fn read_regular_file_retained(
    side: InputSide,
    mut witness: StableFile,
    max_file_bytes: u64,
) -> Result<(FileSnapshot, StableFile), RoundTripFailure> {
    let expected_bytes = witness.byte_length();
    let bytes = read_bounded_bytes(side, witness.file_mut(), expected_bytes, max_file_bytes)?;
    witness.verify().map_err(|error| {
        RoundTripFailure::invalid(format_args!(
            "{side} changed while it was being read: {error}"
        ))
    })?;
    if bytes.len() as u64 != expected_bytes {
        return Err(RoundTripFailure::invalid(format_args!(
            "{side} changed while it was being read"
        )));
    }
    Ok((FileSnapshot { bytes }, witness))
}

pub(crate) fn require_file_bytes(
    side: InputSide,
    actual: u64,
    allowed: u64,
) -> Result<(), RoundTripFailure> {
    if actual > allowed {
        return Err(RoundTripFailure::resource(format_args!(
            "{side} file bytes required {actual}; limit is {allowed}"
        )));
    }
    Ok(())
}

fn read_bounded_bytes(
    side: InputSide,
    file: &mut File,
    expected_bytes: u64,
    max_file_bytes: u64,
) -> Result<Vec<u8>, RoundTripFailure> {
    let capacity = usize::try_from(expected_bytes).map_err(|_| {
        RoundTripFailure::resource(format_args!(
            "{side} file length does not fit this platform"
        ))
    })?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(capacity).map_err(|_| {
        RoundTripFailure::resource(format_args!(
            "{side} file buffer cannot reserve {expected_bytes} bytes"
        ))
    })?;
    file.seek(SeekFrom::Start(0)).map_err(|error| {
        RoundTripFailure::invalid(format_args!("{side} cannot be rewound: {error}"))
    })?;
    file.take(max_file_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            RoundTripFailure::invalid(format_args!("{side} cannot be read: {error}"))
        })?;
    require_file_bytes(side, bytes.len() as u64, max_file_bytes)?;
    Ok(bytes)
}
