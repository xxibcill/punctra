use blake3::Hasher;
use point_contracts::SourceId;

const POINT_ID_HASH_DOMAIN: &[u8] = b"punctra-point-set-ids-v1";

pub(crate) fn point_id_hasher(source: SourceId) -> Hasher {
    let mut hasher = Hasher::new();
    hasher.update(POINT_ID_HASH_DOMAIN);
    hasher.update(source.as_bytes());
    hasher
}
