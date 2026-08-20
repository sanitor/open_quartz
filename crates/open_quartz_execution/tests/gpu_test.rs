use open_quartz_execution::gpu::{align_bytes_per_row, copy_padded_rgba};

#[test]
fn aligns_copy_rows_to_webgpu_requirement() {
    assert_eq!(align_bytes_per_row(4), 256);
    assert_eq!(align_bytes_per_row(256), 256);
    assert_eq!(align_bytes_per_row(257), 512);
}

#[test]
fn strips_readback_row_padding_without_copying_padding_bytes() {
    let width: u32 = 2;
    let height: u32 = 2;
    let bytes_per_row: u32 = 256;
    let mut padded = vec![0; (bytes_per_row * height) as usize];
    padded[0..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
    let row_start = bytes_per_row as usize;
    padded[row_start..row_start + 8].copy_from_slice(&[9, 10, 11, 12, 13, 14, 15, 16]);

    assert_eq!(
        copy_padded_rgba(&padded, width, height, bytes_per_row).unwrap(),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
    );
}

#[test]
fn rejects_short_or_misaligned_readback_buffers() {
    assert!(copy_padded_rgba(&[0; 256], 2, 2, 128).is_err());
    assert!(copy_padded_rgba(&[0; 255], 2, 1, 256).is_err());
}
