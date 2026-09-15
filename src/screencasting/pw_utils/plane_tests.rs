use super::*;

#[test]
fn rejected_buffer_clears_all_planes_and_can_be_rendered_again() {
    let layouts = [(128, 1024, 8192), (4096, 256, 6144)];
    let mut chunks = layouts.map(|(offset, stride, maxsize)| spa_chunk {
        offset,
        size: maxsize - offset,
        stride,
        flags: SPA_CHUNK_FLAG_NONE as i32,
    });
    let mut data: [spa_data; 2] = unsafe { mem::zeroed() };
    for ((plane, chunk), (_, _, maxsize)) in data.iter_mut().zip(&mut chunks).zip(layouts) {
        plane.chunk = chunk;
        plane.maxsize = maxsize;
    }
    let mut spa: spa_buffer = unsafe { mem::zeroed() };
    spa.n_datas = 2;
    spa.datas = data.as_mut_ptr();
    let mut buffer: pw_buffer = unsafe { mem::zeroed() };
    buffer.buffer = &mut spa;
    unsafe {
        mark_buffer_corrupted(NonNull::from(&mut buffer));
    }
    for (chunk, (offset, stride, _)) in chunks.iter().zip(layouts) {
        assert_eq!(chunk.size, 0);
        assert_eq!(chunk.flags, SPA_CHUNK_FLAG_CORRUPTED as i32);
        assert_eq!((chunk.offset, chunk.stride), (offset, stride));
    }
    let mut sequence = 0;
    unsafe {
        mark_buffer_as_good(NonNull::from(&mut buffer), &mut sequence, SharingBuf::Dma);
    }
    for (chunk, (offset, stride, maxsize)) in chunks.iter().zip(layouts) {
        assert_eq!(chunk.size, maxsize - offset);
        assert_eq!(chunk.flags, SPA_CHUNK_FLAG_NONE as i32);
        assert_eq!((chunk.offset, chunk.stride), (offset, stride));
    }
    assert_eq!(sequence, 1);
}
