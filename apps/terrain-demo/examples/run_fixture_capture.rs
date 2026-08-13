//! Captures one already-published Complete Run as an immutable v1 test corpus.
//!
//! This owner-only utility validates the journal chain and exact artifact
//! witnesses before writing a new target. It never overwrites an existing
//! corpus and does not rewrite the Run journal's owner-local path bindings.

use std::{
    env,
    error::Error,
    ffi::OsString,
    fs, io,
    ops::Range,
    path::{Path, PathBuf},
};

const HEADER_MAGIC: &[u8; 8] = b"PTWFJ001";
const FRAME_MAGIC: &[u8; 4] = b"PWF1";
const HEADER_HASH_DOMAIN: &[u8] = b"punctra-terrain-workflow-header-v1";
const FRAME_HASH_DOMAIN: &[u8] = b"punctra-terrain-workflow-frame-v1";
const REPORT_HASH_DOMAIN: &[u8] = b"punctra-terrain-workflow-report-bytes-v1";
const HEADER_BYTES: usize = 80;
const FRAME_HEADER_BYTES: usize = 56;
const FRAME_HASH_BYTES: usize = 32;
const FRAME_PAYLOAD_BYTES: [Option<usize>; 8] = [
    None,
    Some(96),
    Some(193),
    Some(424),
    Some(136),
    Some(169),
    Some(200),
    Some(224),
];
const PREFIX_NAMES: [&str; 8] = [
    "01-intent.pwf",
    "02-revision-resolved.pwf",
    "03-audit-observed.pwf",
    "04-surface-observed.pwf",
    "05-qa-observed.pwf",
    "06-export-ensured.pwf",
    "07-report-ensured.pwf",
    "08-complete.pwf",
];
const README: &str = include_str!("../tests/fixtures/run-v1/README.md");

type CaptureResult<T> = Result<T, Box<dyn Error>>;

struct JournalScan {
    run: [u8; 16],
    payloads: Vec<Range<usize>>,
    prefix_ends: Vec<usize>,
}

fn main() {
    if let Err(error) = capture(arguments()) {
        eprintln!("run fixture capture failed: {error}");
        std::process::exit(2);
    }
}

fn arguments() -> (OsString, OsString) {
    let mut arguments = env::args_os().skip(1);
    let source = arguments.next().unwrap_or_else(|| {
        eprintln!("usage: run_fixture_capture SOURCE_COMPLETE_RUN_ROOT NEW_CORPUS_ROOT");
        std::process::exit(2);
    });
    let target = arguments.next().unwrap_or_else(|| {
        eprintln!("usage: run_fixture_capture SOURCE_COMPLETE_RUN_ROOT NEW_CORPUS_ROOT");
        std::process::exit(2);
    });
    if arguments.next().is_some() {
        eprintln!("usage: run_fixture_capture SOURCE_COMPLETE_RUN_ROOT NEW_CORPUS_ROOT");
        std::process::exit(2);
    }
    (source, target)
}

fn capture((source, target): (OsString, OsString)) -> CaptureResult<()> {
    let source = PathBuf::from(source);
    let target = PathBuf::from(target);
    require_absent(&target)?;

    let journal = read_regular_file(&source.join("run.pwf"))?;
    let terrain = read_regular_file(&source.join("terrain.xml"))?;
    let report = read_regular_file(&source.join("audit.json"))?;
    let lock = read_regular_file(&source.join("run.lock"))?;
    if !lock.is_empty() {
        return Err(invalid("Complete Run lock is not empty").into());
    }
    let scan = scan_complete_journal(&journal)?;
    validate_artifact_witnesses(&journal, &scan, &terrain, &report)?;
    let identities = validate_report(&report, scan.run, &journal, &scan)?;

    let complete = target.join("complete");
    let prefixes = target.join("prefixes");
    fs::create_dir_all(&complete)?;
    fs::create_dir_all(&prefixes)?;
    fs::write(target.join("README.md"), README)?;
    fs::write(complete.join("run.pwf"), &journal)?;
    fs::write(complete.join("terrain.xml"), &terrain)?;
    fs::write(complete.join("audit.json"), &report)?;
    fs::write(complete.join("run.lock"), lock)?;
    for (name, end) in PREFIX_NAMES.iter().zip(&scan.prefix_ends) {
        fs::write(prefixes.join(name), &journal[..*end])?;
    }
    write_manifest(&target, &identities)?;
    Ok(())
}

