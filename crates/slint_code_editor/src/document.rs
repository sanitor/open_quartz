use crate::{
    syntax::{self, Highlight, Language},
    theme::Theme,
};
use cosmic_text::{
    Action, Attrs, AttrsList, Buffer, Change, Color, Cursor, Edit, Editor, Family, FontSystem,
    Metrics, Motion, Selection, Shaping, SwashCache, Wrap,
};
use unicode_segmentation::UnicodeSegmentation;

struct History {
    change: Change,
    before: (Cursor, Selection),
    after: (Cursor, Selection),
}

/// Owns the only editable document, selection and history. Styles are derived.
pub struct Document {
    editor: Editor<'static>,
    fonts: FontSystem,
    cache: SwashCache,
    snapshot: slint::SharedString,
    undo: Vec<History>,
    redo: Vec<History>,
    language: Language,
    theme: Theme,
    provider: Option<Box<dyn Fn(&str) -> Vec<Highlight>>>,
    visual_cursor: (i32, i32),
    scale: f32,
    pub read_only: bool,
    pub focused: bool,
    pub preedit: String,
    preedit_selection: Option<(usize, usize)>,
    pub origin: (f32, f32),
    clipboard: Option<arboard::Clipboard>,
    pub clipboard_error: Option<String>,
}

impl Default for Document {
    fn default() -> Self {
        use cosmic_text::skrifa::raw::TableProvider as _;
        let (locale, mut db) = FontSystem::new().into_locale_and_db();
        // cosmic-text divides shaping metrics by head.units_per_em. Legacy
        // bitmap-only macOS fonts expose bhed instead of head and yield 0,
        // producing infinite advances. Select scalable fonts before fallback.
        let unsupported = db
            .faces()
            .filter_map(|face| {
                let scalable = db
                    .with_face_data(face.id, |data, index| {
                        cosmic_text::skrifa::FontRef::from_index(data, index)
                            .ok()
                            .and_then(|f| f.head().ok())
                            .is_some_and(|head| head.units_per_em() > 0)
                    })
                    .unwrap_or(false);
                (!scalable).then_some(face.id)
            })
            .collect::<Vec<_>>();
        for id in unsupported {
            db.remove_face(id);
        }
        let mut fonts = FontSystem::new_with_locale_and_db(locale, db);
        let mut buffer = Buffer::new(&mut fonts, Metrics::new(14.0, 21.0));
        buffer.set_wrap(&mut fonts, Wrap::None);
        buffer.set_size(&mut fonts, Some(640.0), Some(400.0));
        buffer.set_text(
            &mut fonts,
            "",
            &Attrs::new().family(Family::Monospace),
            Shaping::Advanced,
            None,
        );
        let mut editor = Editor::new(buffer);
        editor.set_auto_indent(true);
        Self {
            editor,
            fonts,
            cache: SwashCache::new(),
            snapshot: slint::SharedString::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            language: Language::Plain,
            theme: Theme::default(),
            provider: None,
            visual_cursor: (0, 0),
            scale: 1.0,
            read_only: false,
            focused: false,
            preedit: String::new(),
            preedit_selection: None,
            origin: (0.0, 0.0),
            clipboard: None,
            clipboard_error: None,
        }
    }
}

impl Document {
    pub fn text(&self) -> slint::SharedString {
        self.snapshot.clone()
    }

    fn publish_snapshot(&mut self) {
        self.snapshot = self.editor.with_buffer(|buffer| {
            let length = buffer
                .lines
                .iter()
                .map(|l| l.text().len() + l.ending().as_str().len())
                .sum();
            let mut text = String::with_capacity(length);
            for line in &buffer.lines {
                text.push_str(line.text());
                text.push_str(line.ending().as_str());
            }
            text.into()
        });
    }

    pub fn cursor_offset(&self) -> usize {
        let cursor = self.editor.cursor();
        self.editor.with_buffer(|b| {
            b.lines[..cursor.line]
                .iter()
                .map(|l| l.text().len() + l.ending().as_str().len())
                .sum::<usize>()
                + cursor.index
        })
    }

