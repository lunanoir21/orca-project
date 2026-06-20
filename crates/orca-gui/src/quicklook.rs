//! Quick Look — a fullscreen preview overlay for the selected file.
//!
//! Bound to the configurable `quick_look` keybind (default: Space). Distinct
//! from the side [`crate::preview`] panel: this is a transient, undecorated
//! fullscreen window that shows one file large, then gets out of the way.
//! Closed with Escape or by clicking the dimmed background.
//!
//! Rendering is shared with the side panel via [`crate::preview::render_into`]
//! so the two never drift apart on what they can show.

use orca_core::FileEntry;
use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::{ComponentParts, ComponentSender, SimpleComponent};

/// The Quick Look overlay component. Stateless beyond its window: it shows
/// one entry and closes itself.
pub struct QuickLookOverlay;

impl SimpleComponent for QuickLookOverlay {
    type Init = FileEntry;
    type Input = ();
    type Output = ();
    type Root = gtk::Window;
    type Widgets = ();

    fn init_root() -> Self::Root {
        gtk::Window::builder()
            .decorated(false)
            .modal(true)
            .css_classes(["orca-root", "orca-quicklook"])
            .build()
    }

    fn init(
        entry: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        root.fullscreen();

        // Outer: dimmed full-bleed background; clicking it closes the overlay.
        let outer = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .halign(gtk::Align::Fill)
            .valign(gtk::Align::Fill)
            .hexpand(true)
            .vexpand(true)
            .css_classes(["orca-quicklook-scrim"])
            .build();
        {
            let root = root.clone();
            let close_on_click = gtk::GestureClick::new();
            close_on_click.connect_released(move |_, _, _, _| root.close());
            outer.add_controller(close_on_click);
        }

        // Card: the actual content, centered, does not propagate clicks to the scrim.
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .width_request(720)
            .height_request(560)
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(24)
            .margin_end(24)
            .css_classes(["orca-quicklook-card"])
            .build();
        {
            let stop_propagation = gtk::GestureClick::new();
            stop_propagation.connect_released(|gesture, _, _, _| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
            });
            card.add_controller(stop_propagation);
        }

        let close_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .halign(gtk::Align::End)
            .build();
        let close_btn = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .has_frame(false)
            .build();
        {
            let root = root.clone();
            close_btn.connect_clicked(move |_| root.close());
        }
        close_row.append(&close_btn);
        card.append(&close_row);

        crate::preview::render_into(&card, &entry, None);

        outer.append(&card);
        root.set_child(Some(&outer));

        // Escape closes, same as clicking the scrim.
        let key_ctrl = gtk::EventControllerKey::new();
        {
            let root = root.clone();
            key_ctrl.connect_key_pressed(move |_, keyval, _, _| {
                if keyval == gtk::gdk::Key::Escape {
                    root.close();
                    gtk::glib::Propagation::Stop
                } else {
                    gtk::glib::Propagation::Proceed
                }
            });
        }
        root.add_controller(key_ctrl);

        ComponentParts {
            model: QuickLookOverlay,
            widgets: (),
        }
    }
}
