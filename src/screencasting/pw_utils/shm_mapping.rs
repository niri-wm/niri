use std::os::fd::BorrowedFd;
use std::{ptr, slice};

use anyhow::{ensure, Context as _};
use smithay::reexports::rustix::mm::{mmap, munmap, MapFlags, ProtFlags};

/// Owns a mapping of a sealed, fixed-size memfd. No references into it escape.
#[derive(Debug)]
pub(super) struct ShmMapping {
    address: *mut std::ffi::c_void,
    len: usize,
}

impl ShmMapping {
    /// # Safety
    ///
    /// The file must contain at least `len` initialized bytes and must not shrink
    /// while mapped. Access through other mappings must be synchronized with
    /// calls to `copy_frame` and `clear` for the lifetime of this mapping.
    pub(super) unsafe fn new(fd: BorrowedFd<'_>, len: usize) -> anyhow::Result<Self> {
        ensure!(
            len > 0 && len <= isize::MAX as usize,
            "invalid mapping length"
        );
        let address = unsafe {
            mmap(
                ptr::null_mut(),
                len,
                ProtFlags::READ | ProtFlags::WRITE,
                MapFlags::SHARED,
                fd,
                0,
            )
        }
        .context("error mapping SHM buffer")?;
        Ok(Self { address, len })
    }

    pub(super) fn copy_frame(&self, bytes: &[u8]) {
        let _span = tracy_client::span!();

        // SAFETY: The constructor's contract keeps the memory valid and ensures
        // exclusive access during this write. No references into the mapping escape.
        let buffer = unsafe { slice::from_raw_parts_mut(self.address.cast::<u8>(), self.len) };
        buffer.copy_from_slice(bytes);
    }

    pub(super) fn clear(&self) {
        let _span = tracy_client::span!();

        // SAFETY: As in copy_frame, the mapping is valid and exclusively accessed.
        let buffer = unsafe { slice::from_raw_parts_mut(self.address.cast::<u8>(), self.len) };
        buffer.fill(0);
    }
}

impl Drop for ShmMapping {
    fn drop(&mut self) {
        if let Err(err) = unsafe { munmap(self.address, self.len) } {
            warn!("error unmapping SHM buffer: {err}");
        }
    }
}
