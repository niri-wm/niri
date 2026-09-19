use std::os::fd::BorrowedFd;

use anyhow::{ensure, Context as _};
use arrayvec::ArrayVec;
use smithay::backend::allocator::dmabuf::{Dmabuf, MAX_PLANES};
use smithay::reexports::rustix::fs::{seek, SeekFrom};

/// Includes padding and auxiliary storage that cannot be inferred from image dimensions.
fn backing_size(fd: BorrowedFd<'_>, offset: u32) -> anyhow::Result<u32> {
    let size = seek(fd, SeekFrom::End(0)).context("error querying DMA-BUF size")?;
    seek(fd, SeekFrom::Start(0)).context("error resetting DMA-BUF position")?;
    let size = u32::try_from(size).context("DMA-BUF exceeds SPA size range")?;
    ensure!(
        offset < size,
        "DMA-BUF plane offset exceeds backing storage"
    );
    Ok(size)
}

pub(super) fn plane_sizes(dmabuf: &Dmabuf) -> anyhow::Result<ArrayVec<u32, MAX_PLANES>> {
    dmabuf
        .handles()
        .zip(dmabuf.offsets())
        .zip(dmabuf.strides())
        .map(|((fd, offset), stride)| {
            i32::try_from(stride).context("DMA-BUF stride exceeds SPA i32")?;
            backing_size(fd, offset)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::os::fd::AsFd;

    use smithay::backend::allocator::dmabuf::DmabufFlags;
    use smithay::backend::allocator::Fourcc;
    use smithay::reexports::gbm::Modifier;
    use smithay::reexports::rustix::fs::{ftruncate, memfd_create, MemfdFlags};

    use super::*;

    #[test]
    fn backing_size_preserves_padding_and_checks_offset() {
        let fd = memfd_create("dma-layout-test", MemfdFlags::CLOEXEC).unwrap();
        ftruncate(&fd, 8192).unwrap();
        assert_eq!(backing_size(fd.as_fd(), 4096).unwrap(), 8192);
        assert!(backing_size(fd.as_fd(), 8192).is_err());
        ftruncate(&fd, u64::from(u32::MAX) + 1).unwrap();
        assert!(backing_size(fd.as_fd(), 0).is_err());
    }

    #[test]
    fn validates_all_planes_before_publishing_layout() {
        for invalid in [false, true] {
            let mut builder = Dmabuf::builder(
                (16, 8),
                Fourcc::Argb8888,
                Modifier::Linear,
                DmabufFlags::empty(),
            );
            for index in 0..2 {
                let fd = memfd_create("dma-planes-test", MemfdFlags::CLOEXEC).unwrap();
                ftruncate(&fd, 4096).unwrap();
                assert!(builder.add_plane(fd, if invalid && index == 1 { 4096 } else { 128 }, 64));
            }
            let dmabuf = builder.build().unwrap();
            let sizes = plane_sizes(&dmabuf);
            if invalid {
                assert!(sizes.is_err());
            } else {
                assert_eq!(sizes.unwrap().as_slice(), &[4096, 4096]);
            }
        }
    }
}
