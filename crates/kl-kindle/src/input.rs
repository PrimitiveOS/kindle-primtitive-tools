//! Feed SDK input to the Launcher and Store event loop.
//!
//! Callers that change viewport geometry can opt into the `viewport_*` loops
//! and put `ViewportEvent` into their event queue. The portrait `Ev::Input` API
//! remains available to existing callers.

use std::sync::mpsc::Sender;

use crate::Ev;

pub use primitive_input::{
    find, gesture, grab_touch, turn_point,
    viewport::{
        viewport_button_loop, viewport_touch_loop, NativeInput, NativeTouch, ResolvedInput,
        TouchPhase, ViewportEvent, ViewportGeneration,
    },
    TURN,
};

pub fn touch_loop(path: String, tx: Sender<Ev>) {
    primitive_input::touch_loop(path, |input| {
        let _ = tx.send(Ev::Input(input));
    });
}

pub fn button_loop(path: String, tx: Sender<Ev>) {
    primitive_input::button_loop(path, |input| {
        let _ = tx.send(Ev::Input(input));
    });
}
