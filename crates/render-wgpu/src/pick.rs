use std::sync::{Arc, Mutex, mpsc};

use render_protocol::{BatchKey, BatchVersion, FrameKey, PointId};
use thiserror::Error;

use crate::pipeline::PICK_FORMAT;

pub(crate) const PICK_READBACK_ROW_BYTES: u64 = 256;
pub(crate) const PICK_TOKEN_BYTES: u64 = 4;

/// One physical viewport pixel requested for provisional point picking.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PickRequest {
    pixel: [u32; 2],
}

impl PickRequest {
    /// Creates a request in top-left-origin physical pixel coordinates.
    #[must_use]
    pub const fn new(pixel: [u32; 2]) -> Self {
        Self { pixel }
    }

    /// Returns the physical pixel coordinate.
    #[must_use]
    pub const fn pixel(self) -> [u32; 2] {
        self.pixel
    }
}

/// Stable caller metadata associated with one provisional GPU pick.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PickHit {
    frame: FrameKey,
    batch: BatchKey,
    version: BatchVersion,
    point: PointId,
}

impl PickHit {
    /// Returns the View generation that produced the hit.
    #[must_use]
    pub const fn frame(self) -> FrameKey {
        self.frame
    }

    /// Returns the producing point batch.
    #[must_use]
    pub const fn batch(self) -> BatchKey {
        self.batch
    }

    /// Returns the exact producing batch version.
    #[must_use]
    pub const fn version(self) -> BatchVersion {
        self.version
    }

    /// Returns the caller's stable point identity.
    #[must_use]
    pub const fn point(self) -> PointId {
        self.point
    }
}

/// The nonblocking state of one pick ticket.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PickPoll {
    /// The host must submit and continue polling its wgpu device.
    Pending,
    /// The GPU completed; `None` means that no resident splat covered the pixel.
    Ready(Option<PickHit>),
}

/// A nonblocking readback owned until one pick result is consumed.
pub struct PickTicket {
    frame: FrameKey,
    readback: wgpu::Buffer,
    receiver: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    table: Arc<PickTable>,
    completed: bool,
}

impl PickTicket {
    pub(crate) fn new(
        frame: FrameKey,
        readback: wgpu::Buffer,
        receiver: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
        table: Arc<PickTable>,
    ) -> Self {
        Self {
            frame,
            readback,
            receiver,
            table,
            completed: false,
        }
    }

    /// Returns the View generation captured by this ticket.
    #[must_use]
    pub const fn frame(&self) -> FrameKey {
        self.frame
    }

    /// Checks for completion without blocking.
    ///
    /// The host must submit the encoder containing the pick and drive normal
    /// wgpu device polling. A ticket yields exactly one ready result.
    ///
    /// # Errors
    ///
    /// Returns a mapping, synchronization, or token-table error. Calling this
    /// method after a ready result returns [`PickError::AlreadyCompleted`].
    pub fn poll(&mut self) -> Result<PickPoll, PickError> {
        if self.completed {
            return Err(PickError::AlreadyCompleted);
        }

        match self.receiver.try_recv() {
            Ok(Ok(())) => self.read_result(),
            Ok(Err(error)) => {
                self.completed = true;
                Err(PickError::BufferMapping(error))
            }
            Err(mpsc::TryRecvError::Empty) => Ok(PickPoll::Pending),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.completed = true;
                Err(PickError::MappingCallbackDropped)
            }
        }
    }

    fn read_result(&mut self) -> Result<PickPoll, PickError> {
        self.completed = true;
        let bytes = match self.readback.get_mapped_range(0..PICK_TOKEN_BYTES) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.readback.unmap();
                return Err(PickError::BufferAccess(error));
            }
        };
        let token = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        drop(bytes);
        self.readback.unmap();

        if token == 0 {
            return Ok(PickPoll::Ready(None));
        }
        let hit = self.table.lookup(token)?;
        Ok(PickPoll::Ready(Some(hit)))
    }
}

