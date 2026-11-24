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
    rgba.resize(len, 255);
    // Leave the RGB white even outside the glyph so filtering preserves straight alpha.
    for pixel in rgba.chunks_exact_mut(4) {
        pixel[3] = 0;
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
        let block_height = buffer
            .layout_runs()
            .last()
            .map_or(0.0, |line| line.line_top + line.line_height);
        let offset_x = margin_x.round() as i32;
        let offset_y = ((height as f32 - block_height) / 2.0).round() as i32;
        buffer.draw(
            fonts,
            &mut cache,
            Color::rgb(255, 255, 255),
            |x, y, w, h, color| {
                for row in (y + offset_y).max(0)..(y + offset_y + h as i32).min(height as i32) {
                    for col in (x + offset_x).max(0)..(x + offset_x + w as i32).min(width as i32) {
                        let dest =
                            &mut rgba[(row as usize * width as usize + col as usize) * 4 + 3];
                        let uncovered = u16::from(255 - *dest);
                        *dest += ((uncovered * u16::from(color.a()) + 127) / 255) as u8;
                    }
                }
            },
        );
    });
    Ok(rgba)
}
