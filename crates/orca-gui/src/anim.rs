//! Shared open-animation for transient dialog windows.
//!
//! Plain GTK4 (no libadwaita) has no `Adw.TimedAnimation`/`PropertyAnimationTarget`,
//! so the fade is driven by hand off the window's frame clock via
//! `add_tick_callback` — the same primitive libadwaita's animations are built
//! on internally, just without the extra dependency.

use std::time::Instant;

use relm4::gtk;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;

/// Fade-in duration. Short enough to feel snappy, long enough to register as
/// intentional rather than a flicker.
const FADE_DURATION_MS: f64 = 150.0;

/// Present `window`, fading its opacity in from 0 to 1 over [`FADE_DURATION_MS`].
///
/// Replaces a bare `.present()` call at every dialog/window call site so every
/// transient window in the app opens the same way instead of snapping into
/// existence.
pub fn present_with_fade(window: &impl IsA<gtk::Window>) {
    let window = window.as_ref();
    window.set_opacity(0.0);
    window.present();

    let start = Instant::now();
    window.add_tick_callback(move |win, _clock| {
        let t = (start.elapsed().as_secs_f64() * 1000.0 / FADE_DURATION_MS).min(1.0);
        let eased = 1.0 - (1.0 - t).powi(3); // ease-out cubic
        win.set_opacity(eased);
        if t >= 1.0 {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}