fn require_absent(path: &Path) -> CaptureResult<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("refusing to overwrite {}", path.display()),
        )
        .into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn read_regular_file(path: &Path) -> CaptureResult<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(invalid(format!("{} is not a regular file", path.display())).into());
    }
    let bytes = fs::read(path)?;
    if metadata.len() != bytes.len() as u64 {
        return Err(invalid(format!("{} changed while being read", path.display())).into());
    }
    Ok(bytes)
}

fn scan_complete_journal(bytes: &[u8]) -> CaptureResult<JournalScan> {
    let header = bytes
        .get(..HEADER_BYTES)
        .ok_or_else(|| invalid("journal header is truncated"))?;
    if &header[..8] != HEADER_MAGIC
        || read_u32(&header[8..12]) != 1
        || read_u32(&header[12..16]) != 1
        || read_u32(&header[16..20])
            != u32::try_from(HEADER_BYTES).expect("fixed header width fits u32")
        || header[20..24] != [0; 4]
        || header[40..48] != [0; 8]
    {
        return Err(invalid("journal v1 header differs").into());
    }
    let expected_header_hash = domain_hash(HEADER_HASH_DOMAIN, &header[..48]);
    if header[48..80] != expected_header_hash {
        return Err(invalid("journal header checksum differs").into());
    }

    let mut previous_hash = expected_header_hash;
    let mut offset = HEADER_BYTES;
    let mut payloads = Vec::with_capacity(8);
    let mut prefix_ends = Vec::with_capacity(8);
    for (sequence, expected_payload_bytes) in FRAME_PAYLOAD_BYTES.iter().enumerate() {
        let frame_header_end = offset
            .checked_add(FRAME_HEADER_BYTES)
            .ok_or_else(|| invalid("journal frame offset overflowed"))?;
        let frame_header = bytes
            .get(offset..frame_header_end)
            .ok_or_else(|| invalid("journal frame header is truncated"))?;
        let expected_kind = u16::try_from(sequence + 1)?;
        if &frame_header[..4] != FRAME_MAGIC
            || read_u16(&frame_header[4..6]) != 1
            || read_u16(&frame_header[6..8]) != expected_kind
            || read_u64(&frame_header[8..16]) != sequence as u64
            || frame_header[20..24] != [0; 4]
            || frame_header[24..56] != previous_hash
        {
            return Err(invalid(format!(
                "journal frame {} header or lineage differs",
                sequence + 1
            ))
            .into());
        }
        let payload_bytes = usize::try_from(read_u32(&frame_header[16..20]))?;
        if expected_payload_bytes.is_some_and(|expected| payload_bytes != expected)
            || (sequence == 0 && payload_bytes < 452)
        {
            return Err(invalid(format!(
                "journal frame {} payload width differs",
                sequence + 1
            ))
            .into());
        }
        let payload_start = frame_header_end;
        let payload_end = payload_start
            .checked_add(payload_bytes)
            .ok_or_else(|| invalid("journal payload offset overflowed"))?;
        let frame_end = payload_end
            .checked_add(FRAME_HASH_BYTES)
            .ok_or_else(|| invalid("journal checksum offset overflowed"))?;
        let payload = bytes
            .get(payload_start..payload_end)
            .ok_or_else(|| invalid("journal payload is truncated"))?;
        let recorded_hash = bytes
            .get(payload_end..frame_end)
            .ok_or_else(|| invalid("journal frame checksum is truncated"))?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(FRAME_HASH_DOMAIN);
        hasher.update(frame_header);
        hasher.update(payload);
        previous_hash = *hasher.finalize().as_bytes();
        if recorded_hash != previous_hash {
            return Err(invalid(format!("journal frame {} checksum differs", sequence + 1)).into());
        }
        payloads.push(payload_start..payload_end);
        prefix_ends.push(frame_end);
        offset = frame_end;
    }
    if offset != bytes.len() {
        return Err(invalid("Complete journal has trailing bytes or extra frames").into());
    }
    let mut run = [0; 16];
    run.copy_from_slice(&header[24..40]);
    if run == [0; 16] {
        return Err(invalid("Run identity is all zero").into());
    }
    Ok(JournalScan {
        run,
        payloads,
        prefix_ends,
    })
}

fn validate_artifact_witnesses(
    journal: &[u8],
    scan: &JournalScan,
    terrain: &[u8],
    report: &[u8],
) -> CaptureResult<()> {
    let export = &journal[scan.payloads[5].clone()];
    let report_checkpoint = &journal[scan.payloads[6].clone()];
    let complete = &journal[scan.payloads[7].clone()];
    let terrain_hash = *blake3::hash(terrain).as_bytes();
    let report_hash = domain_hash(REPORT_HASH_DOMAIN, report);
    if export[128..160] != terrain_hash
        || read_u64(&export[160..168]) != terrain.len() as u64
        || report_checkpoint[..32] != report_hash
        || read_u64(&report_checkpoint[32..40]) != report.len() as u64
        || complete[160..192] != terrain_hash
        || complete[192..224] != report_hash
    {
        return Err(invalid("Complete journal artifact witness differs").into());
    }
    let xml = std::str::from_utf8(terrain)?;
    if !xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>")
        || !xml.contains("<LandXML xmlns=\"http://www.landxml.org/schema/LandXML-1.2\"")
    {
        return Err(invalid("terrain artifact is not canonical LandXML v1.2").into());
    }
    Ok(())
}

