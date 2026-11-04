pub mod document;
pub mod syntax;
pub mod theme;

use document::Document;
use slint::{
    winit_030::{winit, EventResult, WinitWindowAccessor},
    SharedString,
};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

pub struct Runtime {
    documents: HashMap<i32, Document>,
    next: i32,
    focused: Option<i32>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            documents: HashMap::new(),
            next: 0,
            focused: None,
        }
    }
}

struct DocumentLease {
    id: i32,
    runtime: std::rc::Weak<RefCell<Runtime>>,
    notify: slint::ModelNotify,
}
impl slint::Model for DocumentLease {
    type Data = i32;
    fn row_count(&self) -> usize {
        1
    }
    fn row_data(&self, row: usize) -> Option<i32> {
        (row == 0).then_some(self.id)
    }
    fn model_tracker(&self) -> &dyn slint::ModelTracker {
        &self.notify
    }
}
impl Drop for DocumentLease {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.upgrade() {
            let mut r = runtime.borrow_mut();
            r.documents.remove(&self.id);
            if r.focused == Some(self.id) {
                r.focused = None;
            }
        }
    }
}
pub fn create_lease(runtime: &Rc<RefCell<Runtime>>, text: &str) -> slint::ModelRc<i32> {
    let id = runtime.borrow_mut().create(text);
    slint::ModelRc::new(DocumentLease {
        id,
        runtime: Rc::downgrade(runtime),
        notify: slint::ModelNotify::default(),
    })
}

