use open_quartz::gpu::{BLIT_FRAG, FULLSCREEN_VERT_WITH_UV};

#[test]
fn fullscreen_shader_is_a_three_vertex_triangle_with_uv() {
    assert!(FULLSCREEN_VERT_WITH_UV.contains("@builtin(vertex_index)"));
    assert!(FULLSCREEN_VERT_WITH_UV.contains("vertex_index"));
    assert!(FULLSCREEN_VERT_WITH_UV.contains("@location(0) v_uv"));
}

#[test]
fn blit_shader_samples_a_texture() {
    assert!(BLIT_FRAG.contains("texture_2d<f32>"));
    assert!(BLIT_FRAG.contains("textureSample(tex, samp, v_uv)"));
}
