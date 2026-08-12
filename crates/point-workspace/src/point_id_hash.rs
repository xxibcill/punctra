use blake3::Hasher;
use point_contracts::{ContentHash, SourceId};

const POINT_ID_HASH_DOMAIN: &[u8] = b"punctra-point-set-ids-v1";

pub(crate) struct CanonicalPointIdHasher(Hasher);

impl CanonicalPointIdHasher {
    pub(crate) fn new(source: SourceId) -> Self {
        let mut hasher = Hasher::new();
        hasher.update(POINT_ID_HASH_DOMAIN);
        hasher.update(source.as_bytes());
        Self(hasher)
    }

    pub(crate) fn update(&mut self, ordinal: u64) {
        self.0.update(&ordinal.to_le_bytes());
    }

    pub(crate) fn finalize(&self) -> ContentHash {
        ContentHash::new(*self.0.finalize().as_bytes())
    }
}
