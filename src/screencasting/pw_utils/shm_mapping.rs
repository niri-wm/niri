use std::os::fd::BorrowedFd;
use std::ptr;

use anyhow::{ensure, Context as _};
use smithay::reexports::rustix::mm::{mmap, munmap, MapFlags, ProtFlags};

/// Owns a mapping of a sealed, fixed-size memfd. No references into it escape.
#[derive(Debug)]
pub(super) struct ShmMapping {
    address: *mut std::ffi::c_void,
    len: usize,
}

impl ShmMapping {
    pub(super) fn new(fd: BorrowedFd<'_>, len: usize) -> anyhow::Result<Self> {
        ensure!(
            len > 0 && len <= isize::MAX as usize,
            "invalid mapping length"
        );
        // The caller seals the file against shrinking before creating this mapping.
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

    /// Only call while the producer owns the dequeued PipeWire buffer.
    pub(super) fn copy_frame(&self, bytes: &[u8]) -> anyhow::Result<()> {
        ensure!(bytes.len() == self.len, "invalid SHM frame length");
        // The source cannot alias this private mapping; no Rust references to it exist.
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), self.address.cast(), self.len) };
        Ok(())
    }

    pub(super) fn clear(&self) {
        unsafe { ptr::write_bytes(self.address.cast::<u8>(), 0, self.len) };
    }
}

impl Drop for ShmMapping {
    fn drop(&mut self) {
        if let Err(err) = unsafe { munmap(self.address, self.len) } {
            warn!("error unmapping SHM buffer: {err}");
        }
    }
}
