use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LetterboxTensor {
    pub tensor: Vec<f32>,
    pub shape: [usize; 4],
    pub scale: f32,
    pub pad_x: f32,
    pub pad_y: f32,
    pub resized_width: u32,
    pub resized_height: u32,
}

pub fn rgba_to_chw(rgba: &[u8], width: u32, height: u32) -> Result<Vec<f32>, String> {
    validate_rgba(rgba, width, height)?;
    let pixels = width as usize * height as usize;
    let mut tensor = vec![0.0; pixels * 3];
    for index in 0..pixels {
        tensor[index] = rgba[index * 4] as f32 / 255.0;
        tensor[pixels + index] = rgba[index * 4 + 1] as f32 / 255.0;
        tensor[pixels * 2 + index] = rgba[index * 4 + 2] as f32 / 255.0;
    }
    Ok(tensor)
}

pub fn letterbox_preprocess(
    rgba: &[u8],
    source_width: u32,
    source_height: u32,
    target_size: u32,
) -> Result<LetterboxTensor, String> {
    validate_rgba(rgba, source_width, source_height)?;
    if target_size == 0 {
        return Err("target_size must be positive".to_owned());
    }
    let scale =
        (target_size as f32 / source_width as f32).min(target_size as f32 / source_height as f32);
    let resized_width = (source_width as f32 * scale).round() as u32;
    let resized_height = (source_height as f32 * scale).round() as u32;
    let pad_x = (target_size - resized_width) as f32 / 2.0;
    let pad_y = (target_size - resized_height) as f32 / 2.0;
    let pixels = target_size as usize * target_size as usize;
    let fill = 114.0 / 255.0;
    let mut tensor = vec![fill; pixels * 3];
    let dst_x = pad_x.round() as u32;
    let dst_y = pad_y.round() as u32;

    for y in 0..resized_height {
        let source_y = ((y as f32 + 0.5) / scale - 0.5)
            .round()
            .clamp(0.0, source_height.saturating_sub(1) as f32) as u32;
        for x in 0..resized_width {
            let source_x = ((x as f32 + 0.5) / scale - 0.5)
                .round()
                .clamp(0.0, source_width.saturating_sub(1) as f32)
                as u32;
            let source = (source_y as usize * source_width as usize + source_x as usize) * 4;
            let destination =
                (dst_y as usize + y as usize) * target_size as usize + dst_x as usize + x as usize;
            tensor[destination] = rgba[source] as f32 / 255.0;
            tensor[pixels + destination] = rgba[source + 1] as f32 / 255.0;
            tensor[pixels * 2 + destination] = rgba[source + 2] as f32 / 255.0;
        }
    }

    Ok(LetterboxTensor {
        tensor,
        shape: [1, 3, target_size as usize, target_size as usize],
        scale,
        pad_x,
        pad_y,
        resized_width,
        resized_height,
    })
}

pub fn midas_preprocess(rgba: &[u8], width: u32, height: u32) -> Result<Vec<f32>, String> {
    validate_rgba(rgba, width, height)?;
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.229, 0.224, 0.225];
    let pixels = width as usize * height as usize;
    let mut tensor = vec![0.0; pixels * 3];
    for index in 0..pixels {
        let red = rgba[index * 4] as f32 / 255.0;
        let green = rgba[index * 4 + 1] as f32 / 255.0;
        let blue = rgba[index * 4 + 2] as f32 / 255.0;
        tensor[index] = (blue - MEAN[0]) / STD[0];
        tensor[pixels + index] = (green - MEAN[1]) / STD[1];
        tensor[pixels * 2 + index] = (red - MEAN[2]) / STD[2];
    }
    Ok(tensor)
}

pub fn rgb_output_to_rgba(output: &[f32], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let pixels = width as usize * height as usize;
    if output.len() < pixels * 3 {
        return Err(format!(
            "RGB tensor has {} values, expected at least {}",
            output.len(),
            pixels * 3
        ));
    }
    let mut rgba = vec![0; pixels * 4];
    for index in 0..pixels {
        rgba[index * 4] = float_to_byte(output[index]);
        rgba[index * 4 + 1] = float_to_byte(output[pixels + index]);
        rgba[index * 4 + 2] = float_to_byte(output[pixels * 2 + index]);
        rgba[index * 4 + 3] = 255;
    }
    Ok(rgba)
}

pub fn apply_alpha_mask(
    source_rgba: &[u8],
    mask: &[f32],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    validate_rgba(source_rgba, width, height)?;
    let pixels = width as usize * height as usize;
    if mask.len() < pixels {
        return Err(format!(
            "alpha mask has {} values, expected at least {pixels}",
            mask.len()
        ));
    }
    let mut output = source_rgba.to_vec();
    for index in 0..pixels {
        output[index * 4 + 3] = float_to_byte(mask[index]);
    }
    Ok(output)
}

fn validate_rgba(rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("image dimensions must be positive".to_owned());
    }
    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        return Err(format!(
            "RGBA byte length {} does not match {width}x{height} image",
            rgba.len()
        ));
    }
    Ok(())
}

fn float_to_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}
