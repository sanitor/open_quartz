use open_quartz::ffi::validate_gpu_texture;

#[test]
fn validates_gpu_texture_rgba_descriptor() {
    assert!(validate_gpu_texture(2, 3, 24).is_ok());
    assert!(validate_gpu_texture(0, 3, 0).is_err());
    assert!(validate_gpu_texture(2, 3, 23).is_err());
}
