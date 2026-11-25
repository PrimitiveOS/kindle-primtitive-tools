//! Feed SDK input to the Launcher and Store event loop.

use std::sync::mpsc::Sender;

use crate::Ev;

pub use primitive_input::{find, gesture, grab_touch, turn_point, TURN};

pub fn touch_loop(path: String, tx: Sender<Ev>) {
    primitive_input::touch_loop(path, |input| {
        let _ = tx.send(Ev::Input(input));
    });
}

pub fn button_loop(path: String, tx: Sender<Ev>) {
    let keys = tx.clone();
    primitive_input::button_loop_keys(
        path,
        |input| {
            let _ = tx.send(Ev::Input(input));
        },
        |code, down| {
            let _ = keys.send(Ev::PageKey { code, down });
        },
    );
}