fn validate_report(
    report: &[u8],
    run: [u8; 16],
    journal: &[u8],
    scan: &JournalScan,
) -> CaptureResult<serde_json::Map<String, serde_json::Value>> {
    let value: serde_json::Value = serde_json::from_slice(report)?;
    if value["schema"] != "punctra.terrain-workflow.audit.v1" {
        return Err(invalid("audit report schema differs").into());
    }
    for name in [
        "partner_acceptance_evaluated",
        "downstream_round_trip_evaluated",
        "human_workflow_acceptance_evaluated",
    ] {
        if value["external_evidence"][name] != false {
            return Err(invalid("capture may contain external or preference evidence").into());
        }
    }
    let identities = value["identities"]
        .as_object()
        .ok_or_else(|| invalid("audit report identities are absent"))?
        .clone();
    let intent = &journal[scan.payloads[0].clone()];
    let revision = &journal[scan.payloads[1].clone()];
    for (name, expected) in [
        ("run", encode_hex(&run)),
        ("source", encode_hex(&intent[32..64])),
        ("workspace", encode_hex(&intent[64..80])),
        ("baseline_revision", encode_hex(&intent[80..112])),
        ("operation", encode_hex(&intent[112..128])),
        ("changed_revision", encode_hex(&revision[16..48])),
    ] {
        if identities.get(name).and_then(serde_json::Value::as_str) != Some(expected.as_str()) {
            return Err(invalid(format!("audit report {name} identity differs")).into());
        }
    }
    Ok(identities)
}

fn write_manifest(
    root: &Path,
    identities: &serde_json::Map<String, serde_json::Value>,
) -> CaptureResult<()> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();
    let entries = files
        .into_iter()
        .filter(|path| path != Path::new("manifest.json"))
        .map(|relative| {
            let bytes = fs::read(root.join(&relative))?;
            let path = relative
                .to_str()
                .ok_or_else(|| invalid("fixture path is not UTF-8"))?;
            Ok(serde_json::json!({
                "path": path,
                "bytes": bytes.len(),
                "blake3": blake3::hash(&bytes).to_hex().to_string(),
            }))
        })
        .collect::<CaptureResult<Vec<_>>>()?;
    let mut manifest = serde_json::json!({
        "corpus": "punctra-terrain-demo-owner-local-run-v1",
        "generated_data": "generated 64-point LAS workflow facts and exact canonical artifacts only",
        "disk_version": 1,
        "semantic_version": 1,
        "frame_version": 1,
        "checkpoint_count": 8,
        "files": entries,
    });
    let object = manifest
        .as_object_mut()
        .ok_or_else(|| invalid("fixture manifest root is not an object"))?;
    for (manifest_name, report_name) in [
        ("run_id", "run"),
        ("operation_id", "operation"),
        ("source_id", "source"),
        ("workspace_id", "workspace"),
        ("baseline_revision", "baseline_revision"),
        ("committed_revision", "changed_revision"),
    ] {
        object.insert(
            manifest_name.into(),
            identities
                .get(report_name)
                .ok_or_else(|| invalid(format!("audit identity {report_name} is absent")))?
                .clone(),
        );
    }
    let mut bytes = serde_json::to_vec_pretty(&manifest)?;
    bytes.push(b'\n');
    fs::write(root.join("manifest.json"), bytes)?;
    Ok(())
}

fn collect_files(root: &Path, directory: &Path, files: &mut Vec<PathBuf>) -> CaptureResult<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_files(root, &path, files)?;
        } else if file_type.is_file() {
            files.push(path.strip_prefix(root)?.to_path_buf());
        } else {
            return Err(invalid("fixture corpus contains a non-regular entry").into());
        }
    }
    Ok(())
}

fn domain_hash(domain: &[u8], bytes: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(bytes);
    *hasher.finalize().as_bytes()
}

fn read_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes(bytes.try_into().expect("validated u16 field"))
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().expect("validated u32 field"))
}

fn read_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().expect("validated u64 field"))
}

fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("String writes cannot fail");
    }
    encoded
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
