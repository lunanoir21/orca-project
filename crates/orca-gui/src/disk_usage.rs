//! Disk usage visualizer dialog (Phase 5.7).
//!
//! Shows a **bar chart** of the top-N largest items in the current directory
//! and a **treemap** painted on a `gtk::DrawingArea`. Clicking a bar-chart row
//! or treemap rectangle navigates the calling pane into that subdirectory.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Hit-test rectangle list: (absolute path, (x, y, w, h)).
type HitRects = Rc<RefCell<Vec<(PathBuf, (f64, f64, f64, f64))>>>;

use relm4::gtk;
use relm4::gtk::cairo;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender};

use orca_core::UsageNode;

use crate::format;
use crate::i18n;

/// Max bar-chart rows.
const TOP_N: usize = 12;
/// Depth passed to `disk_usage_tree`.
const TREE_DEPTH: usize = 2;

/// What the dialog emits when a directory is chosen.
#[derive(Debug)]
pub enum DiskUsageOutput {
    /// Navigate the calling pane to this path.
    Navigate(PathBuf),
}

/// Disk usage dialog model.
pub struct DiskUsageDialog {
    root_path: PathBuf,
    /// Bar-chart list box (rebuilt on each load).
    bar_list: gtk::ListBox,
    /// Treemap drawing area (draw func re-installed on each load).
    treemap_area: gtk::DrawingArea,
}

#[derive(Debug)]
pub enum DiskUsageInput {
    /// Re-load the tree.
    Reload,
    /// Navigate to a subdirectory.
    NavigateTo(PathBuf),
}

#[derive(Debug)]
pub enum DiskUsageCmd {
    Loaded(PathBuf, Result<UsageNode, String>),
}

#[relm4::component(pub)]
impl Component for DiskUsageDialog {
    type Init = PathBuf;
    type Input = DiskUsageInput;
    type Output = DiskUsageOutput;
    type CommandOutput = DiskUsageCmd;

    view! {
        #[root]
        gtk::Window {
            set_modal: true,
            set_default_width: 640,
            set_default_height: 520,
        }
    }

    fn init(
        path: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let title = path
            .file_name()
            .map(|n| format!("{} — {}", n.to_string_lossy(), i18n::t("du.title")))
            .unwrap_or_else(|| i18n::t("du.title"));
        root.set_title(Some(&title));

        let widgets = view_output!();

        let outer = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();

        // Tab switcher
        let switcher = gtk::StackSwitcher::new();
        outer.append(&switcher);

        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        switcher.set_stack(Some(&stack));
        outer.append(&stack);

        // --- Bar chart page ---
        let bar_list = gtk::ListBox::new();
        bar_list.set_selection_mode(gtk::SelectionMode::None);
        bar_list.add_css_class("boxed-list");
        let bar_scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .child(&bar_list)
            .build();
        stack.add_titled(&bar_scroll, Some("bar"), &i18n::t("du.bar"));

        // --- Treemap page ---
        let treemap_area = gtk::DrawingArea::builder()
            .vexpand(true)
            .hexpand(true)
            .build();
        stack.add_titled(&treemap_area, Some("treemap"), &i18n::t("du.treemap"));

        // Close button
        let close_btn = gtk::Button::with_label(&i18n::t("common.close"));
        close_btn.set_halign(gtk::Align::End);
        {
            let root = root.clone();
            close_btn.connect_clicked(move |_| root.close());
        }
        outer.append(&close_btn);

        root.set_child(Some(&outer));

        let model = DiskUsageDialog {
            root_path: path.clone(),
            bar_list,
            treemap_area,
        };

        sender.input(DiskUsageInput::Reload);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            DiskUsageInput::Reload => {
                let path = self.root_path.clone();
                sender.oneshot_command(async move {
                    let result = orca_core::disk_usage_tree(&path, TREE_DEPTH)
                        .await
                        .map_err(|e| e.to_string());
                    DiskUsageCmd::Loaded(path, result)
                });
            }
            DiskUsageInput::NavigateTo(path) => {
                sender.output(DiskUsageOutput::Navigate(path)).ok();
            }
        }
    }

    fn update_cmd(
        &mut self,
        msg: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            DiskUsageCmd::Loaded(root_path, Ok(tree)) => {
                self.root_path = root_path;
                self.rebuild_bar(&tree, &sender);
                self.install_treemap_draw(&tree, &sender);
            }
            DiskUsageCmd::Loaded(_, Err(e)) => {
                tracing::warn!(error = %e, "disk_usage_tree failed");
            }
        }
    }
}

impl DiskUsageDialog {
    /// Rebuild the bar-chart list from the tree's direct children.
    fn rebuild_bar(&self, tree: &UsageNode, sender: &ComponentSender<Self>) {
        while let Some(child) = self.bar_list.first_child() {
            self.bar_list.remove(&child);
        }

        let total = tree.size.max(1);
        let mut children: Vec<&UsageNode> = tree.children.iter().collect();
        children.sort_by_key(|b| std::cmp::Reverse(b.size));
        children.truncate(TOP_N);

        for child in children {
            let abs_path = self.root_path.join(&child.name);
            let is_dir = !child.children.is_empty();

            let row = gtk::ListBoxRow::new();
            let bx = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(2)
                .margin_top(6)
                .margin_bottom(6)
                .margin_start(6)
                .margin_end(6)
                .build();

            let header = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(8)
                .build();
            let name_lbl = gtk::Label::builder()
                .label(&child.name)
                .halign(gtk::Align::Start)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .build();
            let pct = (child.size as f64 / total as f64 * 100.0).round() as u64;
            let size_lbl = gtk::Label::builder()
                .label(format!("{} ({}%)", format::size(child.size), pct))
                .halign(gtk::Align::End)
                .build();
            size_lbl.add_css_class("dim-label");
            header.append(&name_lbl);
            header.append(&size_lbl);

            let bar = gtk::ProgressBar::new();
            bar.set_fraction((child.size as f64 / total as f64).clamp(0.0, 1.0));
            bar.set_hexpand(true);

            bx.append(&header);
            bx.append(&bar);

            if is_dir {
                let nav_btn = gtk::Button::with_label(&i18n::t("du.navigate"));
                nav_btn.add_css_class("flat");
                nav_btn.set_halign(gtk::Align::Start);
                let s = sender.clone();
                let p = abs_path.clone();
                nav_btn.connect_clicked(move |_| s.input(DiskUsageInput::NavigateTo(p.clone())));
                bx.append(&nav_btn);
            }

            row.set_child(Some(&bx));
            self.bar_list.append(&row);
        }
    }

