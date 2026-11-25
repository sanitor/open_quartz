use slint::winit_030::{winit::event::WindowEvent, EventResult, WinitWindowAccessor};

pub(crate) fn isolate_pointer(window: &slint::Window) {
    // MCP dispatches directly to Slint. Native cursor movement or a click
    // between its synthetic press and release can cancel that interaction.
    window.on_winit_window_event(|_, event| match event {
        WindowEvent::CursorMoved { .. }
        | WindowEvent::CursorEntered { .. }
        | WindowEvent::CursorLeft { .. }
        | WindowEvent::MouseInput { .. }
        | WindowEvent::MouseWheel { .. }
        | WindowEvent::Touch(_) => EventResult::PreventDefault,
        _ => EventResult::Propagate,
    });
}
