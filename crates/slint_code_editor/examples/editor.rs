slint::include_modules!();
fn main() -> Result<(), slint::PlatformError> {
    let window = EditorDemo::new()?;
    let runtime = slint_code_editor::install_editor!(window, CodeEditorBridge);
    window.on_configure_provider(move |id, enabled| {
        if let Some(document) = runtime.borrow_mut().document_mut(id) {
            document.set_provider(if enabled {
                Some(Box::new(|source: &str| {
                    source
                        .match_indices("true")
                        .map(|(start, word)| slint_code_editor::syntax::Highlight {
                            range: start..start + word.len(),
                            category: slint_code_editor::syntax::Category::Number,
                        })
                        .collect()
                }))
            } else {
                None
            });
        }
    });
    window.run()
}
