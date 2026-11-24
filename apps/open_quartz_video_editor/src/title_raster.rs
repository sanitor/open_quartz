use crate::document::Title;
use cosmic_text::{Align, Attrs, Buffer, Color, FontSystem, Metrics, Shaping, SwashCache, Wrap};
use std::cell::RefCell;

thread_local! {
    static FONTS: RefCell<FontSystem> = RefCell::new(FontSystem::new());
}

pub fn rasterize(title: &Title, width: u32, height: u32) -> Result<Vec<u8>, String> {
    let len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or("Title image is too large")?;
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(len)
        .map_err(|_| "Cannot allocate title image")?;
    rgba.resize(len, 0);
    if !title.subtitle {
        // White outside the glyph keeps straight-alpha title edges white when sampled.
        for pixel in rgba.chunks_exact_mut(4) {
            pixel[..3].fill(255);
        }
    }

    FONTS.with(|fonts| {
        let fonts = &mut *fonts.borrow_mut();
        let mut cache = SwashCache::new();
        let font_size = (height as f32 * f32::from(title.size_percent) / 100.0).max(1.0);
        let margin_x = width as f32 * 0.05;
        let mut buffer = Buffer::new(fonts, Metrics::new(font_size, font_size * 1.2));
        buffer.set_size(
            fonts,
            Some(width as f32 - 2.0 * margin_x),
            Some(height as f32 * 0.9),
        );
        buffer.set_wrap(fonts, Wrap::WordOrGlyph);
        buffer.set_text(
            fonts,
            &title.text,
            &Attrs::new(),
            Shaping::Advanced,
            Some(Align::Center),
        );
        let (block_width, block_height) = buffer
            .layout_runs()
            .fold((0.0_f32, 0.0_f32), |(width, _), line| {
                (width.max(line.line_w), line.line_top + line.line_height)
            });
        let offset_x = margin_x.round() as i32;
        let top = if title.subtitle {
            (height as f32 * 0.92 - block_height).max(0.0)
        } else {
            (height as f32 - block_height) / 2.0
        };
        let offset_y = top.round() as i32;
        buffer.draw(
            fonts,
            &mut cache,
            Color::rgb(255, 255, 255),
            |x, y, w, h, color| {
                for row in (y + offset_y).max(0)..(y + offset_y + h as i32).min(height as i32) {
                    for col in (x + offset_x).max(0)..(x + offset_x + w as i32).min(width as i32) {
                        let index = (row as usize * width as usize + col as usize) * 4;
                        let pixel = &mut rgba[index..index + 4];
                        let uncovered = u16::from(255 - pixel[3]);
                        pixel[3] += ((uncovered * u16::from(color.a()) + 127) / 255) as u8;
                        if pixel[3] > 0 {
                            pixel[..3].fill(255);
                        }
                    }
                }
            },
        );
        if title.subtitle && block_width > 0.0 {
            let padding = font_size * 0.25;
            let left = ((width as f32 - block_width) / 2.0 - padding)
                .floor()
                .max(0.0) as usize;
            let right = ((width as f32 + block_width) / 2.0 + padding)
                .ceil()
                .min(width as f32) as usize;
            let first_row = (top - padding).floor().max(0.0) as usize;
            let last_row = (top + block_height + padding).ceil().min(height as f32) as usize;
            for row in first_row..last_row {
                let start = (row * width as usize + left) * 4;
                let end = (row * width as usize + right) * 4;
                for pixel in rgba[start..end].chunks_exact_mut(4) {
                    let glyph = f32::from(pixel[3]) / 255.0;
                    let alpha = glyph + 0.75 * (1.0 - glyph);
                    let linear = glyph / alpha;
                    let gray = if linear <= 0.0031308 {
                        12.92 * linear
                    } else {
                        1.055 * linear.powf(1.0 / 2.4) - 0.055
                    };
                    pixel[..3].fill((gray * 255.0).round() as u8);
                    pixel[3] = (alpha * 255.0).round() as u8;
                }
            }
        }
    });
    Ok(rgba)
}
