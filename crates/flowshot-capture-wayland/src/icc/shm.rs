//! Anonymous shared-memory capture buffers: `memfd` + `wl_shm` pool +
//! `wl_buffer`, with pixel readback through the file descriptor.
//!
//! v1 captures exclusively into `wl_shm` buffers (no dma-buf path). The
//! backing memory is an anonymous file (`memfd_create`), sized with
//! `ftruncate`, handed to the compositor through `wl_shm.create_pool` (the
//! descriptor is duplicated into the socket at send time; the local file stays
//! open for readback). Pixels are read with ordinary file I/O after the frame's
//! `ready` event - both mappings go through the same page cache, so no memory
//! mapping (and no `unsafe`) is needed on the client side.
//!
//! The allocation and readback are shared by every `wl_shm` backend
//! (`ext-image-copy-capture-v1` and `wlr-screencopy-v1`); the `_with` methods
//! are generic over the backend error ([`BackendError`]) and the bare-named
//! wrappers pin it to [`IccError`] for the ICC and cursor runners.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::fd::AsFd;

use nix::sys::memfd::{MemFdCreateFlag, memfd_create};
use nix::unistd::ftruncate;
use wayland_client::QueueHandle;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_shm::WlShm;
use wayland_client::protocol::wl_shm_pool::WlShmPool;

use super::protocol::BufferParams;
use crate::error::{BackendError, IccError};
use crate::session::CaptureState;

/// Name of the anonymous capture file, visible in `/proc/<pid>/fd`.
const MEMFD_NAME: &std::ffi::CStr = c"flowshot-capture";

/// One capture's shared-memory buffer and its Wayland handles.
///
/// Dropping the buffer destroys the `wl_buffer` and the `wl_shm_pool`
/// (one-shot lifecycle: every capture allocates fresh), then closes the
/// anonymous file.
#[derive(Debug)]
pub(crate) struct ShmBuffer {
    file: File,
    pool: WlShmPool,
    buffer: WlBuffer,
    params: BufferParams,
}

impl ShmBuffer {
    /// [`ShmBuffer::allocate_with`] pinned to [`IccError`].
    pub(crate) fn allocate(
        shm: &WlShm,
        qh: &QueueHandle<CaptureState>,
        params: &BufferParams,
    ) -> Result<Self, IccError> {
        Self::allocate_with::<IccError>(shm, qh, params)
    }

    /// Creates the anonymous file, sizes it, and registers pool and buffer with
    /// the compositor.
    ///
    /// # Errors
    ///
    /// The backend's `Io` error when `memfd_create`/`ftruncate` fail, and its
    /// `Internal` error when the negotiated geometry does not fit the
    /// protocol's `i32` wire fields.
    pub(crate) fn allocate_with<E>(
        shm: &WlShm,
        qh: &QueueHandle<CaptureState>,
        params: &BufferParams,
    ) -> Result<Self, E>
    where
        E: BackendError,
    {
        let fd =
            memfd_create(MEMFD_NAME, MemFdCreateFlag::empty()).map_err(std::io::Error::from)?;
        let size = i64::try_from(params.size_bytes)
            .map_err(|_| E::internal("buffer size overflows the wire field"))?;
        ftruncate(&fd, size).map_err(std::io::Error::from)?;
        let file = File::from(fd);
        let size_i32 = i32::try_from(params.size_bytes)
            .map_err(|_| E::internal("buffer size overflows the pool wire field"))?;
        let (width, height, stride) = (
            i32::try_from(params.width).map_err(|_| E::internal("width overflow"))?,
            i32::try_from(params.height).map_err(|_| E::internal("height overflow"))?,
            i32::try_from(params.stride).map_err(|_| E::internal("stride overflow"))?,
        );
        let pool = shm.create_pool(file.as_fd(), size_i32, qh, ());
        let buffer = pool.create_buffer(0, width, height, stride, params.shm_format, qh, ());
        Ok(Self {
            file,
            pool,
            buffer,
            params: *params,
        })
    }

    /// The `wl_buffer` to attach to a capture frame.
    pub(crate) fn buffer(&self) -> &WlBuffer {
        &self.buffer
    }

    /// [`ShmBuffer::read_pixels_with`] pinned to [`IccError`].
    pub(crate) fn read_pixels(&self) -> Result<Vec<u8>, IccError> {
        self.read_pixels_with::<IccError>()
    }

    /// Reads the whole buffer back from the anonymous file.
    ///
    /// Valid after the frame's `ready` event: the compositor wrote the pixels
    /// through its own mapping of the same page-cache pages.
    ///
    /// # Errors
    ///
    /// The backend's `Io` error when the seek or read fails.
    pub(crate) fn read_pixels_with<E>(&self) -> Result<Vec<u8>, E>
    where
        E: BackendError,
    {
        let mut data = vec![0u8; self.params.size_bytes];
        let mut file = &self.file;
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut data)?;
        Ok(data)
    }
}

impl Drop for ShmBuffer {
    fn drop(&mut self) {
        self.buffer.destroy();
        self.pool.destroy();
    }
}