    /// External replacement is a new document, not a synthetic user edit.
    pub fn set_text(&mut self, text: &str) {
        if self.text() == text {
            return;
        }
        self.preedit.clear();
        self.editor.with_buffer_mut(|b| {
            b.set_text(
                &mut self.fonts,
                text,
                &Attrs::new().family(Family::Monospace),
                Shaping::Advanced,
                None,
            )
        });
        self.editor.set_cursor(Cursor::default());
        self.editor.set_selection(Selection::None);
        self.undo.clear();
        self.redo.clear();
        self.snapshot = text.into();
        self.restyle();
    }

    pub fn set_language(&mut self, language: Language) {
        if self.language != language {
            self.language = language;
            self.restyle();
        }
    }

    pub fn set_theme(&mut self, theme: Theme) {
        if self.theme != theme {
            self.theme = theme;
            self.restyle();
        }
    }

    pub fn set_provider(&mut self, provider: Option<Box<dyn Fn(&str) -> Vec<Highlight>>>) {
        self.provider = provider;
        self.restyle();
    }

    fn restyle(&mut self) {
        let text = self.text();
        let spans = self
            .provider
            .as_ref()
            .map_or_else(|| syntax::highlight(&text, self.language), |p| p(&text));
        assert!(
            spans.iter().all(|s| s.range.start < s.range.end
                && s.range.end <= text.len()
                && text.is_char_boundary(s.range.start)
                && text.is_char_boundary(s.range.end))
                && spans.windows(2).all(|s| s[0].range.end <= s[1].range.start),
            "provider must return ordered disjoint UTF-8 ranges"
        );
        self.editor.with_buffer_mut(|buffer| {
            let mut offset = 0;
            let mut first = 0;
            for line in &mut buffer.lines {
                let end = offset + line.text().len();
                let mut attrs = AttrsList::new(&Attrs::new().family(Family::Monospace));
                while first < spans.len() && spans[first].range.end <= offset {
                    first += 1;
                }
                for span in spans[first..].iter().take_while(|s| s.range.start < end) {
                    let start = span.range.start.max(offset) - offset;
                    let stop = span.range.end.min(end) - offset;
                    attrs.add_span(
                        start..stop,
                        &Attrs::new()
                            .family(Family::Monospace)
                            .color(self.theme.color(span.category)),
                    );
                }
                offset = end + line.ending().as_str().len();
                line.set_attrs_list(attrs);
            }
        });
    }