    /// Install the draw function and click handler on the treemap DrawingArea.
    fn install_treemap_draw(&self, tree: &UsageNode, sender: &ComponentSender<Self>) {
        let tree = tree.clone();
        let root_path = self.root_path.clone();

        // Laid-out rectangles for click hit-testing: (abs_path, (x,y,w,h)).
        let rects: HitRects = Rc::new(RefCell::new(Vec::new()));
        let rects_draw = rects.clone();

        self.treemap_area
            .set_draw_func(move |_area, cr, width, height| {
                rects_draw.borrow_mut().clear();
                draw_treemap(
                    cr,
                    &tree,
                    &root_path,
                    0.0,
                    0.0,
                    width as f64,
                    height as f64,
                    &rects_draw,
                );
            });

        // Remove existing click controllers before adding a new one.
        for ctrl in self.treemap_area.observe_controllers().snapshot() {
            if let Ok(c) = ctrl.downcast::<gtk::GestureClick>() {
                self.treemap_area.remove_controller(&c);
            }
        }

        let click = gtk::GestureClick::new();
        let s = sender.clone();
        click.connect_pressed(move |_, _, x, y| {
            let hit = rects.borrow();
            for (path, (rx, ry, rw, rh)) in hit.iter() {
                if x >= *rx && x <= *rx + *rw && y >= *ry && y <= *ry + *rh {
                    s.input(DiskUsageInput::NavigateTo(path.clone()));
                    break;
                }
            }
        });
        self.treemap_area.add_controller(click);
        self.treemap_area.queue_draw();
    }
}

/// Draw a slice-and-dice treemap, storing hit rectangles.
///
/// Top-level children of `node` are layed out in the `(x,y,w,h)` bounding
/// box. Each child's absolute path is `root / child.name`. Recursion is done
/// for the first child level only (grandchildren use a different root).
#[allow(clippy::too_many_arguments)]
fn draw_treemap(
    cr: &cairo::Context,
    node: &UsageNode,
    root: &Path,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    rects: &HitRects,
) {
    if node.children.is_empty() || w < 4.0 || h < 4.0 {
        return;
    }

    const PALETTE: [(f64, f64, f64); 6] = [
        (0.40, 0.65, 0.90),
        (0.52, 0.78, 0.52),
        (0.95, 0.76, 0.42),
        (0.82, 0.53, 0.53),
        (0.68, 0.57, 0.82),
        (0.49, 0.76, 0.76),
    ];

    let total = node.size.max(1) as f64;
    let landscape = w >= h;
    let mut cur = if landscape { x } else { y };

    for (i, child) in node.children.iter().enumerate() {
        if child.size == 0 {
            continue;
        }
        let frac = child.size as f64 / total;
        let (rw, rh) = if landscape {
            (w * frac, h)
        } else {
            (w, h * frac)
        };
        let (rx, ry) = if landscape { (cur, y) } else { (x, cur) };

        // Store hit rect for click-to-navigate (only for non-trivial dirs).
        if !child.children.is_empty() {
            rects
                .borrow_mut()
                .push((root.join(&child.name), (rx, ry, rw, rh)));
        }

        let (r, g, b) = PALETTE[i % PALETTE.len()];
        cr.set_source_rgba(r, g, b, 0.65);
        cr.rectangle(rx + 1.0, ry + 1.0, (rw - 2.0).max(0.0), (rh - 2.0).max(0.0));
        let _ = cr.fill();

        cr.set_source_rgba(0.0, 0.0, 0.0, 0.12);
        cr.set_line_width(1.0);
        cr.rectangle(rx + 0.5, ry + 0.5, (rw - 1.0).max(0.0), (rh - 1.0).max(0.0));
        let _ = cr.stroke();

        if rw > 40.0 && rh > 20.0 {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.80);
            cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
            cr.set_font_size((rh * 0.11).clamp(9.0, 13.0));
            let text = format!("{} ({})", child.name, format::size(child.size));
            let text_x = rx + 4.0;
            let text_y = ry + (rh + 12.0) / 2.0;
            cr.move_to(text_x, text_y.min(ry + rh - 4.0));
            let _ = cr.show_text(&text);
        }

        // Recurse into second level.
        if !child.children.is_empty() && rw > 10.0 && rh > 10.0 {
            let child_root = root.join(&child.name);
            draw_treemap(
                cr,
                child,
                &child_root,
                rx + 2.0,
                ry + 18.0,
                (rw - 4.0).max(0.0),
                (rh - 20.0).max(0.0),
                rects,
            );
        }

        if landscape {
            cur += rw;
        } else {
            cur += rh;
        }
    }
}
