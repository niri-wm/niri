use std::sync::Arc;

use smithay::backend::allocator::format::get_bpp;
use smithay::backend::allocator::Fourcc;
use smithay::utils::{Buffer, Logical, Scale, Size, Transform};

#[derive(Clone)]
pub struct MemoryBuffer {
    data: Arc<[u8]>,
    format: Fourcc,
    size: Size<i32, Buffer>,
    scale: Scale<f64>,
    transform: Transform,
}

impl MemoryBuffer {
    pub fn new(
        data: impl Into<Arc<[u8]>>,
        format: Fourcc,
        size: impl Into<Size<i32, Buffer>>,
        scale: impl Into<Scale<f64>>,
        transform: Transform,
    ) -> Self {
        let data = data.into();

        let size = size.into();
        let stride =
            size.w * (get_bpp(format).expect("Format with unknown bits per pixel") / 8) as i32;
        assert!(data.len() >= (stride * size.h) as usize);

        Self {
            data,
            format,
            size,
            scale: scale.into(),
            transform,
        }
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn format(&self) -> Fourcc {
        self.format
    }

    pub fn size(&self) -> Size<i32, Buffer> {
        self.size
    }

    pub fn scale(&self) -> Scale<f64> {
        self.scale
    }

    pub fn transform(&self) -> Transform {
        self.transform
    }

    pub fn logical_size(&self) -> Size<f64, Logical> {
        self.size.to_f64().to_logical(self.scale, self.transform)
    }
}