    fn edit(&mut self, operation: impl FnOnce(&mut Editor<'static>, &mut FontSystem)) {
        if self.read_only || !self.preedit.is_empty() {
            return;
        }
        let before = (self.editor.cursor(), self.editor.selection());
        self.editor.start_change();
        operation(&mut self.editor, &mut self.fonts);
        if let Some(change) = self.editor.finish_change().filter(|c| !c.items.is_empty()) {
            self.undo.push(History {
                change,
                before,
                after: (self.editor.cursor(), self.editor.selection()),
            });
            self.redo.clear();
            self.publish_snapshot();
            self.restyle();
        }
    }

    pub fn insert(&mut self, text: &str) {
        self.edit(|e, _| e.insert_string(text, None));
    }

    pub fn replace_ranges(&mut self, offsets: &[usize], length: usize, replacement: &str) {
        if self.read_only || !self.preedit.is_empty() || offsets.is_empty() {
            return;
        }
        let text = self.text();
        if offsets
            .windows(2)
            .any(|w| w[0].saturating_add(length) > w[1])
            || offsets.iter().any(|&i| {
                i.checked_add(length).is_none_or(|end| {
                    end > text.len() || !text.is_char_boundary(i) || !text.is_char_boundary(end)
                })
            })
        {
            return;
        }
        let before = (self.editor.cursor(), self.editor.selection());
        self.editor.start_change();
        for &offset in offsets.iter().rev() {
            self.select_offsets(offset, offset + length);
            self.editor.insert_string(replacement, None);
        }
        if let Some(change) = self.editor.finish_change().filter(|c| !c.items.is_empty()) {
            self.undo.push(History {
                change,
                before,
                after: (self.editor.cursor(), self.editor.selection()),
            });
            self.redo.clear();
            self.publish_snapshot();
            self.restyle();
        }
    }

    pub fn undo(&mut self, redo: bool) {
        if self.read_only || !self.preedit.is_empty() {
            return;
        }
        let entry = if redo {
            self.redo.pop()
        } else {
            self.undo.pop()
        };
        if let Some(entry) = entry {
            let mut change = entry.change.clone();
            if !redo {
                change.reverse();
            }
            self.editor.set_selection(Selection::None);
            assert!(self.editor.apply_change(&change));
            let (cursor, selection) = if redo { entry.after } else { entry.before };
            self.editor.set_cursor(cursor);
            self.editor.set_selection(selection);
            if redo {
                self.undo.push(entry);
            } else {
                self.redo.push(entry);
            }
            self.publish_snapshot();
            self.restyle();
        }
    }

    pub fn select_offsets(&mut self, anchor: usize, head: usize) -> bool {
        let text = self.text();
        if anchor > text.len()
            || head > text.len()
            || !text.is_char_boundary(anchor)
            || !text.is_char_boundary(head)
        {
            return false;
        }
        let a = cursor_at(&self.editor, anchor);
        let h = cursor_at(&self.editor, head);
        self.editor.set_cursor(h);
        self.editor.set_selection(if a == h {
            Selection::None
        } else {
            Selection::Normal(a)
        });
        true
    }

    pub fn selected_text(&self) -> Option<String> {
        self.editor.copy_selection()
    }

    fn clipboard(&mut self) -> Result<&mut arboard::Clipboard, arboard::Error> {
        if self.clipboard.is_none() {
            self.clipboard = Some(arboard::Clipboard::new()?);
        }
        Ok(self.clipboard.as_mut().unwrap())
    }

    pub fn command(
        &mut self,
        name: &str,
        value: &str,
        shift: bool,
        control: bool,
        alt: bool,
        meta: bool,
    ) -> bool {
        if !self.preedit.is_empty() {
            return false;
        }
        let shortcut = if cfg!(target_os = "macos") {
            meta
        } else {
            control
        };
        if shortcut {
            let byte = name
                .as_bytes()
                .first()
                .copied()
                .filter(|_| name.len() == 1)
                .map(|b| b.to_ascii_lowercase());
            match byte {
                Some(b'a') => {
                    self.select_offsets(0, self.text().len());
                    return true;
                }
                Some(b'z') => {
                    self.undo(shift);
                    return true;
                }
                Some(b'y') => {
                    self.undo(true);
                    return true;
                }
                Some(b'c' | b'x') => {
                    if let Some(text) = self.selected_text() {
                        match self.clipboard().and_then(|c| c.set_text(text)) {
                            Ok(()) => {
                                self.clipboard_error = None;
                                if name.eq_ignore_ascii_case("x") {
                                    self.edit(|e, _| {
                                        e.delete_selection();
                                    });
                                }
                            }
                            Err(e) => self.clipboard_error = Some(e.to_string()),
                        }
                    }
                    return true;
                }
                Some(b'v') => {
                    if !self.read_only {
                        match self.clipboard().and_then(|c| c.get_text()) {
                            Ok(text) => {
                                self.clipboard_error = None;
                                self.insert(&text);
                            }
                            Err(e) => self.clipboard_error = Some(e.to_string()),
                        }
                    }
                    return true;
                }
                _ => {}
            }
        }
        let word = if cfg!(target_os = "macos") {
            alt
        } else {
            control
        };
        let motion = match name {
            "Left" if meta && cfg!(target_os = "macos") => Some(Motion::Home),
            "Right" if meta && cfg!(target_os = "macos") => Some(Motion::End),
            "Left" => Some(if word { Motion::LeftWord } else { Motion::Left }),
            "Right" => Some(if word {
                Motion::RightWord
            } else {
                Motion::Right
            }),
            "Up" => Some(if shortcut {
                Motion::BufferStart
            } else {
                Motion::Up
            }),
            "Down" => Some(if shortcut {
                Motion::BufferEnd
            } else {
                Motion::Down
            }),
            "Home" => Some(if control {
                Motion::BufferStart
            } else {
                Motion::Home
            }),
            "End" => Some(if control {
                Motion::BufferEnd
            } else {
                Motion::End
            }),
            "PageUp" => Some(Motion::PageUp),
            "PageDown" => Some(Motion::PageDown),
            _ => None,
        };
        if let Some(motion) = motion {
            if !shift && !word && !shortcut && matches!(motion, Motion::Left | Motion::Right) {
                if let Some((start, end)) = self.editor.selection_bounds() {
                    self.editor
                        .set_cursor(if motion == Motion::Left { start } else { end });
                    self.editor.set_selection(Selection::None);
                    return true;
                }
            }
            if shift {
                if self.editor.selection() == Selection::None {
                    self.editor
                        .set_selection(Selection::Normal(self.editor.cursor()));
                }
            } else {
                self.editor.set_selection(Selection::None);
            }
            self.editor.action(&mut self.fonts, Action::Motion(motion));
            return true;
        }
        match name {
            "Backspace" => self.edit(|e, fonts| {
                if !e.delete_selection() {
                    let end = e.cursor();
                    let start = e.with_buffer(|b| {
                        if end.index > 0 {
                            Cursor::new(
                                end.line,
                                b.lines[end.line].text()[..end.index]
                                    .grapheme_indices(true)
                                    .next_back()
                                    .map_or(0, |(i, _)| i),
                            )
                        } else if end.line > 0 {
                            Cursor::new(end.line - 1, b.lines[end.line - 1].text().len())
                        } else {
                            end
                        }
                    });
                    if start != end {
                        e.delete_range(start, end);
                        e.set_cursor(start);
                    }
                }
                e.shape_as_needed(fonts, false);
            }),
            "Delete" => self.edit(|e, f| e.action(f, Action::Delete)),
            "Return" => self.edit(|e, f| e.action(f, Action::Enter)),
            "Tab" => self.edit(|e, f| {
                e.action(
                    f,
                    if shift {
                        Action::Unindent
                    } else {
                        Action::Indent
                    },
                )
            }),
            "Escape" => self.editor.set_selection(Selection::None),
            "Text"
                if !shortcut
                    && !control
                    && !value.is_empty()
                    && !value
                        .chars()
                        .any(|c| c.is_control() || ('\u{f700}'..='\u{f8ff}').contains(&c)) =>
            {
                self.insert(value)
            }
            _ => return false,
        }
        true
    }

    pub fn pointer(&mut self, kind: i32, x: f32, y: f32) {
        if !self.preedit.is_empty() {
            return;
        }
        self.editor.shape_as_needed(&mut self.fonts, false);
        let horizontal = self.editor.with_buffer(|b| b.scroll().horizontal);
        let x = ((x - 48.0) * self.scale + horizontal) as i32;
        let y = (y * self.scale) as i32;
        let action = match kind {
            1 => Action::Drag { x, y },
            2 => Action::DoubleClick { x, y },
            _ => Action::Click { x, y },
        };
        self.editor.action(&mut self.fonts, action);
    }

    pub fn scroll(&mut self, x: f32, y: f32) {
        self.editor.with_buffer_mut(|b| {
            let mut s = b.scroll();
            s.horizontal = (s.horizontal - x * self.scale).max(0.0);
            b.set_scroll(s);
        });
        self.editor.action(
            &mut self.fonts,
            Action::Scroll {
                pixels: -y * self.scale,
            },
        );
    }

    pub fn ime_commit(&mut self, text: &str) {
        self.preedit.clear();
        self.insert(text);
    }

    pub fn update_preedit(&mut self, text: &str, selection: Option<(usize, usize)>) {
        self.preedit.clear();
        self.preedit.push_str(text);
        self.preedit_selection = selection.filter(|&(a, h)| {
            a <= text.len()
                && h <= text.len()
                && text.is_char_boundary(a)
                && text.is_char_boundary(h)
        });
    }

    pub fn cursor_rect(&self) -> (f32, f32, f32, f32) {
        let (x, y) = self.visual_cursor;
        let horizontal = self.editor.with_buffer(|b| b.scroll().horizontal);
        (
            self.origin.0 + 48.0 + (x as f32 - horizontal) / self.scale,
            self.origin.1 + y as f32 / self.scale,
            2.0,
            21.0,
        )
    }

    pub fn render(&mut self, width: f32, height: f32, scale: f32) -> slint::Image {
        let scale = scale.max(0.5);
        let w = (width.max(1.0) * scale).ceil() as u32;
        let h = (height.max(1.0) * scale).ceil() as u32;
        self.scale = scale;
        let gutter = (48.0 * scale) as i32;
        self.editor.with_buffer_mut(|b| {
            b.set_metrics(&mut self.fonts, Metrics::new(14.0 * scale, 21.0 * scale));
            b.set_size(
                &mut self.fonts,
                Some((w as f32 - gutter as f32).max(1.0)),
                Some(h as f32),
            );
        });
        self.editor.shape_as_needed(&mut self.fonts, false);
        let mut buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(w, h);
        let pixels = buffer.make_mut_slice();
        let bg = self.theme.background;
        pixels.fill(slint::Rgba8Pixel::new(bg.r(), bg.g(), bg.b(), 255));
        let mut paint = |x: i32, y: i32, rw: u32, rh: u32, color: Color| {
            for yy in y.max(0)..(y.saturating_add(rh as i32)).min(h as i32) {
                for xx in x.max(0)..(x.saturating_add(rw as i32)).min(w as i32) {
                    let p = &mut pixels[(yy as u32 * w + xx as u32) as usize];
                    let a = color.a() as u32;
                    p.r = ((color.r() as u32 * a + p.r as u32 * (255 - a)) / 255) as u8;
                    p.g = ((color.g() as u32 * a + p.g as u32 * (255 - a)) / 255) as u8;
                    p.b = ((color.b() as u32 * a + p.b as u32 * (255 - a)) / 255) as u8;
                }
            }
        };
        let mut preview;
        let view = if self.preedit.is_empty() {
            &self.editor
        } else {
            preview = self.editor.clone();
            let start = preview
                .selection_bounds()
                .map(|(a, _)| a)
                .unwrap_or(preview.cursor());
            let start_offset = preview.with_buffer(|b| {
                b.lines[..start.line]
                    .iter()
                    .map(|l| l.text().len() + l.ending().as_str().len())
                    .sum::<usize>()
                    + start.index
            });
            preview.insert_string(
                &self.preedit,
                Some(AttrsList::new(
                    &Attrs::new()
                        .family(Family::Monospace)
                        .color(self.theme.foreground),
                )),
            );
            if let Some((a, h)) = self.preedit_selection {
                let anchor = cursor_at(&preview, start_offset + a);
                let head = cursor_at(&preview, start_offset + h);
                preview.set_cursor(head);
                preview.set_selection(if anchor == head {
                    Selection::None
                } else {
                    Selection::Normal(anchor)
                });
            }
            preview.shape_as_needed(&mut self.fonts, false);
            &preview
        };
        self.visual_cursor = view.cursor_position().unwrap_or((0, 0));
        let horizontal = view.with_buffer(|b| b.scroll().horizontal) as i32;
        view.draw(
            &mut self.fonts,
            &mut self.cache,
            self.theme.foreground,
            if self.focused {
                self.theme.cursor
            } else {
                Color::rgba(0, 0, 0, 0)
            },
            self.theme.selection,
            self.theme.selected_text,
            |x, y, rw, rh, c| {
                let left = x + gutter - horizontal;
                let clipped = left.max(gutter);
                let right = left.saturating_add(rw as i32);
                if right > clipped {
                    paint(clipped, y, (right - clipped) as u32, rh, c);
                }
            },
        );
        let runs = view.with_buffer(|b| {
            b.layout_runs()
                .map(|r| (r.line_i, r.line_top))
                .collect::<Vec<_>>()
        });
        paint(0, 0, gutter as u32, h, self.theme.gutter);
        let mut numbers = Buffer::new(&mut self.fonts, Metrics::new(11.0 * scale, 21.0 * scale));
        numbers.set_size(&mut self.fonts, Some(gutter as f32), Some(21.0 * scale));
        for (i, y) in runs {
            numbers.set_text(
                &mut self.fonts,
                &(i + 1).to_string(),
                &Attrs::new().family(Family::Monospace),
                Shaping::Advanced,
                None,
            );
            numbers.shape_until_scroll(&mut self.fonts, false);
            numbers.draw(
                &mut self.fonts,
                &mut self.cache,
                self.theme.line_number,
                |x, yy, rw, rh, c| paint(x + 6, yy + y as i32, rw, rh, c),
            );
        }
        slint::Image::from_rgba8(buffer)
    }
}

fn cursor_at(editor: &Editor<'_>, offset: usize) -> Cursor {
    editor.with_buffer(|buffer| {
        let mut start = 0;
        for (i, line) in buffer.lines.iter().enumerate() {
            let end = start + line.text().len();
            if offset <= end {
                return Cursor::new(i, offset.saturating_sub(start));
            }
            let next = end + line.ending().as_str().len();
            if offset < next {
                return Cursor::new(i, line.text().len());
            }
            start = next;
        }
        let i = buffer.lines.len() - 1;
        Cursor::new(i, buffer.lines[i].text().len())
    })
}