impl Runtime {
    pub fn create(&mut self, text: &str) -> i32 {
        let id = self.next;
        self.next += 1;
        let mut d = Document::default();
        d.set_text(text);
        self.documents.insert(id, d);
        id
    }
    pub fn replace(&mut self, id: i32, text: &str) {
        if let Some(d) = self.documents.get_mut(&id) {
            d.set_text(text);
        }
    }
    pub fn text(&self, id: i32) -> SharedString {
        self.documents
            .get(&id)
            .map(|d| d.text().into())
            .unwrap_or_default()
    }
    pub fn document_mut(&mut self, id: i32) -> Option<&mut Document> {
        self.documents.get_mut(&id)
    }
    pub fn theme(&mut self, id: i32, dark: bool) {
        if let Some(d) = self.documents.get_mut(&id) {
            d.set_theme(if dark {
                theme::Theme::dark()
            } else {
                theme::Theme::default()
            });
        }
    }
    pub fn error(&self, id: i32) -> SharedString {
        self.documents
            .get(&id)
            .and_then(|d| d.clipboard_error.as_deref())
            .unwrap_or("")
            .into()
    }
    pub fn read_only(&mut self, id: i32, value: bool) {
        if let Some(d) = self.documents.get_mut(&id) {
            d.read_only = value;
            if value {
                d.update_preedit("", None);
            }
        }
    }
    pub fn focus(&mut self, id: i32, focused: bool) {
        if focused {
            self.focused = Some(id);
        } else if self.focused == Some(id) {
            self.focused = None;
        }
        if let Some(d) = self.documents.get_mut(&id) {
            d.focused = focused;
            if !focused {
                d.preedit.clear();
            }
        }
    }
    pub fn goto_line(&mut self, id: i32, line: i32) {
        if let Some(d) = self.documents.get_mut(&id) {
            let text = d.text();
            let offset = if line <= 1 {
                0
            } else {
                text.match_indices('\n')
                    .nth((line - 2) as usize)
                    .map(|(i, _)| i + 1)
                    .unwrap_or(text.len())
            };
            d.select_offsets(offset, offset);
        }
    }
    pub fn replace_matches(
        &mut self,
        id: i32,
        query: &str,
        replacement: &str,
        all: bool,
    ) -> SharedString {
        let Some(d) = self.documents.get_mut(&id) else {
            return SharedString::default();
        };
        if !query.is_empty() {
            if all {
                let text = d.text();
                let offsets = text
                    .match_indices(query)
                    .map(|(i, _)| i)
                    .collect::<Vec<_>>();
                d.replace_ranges(&offsets, query.len(), replacement);
            } else if d.selected_text().as_deref() == Some(query) {
                d.insert(replacement);
            }
        }
        d.text().into()
    }
    #[allow(clippy::too_many_arguments)]
    pub fn frame(
        &mut self,
        id: i32,
        w: f32,
        h: f32,
        scale: f32,
        language: &str,
        read_only: bool,
        focused: bool,
        x: f32,
        y: f32,
        dark: bool,
    ) -> slint::Image {
        if focused {
            self.focused = Some(id);
        } else if self.focused == Some(id) {
            self.focused = None;
        }
        let Some(d) = self.documents.get_mut(&id) else {
            return slint::Image::default();
        };
        d.focused = focused;
        d.read_only = read_only;
        d.origin = (x, y);
        if !focused {
            d.preedit.clear();
        }
        d.set_language(match language {
            "wgsl" => syntax::Language::Wgsl,
            "json" => syntax::Language::Json,
            _ => syntax::Language::Plain,
        });
        d.set_theme(if dark {
            theme::Theme::dark()
        } else {
            theme::Theme::default()
        });
        d.render(w, h, scale)
    }
    pub fn command(
        &mut self,
        id: i32,
        key: &str,
        value: &str,
        shift: bool,
        control: bool,
        alt: bool,
        meta: bool,
    ) -> bool {
        let Some(d) = self.documents.get_mut(&id) else {
            return false;
        };
        if !d.preedit.is_empty() {
            return true;
        }
        let shift = shift || key == "\u{19}";
        let key = match key {
            "\u{f702}" => "Left",
            "\u{f703}" => "Right",
            "\u{f700}" => "Up",
            "\u{f701}" => "Down",
            "\u{f729}" => "Home",
            "\u{f72b}" => "End",
            "\u{f72c}" => "PageUp",
            "\u{f72d}" => "PageDown",
            "\u{8}" => "Backspace",
            "\u{7f}" | "\u{f728}" => "Delete",
            "\n" | "\r" => "Return",
            "\t" | "\u{19}" => "Tab",
            "\u{1b}" => "Escape",
            other => other,
        };
        d.command(key, value, shift, control, alt, meta)
            || d.command("Text", key, shift, control, alt, meta)
    }
    pub fn pointer(&mut self, id: i32, kind: i32, x: f32, y: f32) -> SharedString {
        if let Some(d) = self.documents.get_mut(&id) {
            d.pointer(kind, x, y);
        }
        self.text(id)
    }
    pub fn scroll(&mut self, id: i32, x: f32, y: f32) {
        if let Some(d) = self.documents.get_mut(&id) {
            d.scroll(x, y);
        }
    }
    pub fn select(&mut self, id: i32, a: i32, h: i32) {
        if let Some(d) = self.documents.get_mut(&id) {
            d.select_offsets(
                a.max(0) as usize,
                if h < 0 { d.text().len() } else { h as usize },
            );
        }
    }
    pub fn find(&mut self, id: i32, query: &str, next: bool) -> i32 {
        let Some(d) = self.documents.get_mut(&id) else {
            return 0;
        };
        if query.is_empty() {
            return 0;
        }
        let text = d.text();
        let offsets = text
            .match_indices(query)
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        if next {
            let current = d.selected_text();
            let start = if current.as_deref() == Some(query) {
                d.cursor_offset()
            } else {
                0
            };
            if let Some(&offset) = offsets.iter().find(|&&i| i >= start).or(offsets.first()) {
                d.select_offsets(offset, offset + query.len());
            }
        }
        offsets.len() as i32
    }
}

/// Register once per window; all instances share one event filter, not a global document.
pub fn attach(window: &slint::Window, runtime: Rc<RefCell<Runtime>>, notify: impl Fn() + 'static) {
    window.on_winit_window_event(move |_window, event| {
        let mut r = runtime.borrow_mut();
        let Some(id) = r.focused else {
            return EventResult::Propagate;
        };
        let Some(d) = r.documents.get_mut(&id) else {
            return EventResult::Propagate;
        };
        let mut consume = false;
        match event {
            winit::event::WindowEvent::Ime(winit::event::Ime::Preedit(text, selection))
                if !d.read_only =>
            {
                d.update_preedit(text, *selection);
                consume = true;
            }
            winit::event::WindowEvent::Ime(winit::event::Ime::Commit(text)) if !d.read_only => {
                d.ime_commit(text);
                consume = true;
            }
            winit::event::WindowEvent::Ime(winit::event::Ime::Disabled) => {
                d.preedit.clear();
                consume = true;
            }
            winit::event::WindowEvent::Focused(false) => {
                d.preedit.clear();
            }
            // Leave key events to FocusScope: winit/Slint already suppress IME duplicates.
            _ => {}
        }
        drop(r);
        if consume || matches!(event, winit::event::WindowEvent::Focused(_)) {
            notify();
        }
        if consume {
            EventResult::PreventDefault
        } else {
            EventResult::Propagate
        }
    });
}

