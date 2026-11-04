use slint_code_editor::{
    document::Document,
    syntax::{Category, Highlight, Language},
    theme::Theme,
    Runtime,
};

#[test]
fn style_switch_preserves_unicode_selection_and_single_edit_history() {
    let mut d = Document::default();
    let original = "// 中文🙂\r\nlet 色 = 色;\n";
    d.set_text(original);
    assert!(d.select_offsets(19, 22));
    assert_eq!(d.selected_text().as_deref(), Some("色"));
    d.insert("光");
    d.set_language(Language::Wgsl);
    d.set_theme(Theme::dark());
    d.set_provider(Some(Box::new(|s| {
        s.find('光')
            .map(|i| Highlight {
                range: i..i + 3,
                category: Category::Type,
            })
            .into_iter()
            .collect()
    })));
    assert_eq!(d.text(), original.replacen('色', "光", 1));
    d.undo(false);
    assert_eq!(d.text(), original);
    assert_eq!(d.selected_text().as_deref(), Some("色"));
    d.undo(true);
    assert_eq!(d.text(), original.replacen('色', "光", 1));
}

#[test]
fn replace_all_is_one_transaction_and_instances_are_isolated() {
    let mut r = Runtime::default();
    let a = r.create("色 色🙂色");
    let b = r.create("unchanged");
    assert_eq!(r.find(a, "色", false), 3);
    assert_eq!(r.find(a, "", false), 0);
    assert_eq!(r.find(a, "missing", true), 0);
    assert_eq!(r.replace_matches(a, "色", "光", true).as_str(), "光 光🙂光");
    assert_eq!(r.text(b).as_str(), "unchanged");
    r.document_mut(a).unwrap().undo(false);
    assert_eq!(r.text(a).as_str(), "色 色🙂色");
    r.goto_line(a, 1);
    assert_eq!(r.document_mut(a).unwrap().cursor_offset(), 0);
}

#[test]
fn readonly_composition_and_grapheme_boundaries() {
    let mut d = Document::default();
    d.set_text("a👩‍💻e\u{301}\r\n");
    assert!(!d.select_offsets(2, 2));
    let end = "a👩‍💻e\u{301}".len();
    d.select_offsets(end, end);
    d.command("Backspace", "", false, false, false, false);
    assert_eq!(d.text(), "a👩‍💻\r\n");
    d.undo(false);
    d.select_offsets(1, 12);
    d.preedit = "中".into();
    assert_eq!(d.text(), "a👩‍💻e\u{301}\r\n");
    d.ime_commit("中文");
    assert_eq!(d.text(), "a中文e\u{301}\r\n");
    d.undo(false);
    assert_eq!(d.text(), "a👩‍💻e\u{301}\r\n");
    d.read_only = true;
    d.insert("ignored");
    d.undo(true);
    assert_eq!(d.text(), "a👩‍💻e\u{301}\r\n");
    assert!(d.select_offsets(0, 1));
    assert_eq!(d.selected_text().as_deref(), Some("a"));
}

#[test]
fn cjk_fallback_renders_and_hit_testing_uses_shaped_widths() {
    let mut d = Document::default();
    d.set_text("中a\n中文🙂");
    d.select_offsets(4, 4);
    d.render(400.0, 200.0, 2.0);
    let (x, y, _, height) = d.cursor_rect();
    d.select_offsets(0, 0);
    d.pointer(0, x, y + height / 2.0);
    assert_eq!(d.cursor_offset(), 4);
    d.insert("!");
    assert_eq!(d.text(), "中a!\n中文🙂");
}

#[test]
fn readonly_transition_precedes_render_and_selection_collapses_without_skipping() {
    let mut runtime = Runtime::default();
    let id = runtime.create("a色🙂z\r\n");
    runtime.select(id, 1, 8);
    runtime.read_only(id, true);
    runtime.command(id, "X", "", false, false, false, false);
    assert_eq!(runtime.text(id), "a色🙂z\r\n");
    runtime.read_only(id, false);
    runtime.command(id, "\u{f702}", "", false, false, false, false);
    assert_eq!(runtime.document_mut(id).unwrap().cursor_offset(), 1);
    runtime.select(id, 1, 8);
    runtime.command(id, "\u{f703}", "", false, false, false, false);
    assert_eq!(runtime.document_mut(id).unwrap().cursor_offset(), 8);
    runtime.select(id, 10, 10);
    assert_eq!(runtime.document_mut(id).unwrap().cursor_offset(), 9);
    assert!(!runtime.command(id, "\u{f704}", "", false, false, false, false));
    assert_eq!(runtime.text(id), "a色🙂z\r\n");
}