/// A pick request or asynchronous readback error.
#[derive(Debug, Error)]
pub enum PickError {
    /// The GPU rejected the deferred mapping.
    #[error("pick readback mapping failed: {0}")]
    BufferMapping(wgpu::BufferAsyncError),
    /// The mapped range could not be accessed.
    #[error("pick readback range access failed: {0}")]
    BufferAccess(wgpu::MapRangeError),
    /// The mapping callback channel closed without a result.
    #[error("pick mapping callback ended without a result")]
    MappingCallbackDropped,
    /// A successful ticket was polled more than once.
    #[error("pick ticket result was already consumed")]
    AlreadyCompleted,
    /// The GPU returned a token absent from the captured generation table.
    #[error("pick token {token} is absent from frame {frame:?}")]
    UnknownToken {
        /// The returned nonzero token.
        token: u32,
        /// The captured frame.
        frame: FrameKey,
    },
    /// One View generation exceeded the nonzero 32-bit pick-token space.
    #[error("active View exhausted its 32-bit point-picking token space")]
    TokenSpaceExhausted,
    /// Internal pick metadata was left poisoned by a panic.
    #[error("pick metadata is unavailable after a prior panic")]
    MetadataPoisoned,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PickRecord {
    pub(crate) batch: BatchKey,
    pub(crate) version: BatchVersion,
    pub(crate) point: PointId,
}

pub(crate) struct PickTable {
    frame: FrameKey,
    records: Mutex<Vec<PickRecord>>,
}

impl PickTable {
    pub(crate) fn new(frame: FrameKey) -> Self {
        Self {
            frame,
            records: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn append(
        &self,
        records: impl IntoIterator<Item = PickRecord>,
    ) -> Result<Vec<u32>, PickError> {
        let records: Vec<_> = records.into_iter().collect();
        if records.is_empty() {
            return Ok(Vec::new());
        }
        let mut table = self
            .records
            .lock()
            .map_err(|_| PickError::MetadataPoisoned)?;
        let final_len = table
            .len()
            .checked_add(records.len())
            .ok_or(PickError::TokenSpaceExhausted)?;
        u32::try_from(final_len).map_err(|_| PickError::TokenSpaceExhausted)?;
        let first_token = u32::try_from(table.len())
            .map_err(|_| PickError::TokenSpaceExhausted)?
            .checked_add(1)
            .ok_or(PickError::TokenSpaceExhausted)?;
        let last_token = u32::try_from(final_len).map_err(|_| PickError::TokenSpaceExhausted)?;
        let tokens = (first_token..=last_token).collect();
        table.extend(records);
        Ok(tokens)
    }

    fn lookup(&self, token: u32) -> Result<PickHit, PickError> {
        let table = self
            .records
            .lock()
            .map_err(|_| PickError::MetadataPoisoned)?;
        let index = token
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
            .ok_or(PickError::UnknownToken {
                token,
                frame: self.frame,
            })?;
        let record = table.get(index).ok_or(PickError::UnknownToken {
            token,
            frame: self.frame,
        })?;
        Ok(PickHit {
            frame: self.frame,
            batch: record.batch,
            version: record.version,
            point: record.point,
        })
    }
}

pub(crate) struct PickTarget {
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) viewport: [u32; 2],
}

impl PickTarget {
    pub(crate) fn new(device: &wgpu::Device, viewport: [u32; 2]) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("punctra pick texture"),
            size: wgpu::Extent3d {
                width: viewport[0],
                height: viewport[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: PICK_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            viewport,
        }
    }
}

#[cfg(test)]
mod tests {
    use render_protocol::ViewId;

    use super::*;

    #[test]
    fn tokens_preserve_zero_and_duplicate_caller_identities() {
        let frame = FrameKey::new(ViewId::new(4), 2);
        let table = PickTable::new(frame);
        let records = [
            PickRecord {
                batch: BatchKey::new(10),
                version: BatchVersion::new(1),
                point: PointId::new(0),
            },
            PickRecord {
                batch: BatchKey::new(11),
                version: BatchVersion::new(7),
                point: PointId::new(0),
            },
        ];

        let tokens = table.append(records).unwrap();

        assert_eq!(tokens, [1, 2]);
        let first = table.lookup(tokens[0]).unwrap();
        let second = table.lookup(tokens[1]).unwrap();
        assert_eq!(first.point(), PointId::new(0));
        assert_eq!(first.batch(), BatchKey::new(10));
        assert_eq!(second.point(), PointId::new(0));
        assert_eq!(second.batch(), BatchKey::new(11));
        assert!(matches!(
            table.lookup(0),
            Err(PickError::UnknownToken { token: 0, .. })
        ));
        assert!(table.append([]).unwrap().is_empty());
    }
}