pub fn update_ime(window: &slint::Window, runtime: &Rc<RefCell<Runtime>>) {
    let r = runtime.borrow();
    let d = r.focused.and_then(|id| r.documents.get(&id));
    if d.is_none() {
        return;
    }
    window.with_winit_window(|w| {
        w.set_ime_allowed(d.is_some_and(|d| !d.read_only));
        if let Some(d) = d {
            let (x, y, width, height) = d.cursor_rect();
            w.set_ime_cursor_area(
                winit::dpi::LogicalPosition::new(x, y),
                winit::dpi::LogicalSize::new(width, height),
            );
        }
    });
}

/// Generated Slint bridge types belong to the consuming compilation unit.
/// This macro binds them without requiring OpenQuartz types in this crate.
#[macro_export]
macro_rules! install_editor {
    ($window:expr, $bridge:ty) => {{
        use slint::ComponentHandle as _;
        let window = &$window;
        let runtime = std::rc::Rc::new(std::cell::RefCell::new($crate::Runtime::default()));
        let bridge = window.global::<$bridge>();
        let r = runtime.clone();
        bridge.on_create(move |s| $crate::create_lease(&r, &s));
        let r = runtime.clone();
        bridge.on_replace(move |id, s| r.borrow_mut().replace(id, &s));
        let r = runtime.clone();
        bridge.on_text(move |id| r.borrow().text(id));
        let r = runtime.clone();
        let weak = window.as_weak();
        bridge.on_frame(move |id, w, h, s, l, ro, f, x, y, dark| {
            let scale = weak
                .upgrade()
                .map(|w| w.window().scale_factor())
                .unwrap_or(s);
            let image = r.borrow_mut().frame(id, w, h, scale, &l, ro, f, x, y, dark);
            if let Some(window) = weak.upgrade() {
                $crate::update_ime(window.window(), &r);
            }
            image
        });
        let r = runtime.clone();
        bridge
            .on_command(move |id, k, v, s, c, a, m| r.borrow_mut().command(id, &k, &v, s, c, a, m));
        let r = runtime.clone();
        bridge.on_pointer(move |id, k, x, y| r.borrow_mut().pointer(id, k, x, y));
        let r = runtime.clone();
        bridge.on_scroll(move |id, x, y| r.borrow_mut().scroll(id, x, y));
        let r = runtime.clone();
        bridge.on_select(move |id, a, h| r.borrow_mut().select(id, a, h));
        let r = runtime.clone();
        bridge.on_error(move |id| r.borrow().error(id));
        let r = runtime.clone();
        bridge.on_read_only(move |id, ro| r.borrow_mut().read_only(id, ro));
        let r = runtime.clone();
        bridge.on_find(move |id, q, n| r.borrow_mut().find(id, &q, n));
        let r = runtime.clone();
        bridge.on_goto_line(move |id, line| r.borrow_mut().goto_line(id, line));
        let r = runtime.clone();
        bridge.on_replace_matches(move |id, q, s, all| {
            r.borrow_mut().replace_matches(id, &q, &s, all)
        });
        let r = runtime.clone();
        let weak = window.as_weak();
        bridge.on_focus(move |id, focused| {
            r.borrow_mut().focus(id, focused);
            if let Some(window) = weak.upgrade() {
                if focused {
                    $crate::update_ime(window.window(), &r);
                } else {
                    use slint::winit_030::WinitWindowAccessor as _;
                    window
                        .window()
                        .with_winit_window(|w| w.set_ime_allowed(false));
                }
            }
        });
        let weak = window.as_weak();
        $crate::attach(window.window(), runtime.clone(), move || {
            if let Some(window) = weak.upgrade() {
                let b = window.global::<$bridge>();
                b.set_tick(b.get_tick().wrapping_add(1));
            }
        });
        bridge.set_ready(true);
        runtime
    }};
}
