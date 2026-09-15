use std::cell::Cell;
use std::os::fd::BorrowedFd;
use std::ptr;

use anyhow::{ensure, Context as _};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::reexports::rustix::mm::{mmap, munmap, MapFlags, ProtFlags};
use smithay::utils::{Physical, Rectangle, Size};

/// Owns a mapping of a sealed, fixed-size memfd. No references into it escape.
#[derive(Debug)]
pub(super) struct ShmMapping {
    pub last_commit: Cell<Option<CommitCounter>>,
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
        Ok(Self {
            address,
            len,
            last_commit: Cell::new(None),
        })
    }

    /// Only call while the producer owns the dequeued PipeWire buffer.
    pub(super) fn copy_frame(&self, bytes: &[u8]) -> anyhow::Result<()> {
        ensure!(bytes.len() == self.len, "invalid SHM frame length");
        // The source cannot alias this private mapping; no Rust references to it exist.
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), self.address.cast(), self.len) };
        Ok(())
    }

    pub(super) fn clear(&self) {
        self.last_commit.set(None);
        unsafe { ptr::write_bytes(self.address.cast::<u8>(), 0, self.len) };
    }

    pub(super) fn copy_region(
        &self,
        bytes: &[u8],
        size: Size<i32, Physical>,
        region: Rectangle<i32, Physical>,
    ) -> anyhow::Result<()> {
        ensure!(
            size.w > 0
                && size.h > 0
                && !region.is_empty()
                && Rectangle::from_size(size).contains_rect(region),
            "invalid SHM copy region"
        );
        let stride = usize::try_from(size.w)?
            .checked_mul(4)
            .context("SHM stride overflow")?;
        ensure!(
            stride.checked_mul(size.h as usize) == Some(self.len),
            "invalid SHM frame length"
        );
        if region == Rectangle::from_size(size) {
            return self.copy_frame(bytes);
        }
        let row_bytes = region.size.w as usize * 4;
        ensure!(
            row_bytes.checked_mul(region.size.h as usize) == Some(bytes.len()),
            "invalid SHM region length"
        );
        let offset = region.loc.y as usize * stride + region.loc.x as usize * 4;
        for (row, source) in bytes.chunks_exact(row_bytes).enumerate() {
            // The validated rectangle lies inside the mapping; no references into it escape.
            unsafe {
                ptr::copy_nonoverlapping(
                    source.as_ptr(),
                    self.address.cast::<u8>().add(offset + row * stride),
                    row_bytes,
                )
            };
        }
        Ok(())
    }
}

impl Drop for ShmMapping {
    fn drop(&mut self) {
        if let Err(err) = unsafe { munmap(self.address, self.len) } {
            warn!("error unmapping SHM buffer: {err}");
        }
    }
}
