//! Persistent full-document and inspection pairs of read-only diff panes.
//!
//! This module owns presentation only. Hunk acceptance, snapshot writes and
//! the undo stack live in `window`, which keeps all state changes in one
//! place.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use adw::prelude::*;
use gtk::glib;
use sourceview5::prelude::*;

use crate::core::diff::{DiffResult, Hunk, InlineSpan};
use crate::core::inline::ChangeKind;
use crate::core::session::DiffSession;
use crate::core::textstats::{TextStats, text_stats};
use crate::ui::diff_toolbar::{DiffToolbarState, ExitState};
use crate::ui::gutter_selection;
use crate::ui::status_bar::StatusBar;
use crate::ui::theme;

// Fixed/XML opaque review colours distinguish normal and focused diff regions.
const TAG_HUNK: &str = "notediff-hunk";
const TAG_WORD: &str = "notediff-word";
const TAG_INLINE_NORMAL: &str = "notediff-inline-normal";
const TAG_FOCUS: &str = "notediff-focus";
const TAG_WORD_FOCUS: &str = "notediff-word-focus";
const TAG_INLINE_SELECTED: &str = "notediff-inline-selected";
/// Opaque foreground anchor for the custom gutter renderer (assets/notediff.css).
const LINE_NUMBER_CLASS: &str = "notediff-diff-line-number";
/// A blank changed line has nothing to paint, so the panes show a marker
/// character on it. The marker exists only in the display buffers; this
/// unstyled tag identifies it so it can be left out of copied text.
const TAG_MARKER: &str = "notediff-marker";
const BLANK_MARKER: char = '\u{b6}';
/// The marker is an annotation, not document text: keep its foreground faded
/// so it cannot be mistaken for a real character.
const MARKER_FOREGROUND_ALPHA: f32 = 0.50;
/// Width of the strong-red insertion/deletion boundary bar.
const BOUNDARY_MARKER_WIDTH: i32 = 3;

pub struct DiffPage {
    pub root: gtk::Box,
    pub left_view: sourceview5::View,
    pub right_view: sourceview5::View,
    left_buffer: sourceview5::Buffer,
    right_buffer: sourceview5::Buffer,
    left_title: gtk::Label,
    right_title: gtk::Label,
    left_time: gtk::Label,
    right_time: gtk::Label,
    document_path: RefCell<Option<PathBuf>>,
    left_status: StatusBar,
    right_status: StatusBar,
    left_scroller: gtk::ScrolledWindow,
    right_scroller: gtk::ScrolledWindow,
    inspection_left: sourceview5::View,
    inspection_right: sourceview5::View,
    inspection_left_buffer: sourceview5::Buffer,
    inspection_right_buffer: sourceview5::Buffer,
    inspection_left_scroller: gtk::ScrolledWindow,
    inspection_right_scroller: gtk::ScrolledWindow,
    inspection_left_caret: Caret,
    inspection_right_caret: Caret,
    full_left_caret: Caret,
    full_right_caret: Caret,
    left_line_numbers: LineNumberRenderer,
    right_line_numbers: LineNumberRenderer,
    left_stack: gtk::Stack,
    right_stack: gtk::Stack,
    pub previous_button: gtk::Button,
    pub next_button: gtk::Button,
    pub inspect_button: gtk::Button,
    pub accept_button: gtk::Button,
    pub accept_all_button: gtk::Button,
    pub undo_button: gtk::Button,
    pub exit_button: gtk::Button,
    scroll_pending: Cell<bool>,
    scroll_generation: Cell<u64>,
    inspection_layout: Cell<bool>,
    full_focus: Cell<Option<Hunk>>,
    inspection_generation: Cell<u64>,
    inspection_pending: Cell<bool>,
    full_revision: Cell<Option<u64>>,
    full_stats: Cell<Option<[TextStats; 2]>>,
    /// Character offsets of display-only blank-line markers in each full pane.
    left_markers: RefCell<Vec<i32>>,
    right_markers: RefCell<Vec<i32>>,
    /// Whether each pane's trailing display line belongs to its last hunk
    /// because that side gained or lost a final newline.
    left_trailing_blank: Cell<bool>,
    right_trailing_blank: Cell<bool>,
}

impl DiffPage {
    #[cfg(test)]
    pub(crate) fn assert_theme_smoke(&self, selected: &theme::LoadedTheme) {
        for buffer in [
            &self.left_buffer,
            &self.right_buffer,
            &self.inspection_left_buffer,
            &self.inspection_right_buffer,
        ] {
            assert_eq!(buffer.style_scheme(), selected.scheme);
            for (name, style) in tag_colors() {
                let tag = buffer.tag_table().lookup(name).unwrap();
                assert!(!tag.property::<bool>("weight-set"));
                let actual = tag
                    .property::<Option<gtk::gdk::RGBA>>("background-rgba")
                    .unwrap();
                assert_eq!(actual, theme::rgba(style.background));
                let fg = tag
                    .property::<Option<gtk::gdk::RGBA>>("foreground-rgba")
                    .unwrap();
                assert_eq!(fg, theme::rgba(style.foreground));
            }
            let marker = buffer.tag_table().lookup(TAG_MARKER).unwrap();
            let fg = marker
                .property::<Option<gtk::gdk::RGBA>>("foreground-rgba")
                .unwrap();
            assert!(fg.alpha() < 1.0, "the blank-line marker must stay faded");
            assert!(
                marker
                    .property::<Option<gtk::gdk::RGBA>>("background-rgba")
                    .is_none(),
                "the blank-line marker must inherit its background"
            );
        }
    }

    /// The tags on each display-only blank-line marker in a full-diff pane,
    /// with its character offset. Test-only: scans a small document.
    #[cfg(test)]
    pub(crate) fn full_marker_tags(&self, right: bool) -> Vec<(i32, Vec<String>)> {
        let buffer = if right {
            &self.right_buffer
        } else {
            &self.left_buffer
        };
        marker_tags(buffer)
    }

    #[cfg(test)]
    pub(crate) fn inspection_marker_tags(&self, right: bool) -> Vec<(i32, Vec<String>)> {
        let buffer = if right {
            &self.inspection_right_buffer
        } else {
            &self.inspection_left_buffer
        };
        marker_tags(buffer)
    }

    pub fn new() -> Rc<Self> {
        let (left_buffer, left_view, left_scroller) = make_pane();
        let (right_buffer, right_view, right_scroller) = make_pane();
        let (inspection_left_buffer, inspection_left, inspection_left_scroller) = make_pane();
        let (inspection_right_buffer, inspection_right, inspection_right_scroller) = make_pane();
        for (view, scroller) in [
            (&inspection_left, &inspection_left_scroller),
            (&inspection_right, &inspection_right_scroller),
        ] {
            view.set_wrap_mode(gtk::WrapMode::WordChar);
            view.set_show_line_numbers(false);
            scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        }
        let inspection_left_caret = Caret::new(&inspection_left);
        let inspection_right_caret = Caret::new(&inspection_right);
        // The full panes mark the empty side of a focused insertion/deletion.
        let full_left_caret = Caret::new(&left_view);
        let full_right_caret = Caret::new(&right_view);
        let line_number_style = LineNumberStyle::from_palette(&theme::current().palette);
        let left_line_numbers = LineNumberRenderer::install(&left_view, line_number_style.clone());
        let right_line_numbers = LineNumberRenderer::install(&right_view, line_number_style);
        gutter_selection::install_line_selection(&left_view);
        gutter_selection::install_line_selection(&right_view);
        let left_stack = pane_stack(&left_scroller, &inspection_left_scroller);
        let right_stack = pane_stack(&right_scroller, &inspection_right_scroller);

        let left_title = pane_title("Document");
        let right_title = pane_title("Baseline");
        let left_time = pane_time();
        let right_time = pane_time();
        let left_header = pane_header(&left_title, &left_time);
        let right_header = pane_header(&right_title, &right_time);

        let left_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let left_status = StatusBar::pane();
        left_box.append(&left_header);
        left_box.append(&left_stack);
        left_box.append(&left_status.root);
        left_box.set_hexpand(true);
        left_box.set_vexpand(true);

        let right_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let right_status = StatusBar::pane();
        right_box.append(&right_header);
        right_box.append(&right_stack);
        right_box.append(&right_status.root);
        right_box.set_hexpand(true);
        right_box.set_vexpand(true);

        // Keep equal widths after resizing; paint the divider as a CSS border.
        right_box.add_css_class("diff-pane-divider");
        let panes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        panes.set_homogeneous(true);
        panes.append(&left_box);
        panes.append(&right_box);
        panes.set_hexpand(true);
        panes.set_vexpand(true);

        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        toolbar.add_css_class("toolbar");
        toolbar.add_css_class("diff-toolbar");
        toolbar.set_margin_start(8);
        toolbar.set_margin_end(8);
        toolbar.set_margin_top(6);
        toolbar.set_margin_bottom(6);

        let previous_button = icon_button("go-up-symbolic", "Previous hunk (Up)");
        let next_button = icon_button("go-down-symbolic", "Next hunk (Down)");
        let inspect_button = gtk::Button::with_label("Inspect");
        inspect_button.set_tooltip_text(Some("Inspect selected hunk (Enter)"));
        let accept_button = gtk::Button::with_label("Accept");
        accept_button.set_tooltip_text(Some("Accept selected hunk (Right)"));
        let accept_all_button = gtk::Button::with_label("Accept All");
        accept_all_button.set_tooltip_text(Some("Accept all remaining hunks"));
        let undo_button = icon_button("edit-undo-symbolic", "Undo last accept (Left)");
        let exit_button = gtk::Button::new();
        exit_button.add_css_class("flat");
        apply_exit_state(&exit_button, ExitState::Close, 0);

        toolbar.append(&next_button);
        toolbar.append(&previous_button);
        toolbar.append(&inspect_button);
        toolbar.append(&accept_button);
        toolbar.append(&accept_all_button);
        toolbar.append(&undo_button);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        toolbar.append(&spacer);
        toolbar.append(&exit_button);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&toolbar);
        root.append(&panes);
        root.set_hexpand(true);
        root.set_vexpand(true);

        let page = Rc::new(Self {
            root,
            left_view,
            right_view,
            left_buffer,
            right_buffer,
            left_title,
            right_title,
            left_time,
            right_time,
            document_path: RefCell::new(None),
            left_status,
            right_status,
            left_scroller,
            right_scroller,
            inspection_left,
            inspection_right,
            inspection_left_buffer,
            inspection_right_buffer,
            inspection_left_scroller,
            inspection_right_scroller,
            inspection_left_caret,
            inspection_right_caret,
            full_left_caret,
            full_right_caret,
            left_line_numbers,
            right_line_numbers,
            left_stack,
            right_stack,
            previous_button,
            next_button,
            inspect_button,
            accept_button,
            accept_all_button,
            undo_button,
            exit_button,
            scroll_pending: Cell::new(false),
            scroll_generation: Cell::new(0),
            inspection_layout: Cell::new(false),
            full_focus: Cell::new(None),
            inspection_generation: Cell::new(0),
            inspection_pending: Cell::new(false),
            full_revision: Cell::new(None),
            full_stats: Cell::new(None),
            left_markers: RefCell::new(Vec::new()),
            right_markers: RefCell::new(Vec::new()),
            left_trailing_blank: Cell::new(false),
            right_trailing_blank: Cell::new(false),
        });

        for scroller in [
            &page.inspection_left_scroller,
            &page.inspection_right_scroller,
        ] {
            for adjustment in [scroller.vadjustment(), scroller.hadjustment()] {
                let weak = Rc::downgrade(&page);
                adjustment.connect_value_changed(move |_| {
                    if let Some(page) = weak.upgrade() {
                        page.reanchor_inspection_carets();
                    }
                });
                // A width change rewraps the text without changing the scroll
                // value. Re-anchor on the next tick: moving overlays while the
                // text view is still being allocated leaves them unallocated
                // during a snapshot.
                let weak = Rc::downgrade(&page);
                adjustment.connect_page_size_notify(move |_| {
                    if let Some(page) = weak.upgrade() {
                        page.schedule_caret_reanchor();
                    }
                });
            }
        }
        for scroller in [&page.left_scroller, &page.right_scroller] {
            for adjustment in [scroller.vadjustment(), scroller.hadjustment()] {
                let weak = Rc::downgrade(&page);
                adjustment.connect_value_changed(move |_| {
                    if let Some(page) = weak.upgrade() {
                        page.reanchor_full_carets();
                    }
                });
                let weak = Rc::downgrade(&page);
                adjustment.connect_page_size_notify(move |_| {
                    if let Some(page) = weak.upgrade() {
                        page.schedule_caret_reanchor();
                    }
                });
            }
        }
        for view in [&page.inspection_left, &page.inspection_right] {
            let weak = Rc::downgrade(&page);
            view.connect_map(move |_| {
                if let Some(page) = weak.upgrade() {
                    page.reanchor_inspection_carets();
                }
            });
        }
        // Returning from inspection remaps the retained full views; their
        // markers must be re-placed against the freshly laid-out text.
        for view in [&page.left_view, &page.right_view] {
            let weak = Rc::downgrade(&page);
            view.connect_map(move |_| {
                if let Some(page) = weak.upgrade() {
                    page.reanchor_full_carets();
                }
            });
        }
        {
            let weak = Rc::downgrade(&page);
            theme::on_change(move || {
                let Some(page) = weak.upgrade() else {
                    return false;
                };
                page.refresh_theme();
                true
            });
        }
        {
            let weak = Rc::downgrade(&page);
            theme::on_appearance(move || {
                let Some(page) = weak.upgrade() else {
                    return false;
                };
                page.refresh_after_style();
                true
            });
        }
        let weak = Rc::downgrade(&page);
        page.root.connect_unmap(move |_| {
            if let Some(page) = weak.upgrade() {
                page.scroll_generation
                    .set(page.scroll_generation.get().wrapping_add(1));
                page.scroll_pending.set(false);
                page.inspection_generation
                    .set(page.inspection_generation.get().wrapping_add(1));
                page.inspection_pending.set(false);
            }
        });
        page
    }

    /// Recolour the highlight tags of all four panes for the current theme.
    fn refresh_theme(&self) {
        for buffer in [
            &self.left_buffer,
            &self.right_buffer,
            &self.inspection_left_buffer,
            &self.inspection_right_buffer,
        ] {
            refresh_tag_colors(buffer);
        }
        self.update_inspection_carets();
        self.update_full_carets();
        let style = LineNumberStyle::from_palette(&theme::current().palette);
        self.left_line_numbers.set_style(style.clone());
        self.right_line_numbers.set_style(style);
        self.sync_line_number_widths();
    }

    /// Re-measure both full-diff gutters for the current font.
    fn sync_line_number_widths(&self) {
        self.left_line_numbers.sync_width(&self.left_buffer);
        self.right_line_numbers.sync_width(&self.right_buffer);
    }

    /// A CSS font change refreshes the renderer's Pango context and text
    /// geometry while the next frame is laid out, after that frame's tick
    /// callbacks. Refresh on the following tick so both use the new font.
    fn refresh_after_style(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let first = Cell::new(true);
        self.left_view.add_tick_callback(move |_, _| {
            if first.replace(false) {
                return glib::ControlFlow::Continue;
            }
            if let Some(page) = weak.upgrade() {
                page.sync_line_number_widths();
                page.update_inspection_carets();
                page.update_full_carets();
            }
            glib::ControlFlow::Break
        });
    }

    /// Re-anchor both caret pairs after the current allocation finishes. A
    /// page-size change rewraps the text and invalidates the caret's cached
    /// buffer geometry; moving the overlay during that allocation can leave it
    /// unallocated for the frame that snapshots it.
    fn schedule_caret_reanchor(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.left_view.add_tick_callback(move |_, _| {
            if let Some(page) = weak.upgrade() {
                page.reanchor_inspection_carets();
                page.reanchor_full_carets();
            }
            glib::ControlFlow::Break
        });
    }

    fn clear_carets(&self) {
        self.set_caret(&self.inspection_left, &self.inspection_left_caret, None);
        self.set_caret(&self.inspection_right, &self.inspection_right_caret, None);
    }

    fn set_caret(&self, view: &sourceview5::View, caret: &Caret, offset: Option<i32>) {
        caret.offset.set(offset);
        update_caret(view, caret);
    }

    fn update_inspection_carets(&self) {
        update_caret(&self.inspection_left, &self.inspection_left_caret);
        update_caret(&self.inspection_right, &self.inspection_right_caret);
    }

    /// Force both inspection carets to be re-placed. GtkTextView 4.18 only
    /// queues a draw when overlay scroll offsets change, so the view
    /// allocation and the overlay position both need to be refreshed on
    /// every user scroll.
    fn reanchor_inspection_carets(&self) {
        if self.inspection_left_caret.offset.get().is_none()
            && self.inspection_right_caret.offset.get().is_none()
        {
            return;
        }
        self.inspection_left.queue_allocate();
        self.inspection_right.queue_allocate();
        self.inspection_left_caret.reanchor();
        self.inspection_right_caret.reanchor();
        self.update_inspection_carets();
    }

    fn update_full_carets(&self) {
        update_caret(&self.left_view, &self.full_left_caret);
        update_caret(&self.right_view, &self.full_right_caret);
    }

    fn reanchor_full_carets(&self) {
        if self.full_left_caret.offset.get().is_none()
            && self.full_right_caret.offset.get().is_none()
        {
            return;
        }
        self.left_view.queue_allocate();
        self.right_view.queue_allocate();
        self.full_left_caret.reanchor();
        self.full_right_caret.reanchor();
        self.update_full_carets();
    }

    /// Show a boundary caret on a full pane only when the focused hunk's side
    /// is empty, so a pure insertion or deletion still has a visible location.
    fn set_full_carets(&self, hunk: Option<Hunk>) {
        let left = hunk
            .filter(|hunk| hunk.new_len == 0)
            .map(|hunk| line_start_offset(&self.left_buffer, hunk.new_start));
        let right = hunk
            .filter(|hunk| hunk.old_len == 0)
            .map(|hunk| line_start_offset(&self.right_buffer, hunk.old_start));
        self.set_caret(&self.left_view, &self.full_left_caret, left);
        self.set_caret(&self.right_view, &self.full_right_caret, right);
    }

    #[cfg(test)]
    pub(crate) fn inspection_caret(&self, right: bool) -> (bool, Option<i32>) {
        let caret = if right {
            &self.inspection_right_caret
        } else {
            &self.inspection_left_caret
        };
        caret.state()
    }

    /// True when the active caret overlay really has a mapped, sized allocation.
    #[cfg(test)]
    pub(crate) fn inspection_caret_ready(&self, right: bool) -> bool {
        let caret = if right {
            &self.inspection_right_caret
        } else {
            &self.inspection_left_caret
        };
        caret.ready()
    }

    #[cfg(test)]
    pub(crate) fn full_boundary_marker(&self, right: bool) -> (bool, Option<i32>) {
        let caret = if right {
            &self.full_right_caret
        } else {
            &self.full_left_caret
        };
        caret.state()
    }

    /// True when the full-pane boundary marker has a mapped, sized allocation.
    #[cfg(test)]
    pub(crate) fn full_boundary_marker_ready(&self, right: bool) -> bool {
        let caret = if right {
            &self.full_right_caret
        } else {
            &self.full_left_caret
        };
        caret.ready()
    }

    #[cfg(test)]
    pub(crate) fn inspection_caret_width(&self, right: bool) -> i32 {
        let caret = if right {
            &self.inspection_right_caret
        } else {
            &self.inspection_left_caret
        };
        caret.width()
    }

    /// Last applied inspection caret geometry, `(x, y, height)` in buffer
    /// coordinates.
    #[cfg(test)]
    pub(crate) fn inspection_caret_geometry(&self, right: bool) -> Option<(i32, i32, i32)> {
        let caret = if right {
            &self.inspection_right_caret
        } else {
            &self.inspection_left_caret
        };
        caret.geometry.get()
    }

    #[cfg(test)]
    pub(crate) fn full_boundary_marker_width(&self, right: bool) -> i32 {
        let caret = if right {
            &self.full_right_caret
        } else {
            &self.full_left_caret
        };
        caret.width()
    }

    #[cfg(test)]
    pub(crate) fn full_boundary_marker_geometry(&self, right: bool) -> Option<(i32, i32, i32)> {
        let caret = if right {
            &self.full_right_caret
        } else {
            &self.full_left_caret
        };
        caret.geometry.get()
    }

    #[cfg(test)]
    pub(crate) fn left_renderer_weak(&self) -> glib::WeakRef<sourceview5::GutterRendererText> {
        self.left_line_numbers.renderer.downgrade()
    }

    /// Width requests set by the last gutter measurement, left then right.
    #[cfg(test)]
    pub(crate) fn full_line_number_widths(&self) -> [i32; 2] {
        [
            self.left_line_numbers.renderer.width_request(),
            self.right_line_numbers.renderer.width_request(),
        ]
    }

    #[cfg(test)]
    pub(crate) fn line_number_color_alpha(&self, right: bool) -> f32 {
        let line_numbers = if right {
            &self.right_line_numbers
        } else {
            &self.left_line_numbers
        };
        line_numbers.renderer.color().alpha()
    }

    #[cfg(test)]
    pub(crate) fn line_number_markup_for_test(&self, right: bool, line: u32) -> String {
        let line_numbers = if right {
            &self.right_line_numbers
        } else {
            &self.left_line_numbers
        };
        let object = self.left_buffer.clone().upcast::<glib::Object>();
        line_numbers
            .renderer
            .emit_by_name::<()>("query-data", &[&object, &line]);
        line_numbers
            .renderer
            .markup()
            .map(|markup| markup.to_string())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn full_line_number_strong(&self, right: bool) -> Option<(usize, usize)> {
        let line_numbers = if right {
            &self.right_line_numbers
        } else {
            &self.left_line_numbers
        };
        line_numbers.strong.get()
    }

    /// The overlay's allocated position relative to the text view's child.
    #[cfg(test)]
    pub(crate) fn full_boundary_marker_allocation(&self, right: bool) -> Option<(i32, i32)> {
        let caret = if right {
            &self.full_right_caret
        } else {
            &self.full_left_caret
        };
        let parent = caret.area.parent()?;
        let bounds = caret.area.compute_bounds(&parent)?;
        (caret.area.is_mapped() && bounds.width() > 0.0)
            .then(|| (bounds.x() as i32, bounds.y() as i32))
    }

    #[cfg(test)]
    pub(crate) fn full_scrollers(&self) -> [&gtk::ScrolledWindow; 2] {
        [&self.left_scroller, &self.right_scroller]
    }

    pub(super) fn begin_review(&self) {
        self.full_revision.set(None);
        self.full_stats.set(None);
        self.full_focus.set(None);
        self.left_trailing_blank.set(false);
        self.right_trailing_blank.set(false);
        self.left_line_numbers.set_strong(None);
        self.right_line_numbers.set_strong(None);
    }

    pub fn active_left_view(&self) -> &sourceview5::View {
        if self.inspection_layout.get() {
            &self.inspection_left
        } else {
            &self.left_view
        }
    }

    pub fn active_right_view(&self) -> &sourceview5::View {
        if self.inspection_layout.get() {
            &self.inspection_right
        } else {
            &self.right_view
        }
    }

    pub(super) fn copy_views(&self) -> [&sourceview5::View; 4] {
        [
            &self.left_view,
            &self.right_view,
            &self.inspection_left,
            &self.inspection_right,
        ]
    }

    pub(super) fn set_focus(
        self: &Rc<Self>,
        diff: &DiffResult,
        focus: Option<usize>,
        can_accept: bool,
    ) {
        self.inspect_button.set_sensitive(can_accept);
        self.accept_button.set_sensitive(can_accept);
        self.full_focus
            .set(focus.and_then(|index| diff.hunks.get(index)).copied());
        self.apply_focus(diff, focus);
        self.scroll_to_focus(diff, focus);
    }

    pub(super) fn render(self: &Rc<Self>, session: &DiffSession) {
        self.set_headers();
        let diff = session.diff();
        let focus = session.focus();
        let toolbar = DiffToolbarState::for_session(session);
        let inspecting = toolbar.inspection.is_some();
        let focused_hunk = focus.and_then(|index| diff.hunks.get(index)).copied();
        let left_position = self.left_scroller.vadjustment().value();
        let right_position = self.right_scroller.vadjustment().value();
        // Keep the full-document buffers and views alive throughout inspection.
        // Only actual text edits invalidate their layout; a plain toggle does
        // not even remove/reapply tags or schedule a scroll.
        let content_changed = self.full_revision.replace(Some(session.content_revision()))
            != Some(session.content_revision());
        if content_changed {
            // Marked blank lines depend on the hunks, so both panes are
            // refreshed. `set_text` only rewrites the part that differs.
            let live_stats = if let Some(stats) = self.full_stats.get() {
                stats[0]
            } else {
                text_stats(session.live())
            };
            let live_line_count = crate::core::diff::split_lines(session.live()).len();
            let baseline_line_count = crate::core::diff::split_lines(session.baseline()).len();
            // The display-only line after a final newline belongs to the last
            // hunk only when that side changed content there. A pure insertion
            // or deletion leaves the other side's final newline unchanged.
            let last_hunk = diff.hunks.last();
            let live_trailing = trailing_blank_change(session.live(), session.baseline())
                && last_hunk.is_some_and(|hunk| hunk_changes_live_eof(hunk, live_line_count));
            let baseline_trailing = trailing_blank_change(session.baseline(), session.live())
                && last_hunk
                    .is_some_and(|hunk| hunk_changes_baseline_eof(hunk, baseline_line_count));
            self.left_trailing_blank.set(live_trailing);
            self.right_trailing_blank.set(baseline_trailing);
            let new_ranges: Vec<Range<usize>> = diff
                .hunks
                .iter()
                .map(|h| h.new_start..h.new_end())
                .collect();
            let old_ranges: Vec<Range<usize>> = diff
                .hunks
                .iter()
                .map(|h| h.old_start..h.old_end())
                .collect();
            let (_, live_markers) = set_marked_text(
                &self.left_buffer,
                session.live(),
                &new_ranges,
                live_trailing,
            );
            let (_, baseline_markers) = set_marked_text(
                &self.right_buffer,
                session.baseline(),
                &old_ranges,
                baseline_trailing,
            );
            self.left_markers.replace(live_markers);
            self.right_markers.replace(baseline_markers);
            self.sync_line_number_widths();
            self.full_stats
                .set(Some([live_stats, text_stats(session.baseline())]));
            clear_tags(&self.left_buffer);
            clear_tags(&self.right_buffer);
            for hunk in &diff.hunks {
                let live_end = hunk.new_end()
                    + usize::from(live_trailing && hunk.new_end() == live_line_count);
                let baseline_end = hunk.old_end()
                    + usize::from(baseline_trailing && hunk.old_end() == baseline_line_count);
                tag_line_range(&self.left_buffer, hunk.new_start, live_end, TAG_HUNK);
                tag_line_range(&self.right_buffer, hunk.old_start, baseline_end, TAG_HUNK);
            }
            tag_inline(
                &self.left_buffer,
                &diff.live_inline,
                0..usize::MAX,
                TAG_WORD,
            );
            tag_inline(
                &self.right_buffer,
                &diff.baseline_inline,
                0..usize::MAX,
                TAG_WORD,
            );
            tag_markers(
                &self.left_buffer,
                &self.left_markers.borrow(),
                0..usize::MAX,
                TAG_WORD,
            );
            tag_markers(
                &self.right_buffer,
                &self.right_markers.borrow(),
                0..usize::MAX,
                TAG_WORD,
            );
        }
        let focus_changed = self.full_focus.replace(focused_hunk) != focused_hunk;
        if content_changed || focus_changed {
            self.apply_focus(diff, focus);
            let targets = match focused_hunk {
                Some(hunk) => [
                    ScrollTarget::Line(hunk.new_start),
                    ScrollTarget::Line(hunk.old_start),
                ],
                None => [
                    ScrollTarget::Position(left_position),
                    ScrollTarget::Position(right_position),
                ],
            };
            // Validate and position the retained views *before* revealing them,
            // including when inspection navigation jumps to another hunk.
            self.schedule_scroll(targets);
        }

        if let Some(index) = toolbar.inspection {
            self.clear_carets();
            let hunk = &diff.hunks[index];
            let live = hunk_text(session.live(), hunk.new_start, hunk.new_len);
            let baseline = hunk_text(session.baseline(), hunk.old_start, hunk.old_len);
            let live_trailing = self.left_trailing_blank.get()
                && hunk.new_end() == crate::core::diff::split_lines(session.live()).len()
                && has_trailing_display_line(live);
            let baseline_trailing = self.right_trailing_blank.get()
                && hunk.old_end() == crate::core::diff::split_lines(session.baseline()).len()
                && has_trailing_display_line(baseline);
            let whole = 0..usize::MAX;
            let whole = std::slice::from_ref(&whole);
            let (live_display, live_markers) =
                set_marked_text(&self.inspection_left_buffer, live, whole, live_trailing);
            let (baseline_display, baseline_markers) = set_marked_text(
                &self.inspection_right_buffer,
                baseline,
                whole,
                baseline_trailing,
            );
            clear_tags(&self.inspection_left_buffer);
            clear_tags(&self.inspection_right_buffer);
            // The inspected hunk is focused, so its unselected blank-line
            // markers use the focused-hunk layer; a selected change overlays
            // the focused-word layer later.
            tag_markers(
                &self.inspection_left_buffer,
                &live_markers,
                0..usize::MAX,
                TAG_INLINE_NORMAL,
            );
            tag_markers(
                &self.inspection_right_buffer,
                &baseline_markers,
                0..usize::MAX,
                TAG_INLINE_NORMAL,
            );
            let live_word_ranges = tag_excerpt(
                &self.inspection_left_buffer,
                &live_display,
                hunk.new_start,
                &diff.live_inline,
            );
            let baseline_word_ranges = tag_excerpt(
                &self.inspection_right_buffer,
                &baseline_display,
                hunk.old_start,
                &diff.baseline_inline,
            );
            self.left_status.set_stats(&text_stats(live));
            self.right_status.set_stats(&text_stats(baseline));
            if let Some(change) = session.selected_inline() {
                let (left, left_point) = select_excerpt(
                    &self.inspection_left_buffer,
                    live,
                    &change.new,
                    line_byte_start(session.live(), hunk.new_start),
                    live_trailing,
                    change.kind,
                    &live_word_ranges,
                );
                let (right, right_point) = select_excerpt(
                    &self.inspection_right_buffer,
                    baseline,
                    &change.old,
                    line_byte_start(session.baseline(), hunk.old_start),
                    baseline_trailing,
                    change.kind,
                    &baseline_word_ranges,
                );
                self.set_caret(
                    &self.inspection_left,
                    &self.inspection_left_caret,
                    left_point,
                );
                self.set_caret(
                    &self.inspection_right,
                    &self.inspection_right_caret,
                    right_point,
                );
                self.scroll_inspection([ScrollTarget::Offset(left), ScrollTarget::Offset(right)]);
            }
        } else {
            self.clear_carets();
            let stats = self.full_stats.get().unwrap();
            self.left_status.set_stats(&stats[0]);
            self.right_status.set_stats(&stats[1]);
        }
        self.set_inspection_layout(inspecting);
        self.update_toolbar(toolbar);
    }

    fn set_inspection_layout(&self, inspecting: bool) {
        if self.inspection_layout.replace(inspecting) == inspecting {
            return;
        }
        let child = if inspecting { "inspection" } else { "full" };
        self.left_stack.set_visible_child_name(child);
        self.right_stack.set_visible_child_name(child);
        if !inspecting {
            self.inspection_generation
                .set(self.inspection_generation.get().wrapping_add(1));
            self.inspection_pending.set(false);
        }
    }

    /// Refresh controls without touching buffers, highlights or scroll position.
    pub(super) fn update_toolbar(&self, toolbar: DiffToolbarState) {
        for button in [
            &self.previous_button,
            &self.next_button,
            &self.accept_all_button,
        ] {
            button.set_sensitive(toolbar.has_pending);
        }
        let inspecting = toolbar.inspection.is_some();
        self.accept_button.set_tooltip_text(Some(if inspecting {
            "Accept selected change (Right)"
        } else {
            "Accept selected hunk (Right)"
        }));
        self.previous_button.set_tooltip_text(Some(if inspecting {
            "Previous change (Up)"
        } else {
            "Previous hunk (Up)"
        }));
        self.next_button.set_tooltip_text(Some(if inspecting {
            "Next change (Down)"
        } else {
            "Next hunk (Down)"
        }));
        self.inspect_button.set_visible(!inspecting);
        self.inspect_button.set_sensitive(toolbar.can_accept);
        self.accept_all_button.set_visible(!inspecting);
        self.accept_button.set_sensitive(toolbar.can_accept);
        self.undo_button.set_sensitive(toolbar.can_undo);
        apply_exit_state(&self.exit_button, toolbar.exit, toolbar.accepted_hunks);
    }

    /// Move the reddish-orange focus highlight (line and word level) to the
    /// focused hunk, leaving every other hunk in the yellowish base colors.
    fn apply_focus(&self, diff: &DiffResult, focus: Option<usize>) {
        for buffer in [&self.left_buffer, &self.right_buffer] {
            for tag in [TAG_FOCUS, TAG_WORD_FOCUS] {
                buffer.remove_tag_by_name(tag, &buffer.start_iter(), &buffer.end_iter());
            }
        }

        let Some(hunk) = focus.and_then(|index| diff.hunks.get(index)) else {
            self.set_full_carets(None);
            self.left_line_numbers.set_strong(None);
            self.right_line_numbers.set_strong(None);
            return;
        };
        self.set_full_carets(Some(*hunk));
        let live_trailing = self.left_trailing_blank.get()
            && hunk.new_end() + 1 == self.left_buffer.line_count() as usize;
        let baseline_trailing = self.right_trailing_blank.get()
            && hunk.old_end() + 1 == self.right_buffer.line_count() as usize;
        let live_lines = hunk.new_start..hunk.new_end() + usize::from(live_trailing);
        let baseline_lines = hunk.old_start..hunk.old_end() + usize::from(baseline_trailing);
        self.left_line_numbers.set_strong(strong_line_range(
            &self.left_buffer,
            live_lines.start,
            live_lines.end,
            hunk.new_len == 0,
        ));
        self.right_line_numbers.set_strong(strong_line_range(
            &self.right_buffer,
            baseline_lines.start,
            baseline_lines.end,
            hunk.old_len == 0,
        ));

        tag_line_range(
            &self.left_buffer,
            live_lines.start,
            live_lines.end,
            TAG_FOCUS,
        );
        tag_line_range(
            &self.right_buffer,
            baseline_lines.start,
            baseline_lines.end,
            TAG_FOCUS,
        );
        tag_inline(
            &self.left_buffer,
            &diff.live_inline,
            live_lines.clone(),
            TAG_WORD_FOCUS,
        );
        tag_inline(
            &self.right_buffer,
            &diff.baseline_inline,
            baseline_lines.clone(),
            TAG_WORD_FOCUS,
        );
        tag_markers(
            &self.left_buffer,
            &self.left_markers.borrow(),
            live_lines,
            TAG_WORD_FOCUS,
        );
        tag_markers(
            &self.right_buffer,
            &self.right_markers.borrow(),
            baseline_lines,
            TAG_WORD_FOCUS,
        );
    }

    /// Move each pane to its own hunk anchor and reset horizontal scrolling.
    fn scroll_to_focus(self: &Rc<Self>, diff: &DiffResult, focus: Option<usize>) {
        let Some(hunk) = focus.and_then(|index| diff.hunks.get(index)) else {
            return;
        };
        self.schedule_scroll([
            ScrollTarget::Line(hunk.new_start),
            ScrollTarget::Line(hunk.old_start),
        ]);
    }

    /// Place each pane independently after layout. Keep placement pending until
    /// geometry and placement agree across consecutive frames, not just one idle.
    /// Weak ownership and a generation ensure obsolete requests cannot win.
    fn schedule_scroll(self: &Rc<Self>, targets: [ScrollTarget; 2]) {
        let generation = self.scroll_generation.get().wrapping_add(1);
        self.scroll_generation.set(generation);
        self.scroll_pending.set(true);
        // Hunk navigation starts both panes at the left edge. Other requests
        // keep whatever horizontal position each pane has.
        if targets
            .iter()
            .all(|target| matches!(target, ScrollTarget::Line(_)))
        {
            self.left_scroller.hadjustment().set_value(0.0);
            self.right_scroller.hadjustment().set_value(0.0);
        }
        self.place_full_scroll(targets);
        let weak = Rc::downgrade(self);
        let previous = RefCell::new(None);
        self.root.add_tick_callback(move |_, _| {
            let Some(page) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if page.scroll_generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            let geometry = page.place_full_scroll(targets);
            if previous.borrow().as_ref() == Some(&geometry) {
                page.scroll_pending.set(false);
                glib::ControlFlow::Break
            } else {
                previous.replace(Some(geometry));
                glib::ControlFlow::Continue
            }
        });
    }

    fn place_full_scroll(&self, targets: [ScrollTarget; 2]) -> Vec<f64> {
        let geometry = place_scroll(
            [
                (&self.left_view, &self.left_scroller),
                (&self.right_view, &self.right_scroller),
            ],
            targets,
        );
        // `place_scroll` validates the retained layout up to each anchor. This
        // is the first point after a content edit where marker placement sees
        // the new line geometry, so refresh the boundary carets here.
        self.update_full_carets();
        geometry
    }

    fn scroll_inspection(self: &Rc<Self>, targets: [ScrollTarget; 2]) {
        let generation = self.inspection_generation.get().wrapping_add(1);
        self.inspection_generation.set(generation);
        self.inspection_pending.set(true);
        self.inspection_left_scroller.hadjustment().set_value(0.0);
        self.inspection_right_scroller.hadjustment().set_value(0.0);
        let weak = Rc::downgrade(self);
        let previous = RefCell::new(None);
        self.root.add_tick_callback(move |_, _| {
            let Some(page) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if page.inspection_generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            let geometry = place_scroll(
                [
                    (&page.inspection_left, &page.inspection_left_scroller),
                    (&page.inspection_right, &page.inspection_right_scroller),
                ],
                targets,
            );
            if previous.borrow().as_ref() == Some(&geometry) {
                page.inspection_pending.set(false);
                // The caret may have been outside the viewport when it was
                // positioned; ask for its region once scrolling has settled.
                page.update_inspection_carets();
                glib::ControlFlow::Break
            } else {
                previous.replace(Some(geometry));
                glib::ControlFlow::Continue
            }
        });
    }

    #[cfg(test)]
    pub fn inspection_active(&self) -> bool {
        self.inspection_layout.get()
    }

    #[cfg(test)]
    pub fn panes_ready(&self) -> bool {
        if self.inspection_layout.get() {
            !self.inspection_pending.get()
        } else {
            !self.scroll_pending.get()
        }
    }

    #[cfg(test)]
    pub fn panes_presented(&self) -> bool {
        self.active_left_view().is_mapped() && self.active_right_view().is_mapped()
    }

    #[cfg(test)]
    pub fn full_scroll_requests(&self) -> u64 {
        self.scroll_generation.get()
    }

    /// Move both panes to the top-left corner for a new review or excerpt.
    /// Use layout-aware placement so this supersedes older scroll requests.
    pub fn reset_scroll(self: &Rc<Self>) {
        self.left_scroller.hadjustment().set_value(0.0);
        self.right_scroller.hadjustment().set_value(0.0);
        self.schedule_scroll([ScrollTarget::Position(0.0); 2]);
    }

    #[cfg(test)]
    pub fn header_texts(&self) -> (String, String) {
        (
            self.left_title.text().to_string(),
            self.right_title.text().to_string(),
        )
    }

    #[cfg(test)]
    pub fn header_times(&self) -> (String, String) {
        (
            self.left_time.text().to_string(),
            self.right_time.text().to_string(),
        )
    }

    #[cfg(test)]
    pub fn header_tooltips(&self) -> (Option<String>, Option<String>) {
        (
            self.left_time.tooltip_text().map(|text| text.to_string()),
            self.right_time.tooltip_text().map(|text| text.to_string()),
        )
    }

    #[cfg(test)]
    pub fn document_path_tooltip(&self) -> Option<String> {
        self.left_title.tooltip_text().map(|text| text.to_string())
    }

    /// The live document path is shown in the left header for this review.
    pub(super) fn set_document_path(&self, path: Option<&Path>) {
        self.document_path.replace(path.map(Path::to_path_buf));
    }

    /// The live side's timestamp and whether it came from an in-memory edit
    /// rather than the file's last save.
    pub(super) fn set_document_time(&self, at: Option<SystemTime>, edited: bool) {
        let tooltip = if edited { "Last edit" } else { "Last saved" };
        set_time_label(&self.left_time, at, tooltip);
    }

    pub(super) fn set_baseline_time(&self, at: Option<SystemTime>) {
        set_time_label(&self.right_time, at, "Last saved");
    }

    /// Pane headers stay the same in full diff and inspection, even on empty
    /// sides. The left title shortens the home directory, with the full
    /// canonical path as its tooltip, or "Document" if no path was supplied.
    fn set_headers(&self) {
        let path = self.document_path.borrow();
        let text = path.as_deref().map(|path| path.display().to_string());
        let display = path
            .as_deref()
            .map(|path| super::display_path(path, &glib::home_dir()));
        self.left_title
            .set_text(display.as_deref().unwrap_or("Document"));
        self.left_title.set_tooltip_text(text.as_deref());
        self.right_title.set_text("Baseline");
    }
}

fn pane_stack(full: &gtk::ScrolledWindow, inspection: &gtk::ScrolledWindow) -> gtk::Stack {
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::None);
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    stack.add_named(full, Some("full"));
    stack.add_named(inspection, Some("inspection"));
    stack.set_visible_child_name("full");
    stack
}

fn make_pane() -> (sourceview5::Buffer, sourceview5::View, gtk::ScrolledWindow) {
    // notediff renders every document as plain text: no language is attached.
    let buffer = sourceview5::Buffer::new(None);
    buffer.set_highlight_syntax(false);
    theme::apply_scheme(&buffer);

    install_tags(&buffer);

    let view = sourceview5::View::with_buffer(&buffer);
    theme::apply_content_view(&view);
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_monospace(true);
    view.set_show_line_numbers(true);
    view.set_highlight_current_line(false);
    // Keep lines aligned vertically; long lines scroll horizontally.
    view.set_wrap_mode(gtk::WrapMode::None);
    view.set_left_margin(8);
    view.set_top_margin(6);
    view.set_bottom_margin(6);
    view.set_hexpand(true);
    view.set_vexpand(true);
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_child(Some(&view));
    scroller.set_hexpand(true);
    scroller.set_vexpand(true);
    scroller.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);

    (buffer, view, scroller)
}

fn pane_title(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_halign(gtk::Align::Start);
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    label.set_margin_start(10);
    label.set_margin_top(4);
    label.set_margin_bottom(4);
    label.add_css_class("dim-label");
    label.add_css_class("caption-heading");
    label
}

fn pane_header(title: &gtk::Label, time: &gtk::Label) -> gtk::Box {
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.set_hexpand(true);
    title.set_hexpand(true);
    header.append(title);
    header.append(time);
    header
}

fn pane_time() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_halign(gtk::Align::End);
    label.set_valign(gtk::Align::Center);
    label.set_margin_end(10);
    label.set_margin_top(4);
    label.set_margin_bottom(4);
    label.add_css_class("dim-label");
    label.add_css_class("caption");
    label.set_visible(false);
    label
}

fn set_time_label(label: &gtk::Label, at: Option<SystemTime>, tooltip: &str) {
    let text = at.and_then(format_time);
    label.set_text(text.as_deref().unwrap_or(""));
    label.set_tooltip_text(text.as_ref().map(|_| tooltip));
    label.set_visible(text.is_some());
}

fn format_time(at: SystemTime) -> Option<String> {
    let secs = at.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let secs = i64::try_from(secs).ok()?;
    let date = glib::DateTime::from_unix_local(secs).ok()?;
    date.format("%Y-%m-%d %H:%M")
        .ok()
        .map(|text| text.to_string())
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.add_css_class("flat");
    button.set_tooltip_text(Some(tooltip));
    button
}

fn apply_exit_state(button: &gtk::Button, state: ExitState, accepted: usize) {
    button.set_label(&state.label(accepted));
    button.set_tooltip_text(state.tooltip());
    if state == ExitState::ConfirmAll {
        button.add_css_class("success");
    } else {
        button.remove_css_class("success");
    }
}

/// Faded foreground for the display-only blank-line marker. It inherits the
/// hunk/word background underneath, so only the glyph is dimmed.
fn marker_color() -> gtk::gdk::RGBA {
    let mut color = theme::rgba(theme::current().palette.hunk.foreground);
    color.set_alpha(MARKER_FOREGROUND_ALPHA);
    color
}

/// Tag colours in ascending priority: focused tags paint over base tags.
fn tag_colors() -> [(&'static str, super::theme_palette::Highlight); 6] {
    let current = theme::current();
    let theme = &current.palette;
    [
        (TAG_HUNK, theme.hunk.clone()),
        (TAG_WORD, theme.word.clone()),
        (TAG_INLINE_NORMAL, theme.focused_hunk.clone()),
        (TAG_FOCUS, theme.focused_hunk.clone()),
        (TAG_WORD_FOCUS, theme.focused_word.clone()),
        (TAG_INLINE_SELECTED, theme.focused_word.clone()),
    ]
}

fn refresh_tag_colors(buffer: &sourceview5::Buffer) {
    let table = buffer.tag_table();
    for (name, style) in tag_colors() {
        if let Some(tag) = table.lookup(name) {
            apply_tag_style(&tag, &style);
        }
    }
    if let Some(tag) = table.lookup(TAG_MARKER) {
        tag.set_foreground_rgba(Some(&marker_color()));
    }
}

fn install_tags(buffer: &sourceview5::Buffer) {
    let table = buffer.tag_table();
    let tags = tag_colors();

    for (name, style) in tags {
        let tag = gtk::TextTag::new(Some(name));
        apply_tag_style(&tag, &style);
        // Review tags change colour only; they never add font weight.
        table.add(&tag);
        tag.set_priority(table.size() - 1);
    }
    table.add(&gtk::TextTag::new(Some(TAG_MARKER)));
    if let Some(marker) = table.lookup(TAG_MARKER) {
        marker.set_foreground_rgba(Some(&marker_color()));
        marker.set_priority(table.size() - 1);
    }
}

fn apply_tag_style(tag: &gtk::TextTag, style: &super::theme_palette::Highlight) {
    tag.set_background_rgba(Some(&theme::rgba(style.background)));
    tag.set_foreground_rgba(Some(&theme::rgba(style.foreground)));
}

/// `text` with a marker added to each blank line in `ranges` (LF-based line
/// ranges). A `trailing` marker also covers the display-only line after a
/// final newline. Also returns the marker positions as character offsets.
fn marked_text(text: &str, ranges: &[Range<usize>], trailing: bool) -> (String, Vec<i32>) {
    if trailing {
        debug_assert!(
            has_trailing_display_line(text),
            "a trailing marker needs a final line break"
        );
    }
    let mut out = String::with_capacity(text.len() + 8);
    let mut markers = Vec::new();
    let mut chars = 0i32;
    let mut next = 0;
    for (index, line) in crate::core::diff::split_lines(text).into_iter().enumerate() {
        while next < ranges.len() && ranges[next].end <= index {
            next += 1;
        }
        let in_hunk = ranges.get(next).is_some_and(|r| r.contains(&index));
        if in_hunk && crate::core::diff::line_content(line).is_empty() {
            out.push(BLANK_MARKER);
            markers.push(chars);
            chars += 1;
        }
        out.push_str(line);
        chars += line.chars().count() as i32;
    }
    if trailing {
        out.push(BLANK_MARKER);
        markers.push(chars);
    }
    (out, markers)
}

/// Text-level part of the trailing display-line rule: this side ends with a
/// line break and the other side does not. The caller also requires the last
/// hunk to change content on this side.
fn trailing_blank_change(text: &str, other: &str) -> bool {
    has_trailing_display_line(text) && !crate::core::textlines::ends_with_break(other)
}

/// True when the last hunk changes content on the live side at the end of the
/// text, so the display-only line after a final newline belongs to it.
fn hunk_changes_live_eof(hunk: &Hunk, line_count: usize) -> bool {
    hunk.new_len > 0 && hunk.new_end() == line_count
}

/// True when the last hunk changes content on the baseline side at the end of
/// the text, so the display-only line after a final newline belongs to it.
fn hunk_changes_baseline_eof(hunk: &Hunk, line_count: usize) -> bool {
    hunk.old_len > 0 && hunk.old_end() == line_count
}

/// True when `text` ends with a line break and therefore has one extra
/// display-only line after it.
fn has_trailing_display_line(text: &str) -> bool {
    crate::core::textlines::ends_with_break(text)
}

/// Show `text` with blank-line markers, returning the displayed text and the
/// marker character offsets.
fn set_marked_text(
    buffer: &sourceview5::Buffer,
    text: &str,
    ranges: &[Range<usize>],
    trailing: bool,
) -> (String, Vec<i32>) {
    let (display, markers) = marked_text(text, ranges, trailing);
    set_text(buffer, &display);
    let marker = buffer.tag_table().lookup(TAG_MARKER);
    if let Some(tag) = marker {
        buffer.remove_tag(&tag, &buffer.start_iter(), &buffer.end_iter());
        for &offset in &markers {
            buffer.apply_tag(
                &tag,
                &buffer.iter_at_offset(offset),
                &buffer.iter_at_offset(offset + 1),
            );
        }
    }
    (display, markers)
}

/// Tags on each display-only marker in `buffer`, with its character offset.
#[cfg(test)]
fn marker_tags(buffer: &sourceview5::Buffer) -> Vec<(i32, Vec<String>)> {
    let Some(marker) = buffer.tag_table().lookup(TAG_MARKER) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut iter = buffer.start_iter();
    while !iter.is_end() {
        if iter.has_tag(&marker) {
            found.push((
                iter.offset(),
                iter.tags()
                    .iter()
                    .filter_map(|tag| tag.name())
                    .map(|name| name.to_string())
                    .collect(),
            ));
        }
        iter.forward_char();
    }
    found
}

/// Character offset in the displayed (fully marked) excerpt of a raw byte
/// offset: every blank line starting before it contributes one marker. When
/// requested, a byte at the end also includes the display-only marker after a
/// final newline.
fn display_offset(raw: &str, byte: usize, include_trailing: bool) -> i32 {
    let mut markers = 0;
    let mut start = 0;
    for line in crate::core::diff::split_lines(raw) {
        if start >= byte {
            break;
        }
        if crate::core::diff::line_content(line).is_empty() {
            markers += 1;
        }
        start += line.len();
    }
    raw[..byte].chars().count() as i32 + markers + i32::from(include_trailing && byte == raw.len())
}

/// The selected text without display-only markers.
pub(crate) fn selected_text(buffer: &gtk::TextBuffer) -> Option<String> {
    let (start, end) = buffer.selection_bounds()?;
    let marker = buffer.tag_table().lookup(TAG_MARKER);
    let mut out = String::new();
    let mut iter = start;
    while iter < end {
        let ch = iter.char();
        if !marker.as_ref().is_some_and(|tag| iter.has_tag(tag)) {
            out.push(ch);
        }
        iter.forward_char();
    }
    Some(out)
}

fn set_text(buffer: &sourceview5::Buffer, text: &str) {
    let previous = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
    let Some((range, replacement)) = replacement_span(&previous, text) else {
        return;
    };
    // Retain the unchanged prefix/suffix, including their layout and marks.
    // Clearing the whole baseline makes GTK temporarily lose its viewport
    // anchor and repaint unrelated content before scroll placement settles.
    let mut start = buffer.iter_at_offset(range.start);
    let mut end = buffer.iter_at_offset(range.end);
    if range.start != range.end {
        buffer.delete(&mut start, &mut end);
    }
    if !replacement.is_empty() {
        buffer.insert(&mut start, replacement);
    }
}

fn clear_tags(buffer: &sourceview5::Buffer) {
    for tag in [
        TAG_HUNK,
        TAG_WORD,
        TAG_INLINE_NORMAL,
        TAG_FOCUS,
        TAG_WORD_FOCUS,
        TAG_INLINE_SELECTED,
    ] {
        buffer.remove_tag_by_name(tag, &buffer.start_iter(), &buffer.end_iter());
    }
}

/// The smallest contiguous replacement, using GTK character offsets and a
/// UTF-8-safe slice of the new text. Prefix and suffix must not overlap.
fn replacement_span<'a>(old: &str, new: &'a str) -> Option<(Range<i32>, &'a str)> {
    if old == new {
        return None;
    }
    let prefix: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let suffix: usize = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let start = old[..prefix].chars().count() as i32;
    let end = start + old[prefix..old.len() - suffix].chars().count() as i32;
    Some((start..end, &new[prefix..new.len() - suffix]))
}

fn tag_line_range(
    buffer: &sourceview5::Buffer,
    start_line: usize,
    end_line: usize,
    tag_name: &str,
) {
    if start_line >= end_line {
        return;
    }
    let line_count = buffer.line_count() as usize;
    let Some(start) = buffer.iter_at_line(start_line as i32) else {
        return;
    };
    let end = if end_line >= line_count {
        buffer.end_iter()
    } else {
        match buffer.iter_at_line(end_line as i32) {
            Some(iter) => iter,
            None => return,
        }
    };
    if let Some(tag) = buffer.tag_table().lookup(tag_name) {
        buffer.apply_tag(&tag, &start, &end);
    }
}

/// Apply `tag_name` to every span whose line falls within `lines`.
fn tag_inline(
    buffer: &sourceview5::Buffer,
    spans: &[InlineSpan],
    lines: Range<usize>,
    tag_name: &str,
) {
    let Some(tag) = buffer.tag_table().lookup(tag_name) else {
        return;
    };
    for span in spans.iter().filter(|span| lines.contains(&span.line)) {
        let Some(start) = buffer.iter_at_line_offset(span.line as i32, span.start as i32) else {
            continue;
        };
        let Some(end) = buffer.iter_at_line_offset(span.line as i32, span.end as i32) else {
            continue;
        };
        if start.offset() < end.offset() {
            buffer.apply_tag(&tag, &start, &end);
        }
    }
}

/// Apply `tag_name` to each display-only marker on one of the given display
/// lines. A marker is a single character, so the range is always one char.
fn tag_markers(buffer: &sourceview5::Buffer, markers: &[i32], lines: Range<usize>, tag_name: &str) {
    let Some(tag) = buffer.tag_table().lookup(tag_name) else {
        return;
    };
    for &offset in markers {
        let start = buffer.iter_at_offset(offset);
        if !lines.contains(&(start.line() as usize)) {
            continue;
        }
        let mut end = start;
        end.forward_char();
        buffer.apply_tag(&tag, &start, &end);
    }
}

/// Slice a hunk using the diff engine's GTK-compatible line coordinates.
/// Terminators are preserved, including CRLF, lone CR, U+2029 and an
/// unterminated final line.
fn hunk_text(text: &str, start: usize, len: usize) -> &str {
    crate::core::textlines::slice_lines(text, start, len)
}

/// Find a byte boundary without using GTK's paragraph coordinates.
fn line_byte_start(text: &str, line: usize) -> usize {
    crate::core::textlines::line_start_byte(text, line)
}

/// Character offset of the start of a display line. Past the last line, as
/// with an insertion at the end of the document, use the buffer's end.
fn line_start_offset(buffer: &sourceview5::Buffer, line: usize) -> i32 {
    i32::try_from(line)
        .ok()
        .and_then(|line| buffer.iter_at_line(line))
        .unwrap_or_else(|| buffer.end_iter())
        .offset()
}

#[derive(Clone)]
struct LineNumberStyle {
    normal: String,
    strong: String,
    strong_bold: bool,
}

impl LineNumberStyle {
    fn from_palette(palette: &super::theme_palette::Palette) -> Self {
        Self {
            normal: palette.line_number.clone(),
            strong: palette.line_number_strong.clone(),
            strong_bold: palette.line_number_strong_bold,
        }
    }
}

/// Replacement line-number renderer for the full diff panes. The stock
/// GtkSourceView renderer can only emphasise the cursor line; review
/// selections need an arbitrary set of display rows.
struct LineNumberRenderer {
    renderer: sourceview5::GutterRendererText,
    strong: Rc<Cell<Option<(usize, usize)>>>,
    style: Rc<RefCell<LineNumberStyle>>,
}

impl LineNumberRenderer {
    fn install(view: &sourceview5::View, style: LineNumberStyle) -> Self {
        view.set_show_line_numbers(false);
        let renderer = sourceview5::GutterRendererText::new();
        renderer.add_css_class(LINE_NUMBER_CLASS);
        renderer.set_alignment_mode(sourceview5::GutterRendererAlignmentMode::First);
        renderer.set_xalign(1.0);
        renderer.set_yalign(0.5);
        renderer.set_xpad(6);
        renderer.set_visible(true);

        let strong = Rc::new(Cell::new(None));
        let style = Rc::new(RefCell::new(style));
        let query_strong = strong.clone();
        let query_style = style.clone();
        renderer.connect_query_data(move |renderer, _object, line| {
            let line = line as usize;
            let is_strong = query_strong
                .get()
                .is_some_and(|(start, end)| line >= start && line < end);
            renderer.set_markup(&line_number_markup(
                line + 1,
                is_strong,
                &query_style.borrow(),
            ));
        });
        sourceview5::prelude::ViewExt::gutter(view, gtk::TextWindowType::Left).insert(&renderer, 0);

        let line_numbers = Self {
            renderer,
            strong,
            style,
        };
        line_numbers.sync_width(&view.buffer());
        line_numbers
    }

    /// Emphasise display rows in the half-open range, or clear the emphasis.
    fn set_strong(&self, range: Option<(usize, usize)>) {
        self.strong.set(range);
        self.renderer.queue_draw();
    }

    fn set_style(&self, style: LineNumberStyle) {
        *self.style.borrow_mut() = style;
        self.renderer.queue_draw();
    }

    /// Keep the gutter wide enough for the largest line number. The renderer
    /// measures the line number set by the last query, so reserve the widest
    /// digit count as a minimum width instead.
    fn sync_width(&self, buffer: &impl IsA<gtk::TextBuffer>) {
        let buffer = buffer.upcast_ref::<gtk::TextBuffer>();
        let digits = buffer.line_count().max(99).to_string().len().max(2);
        let sample = "9".repeat(digits);
        let (width, _) =
            sourceview5::prelude::GutterRendererTextExt::measure(&self.renderer, &sample);
        self.renderer
            .set_size_request(width + self.renderer.xpad() * 2, -1);
    }
}

fn line_number_markup(number: usize, strong: bool, style: &LineNumberStyle) -> String {
    let color = if strong { &style.strong } else { &style.normal };
    if strong && style.strong_bold {
        format!(r#"<span foreground="{color}" weight="bold">{number}</span>"#)
    } else {
        format!(r#"<span foreground="{color}">{number}</span>"#)
    }
}

/// Rows whose line numbers should use the strong colour. An empty hunk side
/// marks its single boundary row; a non-empty side marks its whole range.
fn strong_line_range(
    buffer: &sourceview5::Buffer,
    start_line: usize,
    end_line: usize,
    empty_side: bool,
) -> Option<(usize, usize)> {
    if empty_side {
        let offset = line_start_offset(buffer, start_line);
        let iter = buffer.iter_at_offset(offset.clamp(0, buffer.char_count()));
        let line = iter.line() as usize;
        (line < buffer.line_count() as usize).then_some((line, line + 1))
    } else {
        let count = buffer.line_count() as usize;
        let start = start_line.min(count);
        let end = end_line.min(count);
        (end > start).then_some((start, end))
    }
}

struct Caret {
    area: gtk::DrawingArea,
    /// Logical caret position; `None` means no caret on this side.
    offset: Cell<Option<i32>>,
    /// Whether the shared draw function should paint anything.
    paint: Rc<Cell<bool>>,
    /// Last applied buffer geometry, so scrolling queues no needless redraws.
    geometry: Cell<Option<(i32, i32, i32)>>,
}

impl Caret {
    fn new(view: &sourceview5::View) -> Self {
        let area = gtk::DrawingArea::new();
        area.set_content_width(BOUNDARY_MARKER_WIDTH);
        area.set_content_height(1);
        area.set_can_target(false);
        area.set_can_focus(false);
        // Keep the area mapped for the view's whole life. Toggling visibility
        // can drop the repaint of the frame that shows it again, which leaves
        // the caret invisible until some unrelated repaint. Painting nothing
        // while inactive is enough to hide it.
        let paint = Rc::new(Cell::new(false));
        let active = paint.clone();
        area.set_draw_func(move |_, cr, width, height| {
            if !active.get() {
                return;
            }
            let current = theme::current();
            let color = theme::rgba(current.palette.focused_word.background);
            cr.set_source_rgba(
                color.red() as f64,
                color.green() as f64,
                color.blue() as f64,
                color.alpha() as f64,
            );
            cr.rectangle(0.0, 0.0, width as f64, height as f64);
            let _ = cr.fill();
        });
        area.set_visible(true);
        view.add_overlay(&area, 0, 0);
        Self {
            area,
            offset: Cell::new(None),
            paint,
            geometry: Cell::new(None),
        }
    }

    #[cfg(test)]
    fn state(&self) -> (bool, Option<i32>) {
        (self.paint.get(), self.offset.get())
    }

    /// Forget the last applied geometry so the next update calls
    /// `move_overlay` again. GTK 4.18 only queues a draw when a text view's
    /// overlay scroll offset changes, so a fresh move is needed to reallocate
    /// the overlay on user scrolling.
    fn reanchor(&self) {
        self.geometry.set(None);
    }

    #[cfg(test)]
    fn ready(&self) -> bool {
        self.paint.get() && self.area.is_mapped() && self.area.width() > 0 && self.area.height() > 0
    }

    #[cfg(test)]
    fn width(&self) -> i32 {
        self.area.width()
    }
}

/// Place the caret at an exact text boundary. Overlays use buffer coordinates
/// and scroll with the text; they are moved again after wrapping or resizing.
/// Always damage the view itself when the caret's appearance changes: damaging
/// only the overlay can be swallowed while the widget is still being laid out.
fn update_caret(view: &sourceview5::View, caret: &Caret) {
    let geometry = caret.offset.get().and_then(|offset| {
        let buffer = view.buffer();
        let iter = buffer.iter_at_offset(offset.clamp(0, buffer.char_count()));
        let (strong, _) = view.cursor_locations(Some(&iter));
        (strong.height() > 0).then(|| (strong.x(), strong.y(), strong.height()))
    });

    let Some(geometry) = geometry else {
        let was_painting = caret.paint.replace(false);
        if was_painting {
            caret.area.queue_draw();
            view.queue_draw();
        }
        return;
    };

    let was_painting = caret.paint.replace(true);
    let moved = caret.geometry.replace(Some(geometry)) != Some(geometry);
    if !was_painting || moved {
        caret.area.set_content_height(geometry.2);
        view.move_overlay(&caret.area, geometry.0, geometry.1);
        view.queue_draw();
    }
    caret.area.queue_draw();
}

/// Character offsets in the displayed excerpt for a raw byte range. The
/// display-only marker after a final newline belongs to a non-empty selection
/// that reaches the end of the excerpt, but not to a zero-width boundary.
fn display_selection(text: &str, range: &Range<usize>, first: usize, trailing: bool) -> (i32, i32) {
    let start_byte = range.start.saturating_sub(first).min(text.len());
    let end_byte = range.end.saturating_sub(first).min(text.len());
    let start = display_offset(text, start_byte, false);
    let end = display_offset(text, end_byte, trailing && start_byte < end_byte);
    (start, end)
}

/// Word-level display ranges clipped to the selected review unit. Intersecting
/// rather than requiring containment keeps the selection tight when a re-diff
/// extends a word span into boundary whitespace.
fn selected_word_ranges(spans: &[Range<i32>], selection: Range<i32>) -> Vec<Range<i32>> {
    spans
        .iter()
        .filter_map(|span| {
            let start = span.start.max(selection.start);
            let end = span.end.min(selection.end);
            (start < end).then_some(start..end)
        })
        .collect()
}

/// Highlight an exact byte range after translating it to GTK character
/// offsets. Text changes use the same word-level spans as the unselected
/// highlight so selecting a phrase does not also select its boundary
/// whitespace. A zero-width side returns its exact boundary for an overlay
/// caret, leaving the neighbouring character untouched.
fn select_excerpt(
    buffer: &sourceview5::Buffer,
    text: &str,
    range: &Range<usize>,
    first: usize,
    trailing: bool,
    kind: ChangeKind,
    word_ranges: &[Range<i32>],
) -> (i32, Option<i32>) {
    let (start, end) = display_selection(text, range, first, trailing);
    if start != end {
        let selected = if kind == ChangeKind::Text {
            selected_word_ranges(word_ranges, start..end)
        } else {
            Vec::new()
        };
        if selected.is_empty() {
            buffer.apply_tag_by_name(
                TAG_INLINE_SELECTED,
                &buffer.iter_at_offset(start),
                &buffer.iter_at_offset(end),
            );
            (start, None)
        } else {
            for span in &selected {
                buffer.apply_tag_by_name(
                    TAG_INLINE_SELECTED,
                    &buffer.iter_at_offset(span.start),
                    &buffer.iter_at_offset(span.end),
                );
            }
            (selected[0].start, None)
        }
    } else {
        (start, Some(start))
    }
}

/// Character offset at the start of every logical line, using GTK's line
/// breaks (`\n`, `\r\n`, a lone `\r` and U+2029).
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut offset = 0;
    for line in crate::core::textlines::split_inclusive(text) {
        starts.push(offset);
        offset += line.chars().count();
    }
    starts
}

/// Rebase the original inline highlights onto the excerpt's character offsets,
/// returning the display ranges they cover.
fn tag_excerpt(
    buffer: &sourceview5::Buffer,
    text: &str,
    first_line: usize,
    spans: &[InlineSpan],
) -> Vec<Range<i32>> {
    // The diff engine emits spans in ascending line order. Use character
    // offsets so GTK paragraph breaks and wrapped rows cannot shift the tags.
    let first_span = spans.partition_point(|span| span.line < first_line);
    let mut spans = spans[first_span..].iter().peekable();
    let mut ranges = Vec::new();
    for (line, offset) in line_starts(text).into_iter().enumerate() {
        while let Some(span) = spans.next_if(|span| span.line == first_line + line) {
            let start = (offset + span.start) as i32;
            let end = (offset + span.end) as i32;
            buffer.apply_tag_by_name(
                TAG_INLINE_NORMAL,
                &buffer.iter_at_offset(start),
                &buffer.iter_at_offset(end),
            );
            ranges.push(start..end);
        }
        if spans.peek().is_none() {
            break;
        }
    }
    ranges
}

#[derive(Clone, Copy)]
enum ScrollTarget {
    Line(usize),
    Offset(i32),
    Position(f64),
}

fn place_scroll(
    panes: [(&sourceview5::View, &gtk::ScrolledWindow); 2],
    targets: [ScrollTarget; 2],
) -> Vec<f64> {
    let mut geometry = Vec::new();
    for ((view, scroller), target) in panes.into_iter().zip(targets) {
        let adjustment = scroller.vadjustment();
        let position = match target {
            ScrollTarget::Position(value) => value,
            ScrollTarget::Line(_) | ScrollTarget::Offset(_) => {
                let buffer = view.buffer();
                let iter = match target {
                    ScrollTarget::Line(line) => buffer
                        .iter_at_line(line as i32)
                        .unwrap_or_else(|| buffer.end_iter()),
                    ScrollTarget::Offset(offset) => buffer.iter_at_offset(offset),
                    _ => unreachable!(),
                };
                // Validate the retained layout up to the anchor synchronously;
                // scroll_to_iter alone defers this work until a later frame.
                let location = view.iter_location(&iter);
                let height = adjustment.page_size();
                let y = f64::from(location.y());
                let margin = height * 0.15;
                if y >= adjustment.value() + margin
                    && y + f64::from(location.height()) <= adjustment.value() + height - margin
                {
                    adjustment.value()
                } else {
                    y - height * 0.35
                }
            }
        };
        let max = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
        adjustment.set_value(position.clamp(adjustment.lower(), max));
        geometry.extend([
            adjustment.value(),
            adjustment.upper(),
            adjustment.page_size(),
        ]);
    }
    geometry
}

#[cfg(test)]
mod tests {
    use super::{
        LineNumberStyle, display_offset, display_selection, hunk_changes_baseline_eof,
        hunk_changes_live_eof, hunk_text, line_number_markup, line_starts, marked_text,
        replacement_span, selected_word_ranges, trailing_blank_change,
    };

    #[test]
    fn line_number_markup_uses_normal_and_strong_colours() {
        let style = LineNumberStyle {
            normal: "#111111".into(),
            strong: "#eeeeee".into(),
            strong_bold: true,
        };
        assert_eq!(
            line_number_markup(3, false, &style),
            r##"<span foreground="#111111">3</span>"##
        );
        assert_eq!(
            line_number_markup(4, true, &style),
            r##"<span foreground="#eeeeee" weight="bold">4</span>"##
        );

        let unbold = LineNumberStyle {
            strong_bold: false,
            ..style
        };
        assert_eq!(
            line_number_markup(5, true, &unbold),
            r##"<span foreground="#eeeeee">5</span>"##
        );
    }

    #[test]
    fn inspection_slice_keeps_exact_logical_lines() {
        let text = "context\r\nα\r\n\n🙂final";
        assert_eq!(hunk_text(text, 1, 2), "α\r\n\n");
        assert_eq!(hunk_text(text, 3, 1), "🙂final");
        assert_eq!(hunk_text(text, 4, 0), "");
        assert_eq!(hunk_text("one\rtwo\nthree", 0, 1), "one\r");
        assert_eq!(hunk_text("one\r\ntwo\nthree", 1, 1), "two\n");
    }

    #[test]
    fn excerpt_line_offsets_follow_gtk_line_breaks() {
        assert_eq!(line_starts(""), Vec::<usize>::new());
        assert_eq!(line_starts("one\ntwo"), vec![0, 4]);
        assert_eq!(line_starts("a\rb\u{2029}c\r\nd"), vec![0, 2, 4, 7]);
    }

    #[test]
    fn blank_lines_in_hunks_get_markers() {
        let ranges = std::slice::from_ref(&(1..3usize));
        let (text, markers) = marked_text("a\n\n\r\nb\n\n", ranges, false);
        assert_eq!(text, "a\n\u{b6}\n\u{b6}\r\nb\n\n");
        assert_eq!(markers, vec![2, 4]);
        let ranges = std::slice::from_ref(&(0..usize::MAX));
        let (text, markers) = marked_text("\n\nx", ranges, false);
        assert_eq!(text, "\u{b6}\n\u{b6}\nx");
        assert_eq!(markers, vec![0, 2]);
    }

    #[test]
    fn a_changed_final_newline_marks_the_trailing_display_line() {
        assert!(trailing_blank_change("\nsentence\n", "sentence"));
        assert!(trailing_blank_change("sentence\n", "sentence"));
        assert!(trailing_blank_change("\nsentence\n\n", "sentence"));
        assert!(!trailing_blank_change("sentence", "sentence\n"));
        assert!(!trailing_blank_change("\nsentence\n", "sentence\n"));
        assert!(!trailing_blank_change("", "\nsentence\n"));

        let ranges = std::slice::from_ref(&(0..2usize));
        let (text, markers) = marked_text("\nsentence\n", ranges, true);
        assert_eq!(text, "\u{b6}\nsentence\n\u{b6}");
        assert_eq!(markers, vec![0, 11]);
        let ranges = std::slice::from_ref(&(0..3usize));
        let (text, markers) = marked_text("\nsentence\n\n", ranges, true);
        assert_eq!(text, "\u{b6}\nsentence\n\u{b6}\n\u{b6}");
        assert_eq!(markers, vec![0, 11, 13]);
        let (text, markers) = marked_text("\nsentence\n", ranges, false);
        assert_eq!(text, "\u{b6}\nsentence\n");
        assert_eq!(markers, vec![0]);
    }

    #[test]
    fn only_the_side_that_changes_eof_claims_the_trailing_line() {
        use crate::core::diff::Hunk;

        let deletion = Hunk {
            old_start: 1,
            old_len: 1,
            new_start: 1,
            new_len: 0,
        };
        assert!(!hunk_changes_live_eof(&deletion, 1));
        assert!(hunk_changes_baseline_eof(&deletion, 2));

        let insertion = Hunk {
            old_start: 1,
            old_len: 0,
            new_start: 1,
            new_len: 1,
        };
        assert!(hunk_changes_live_eof(&insertion, 2));
        assert!(!hunk_changes_baseline_eof(&insertion, 1));

        let final_newline = Hunk {
            old_start: 1,
            old_len: 1,
            new_start: 1,
            new_len: 1,
        };
        assert!(hunk_changes_live_eof(&final_newline, 2));
        assert!(hunk_changes_baseline_eof(&final_newline, 2));
    }

    #[test]
    fn display_offsets_count_markers_before_the_byte() {
        let raw = "\n\nab";
        assert_eq!(display_offset(raw, 0, false), 0);
        assert_eq!(display_offset(raw, 1, false), 2);
        assert_eq!(display_offset(raw, 2, false), 4);
        assert_eq!(display_offset(raw, 4, false), 6);
    }

    #[test]
    fn selections_include_the_trailing_marker_at_the_excerpt_end() {
        let raw = "\nA sentence.\n\n";
        let whole = 0..raw.len();
        assert_eq!(display_selection(raw, &whole, 0, false), (0, 16));
        assert_eq!(display_selection(raw, &whole, 0, true), (0, 17));

        // A zero-width caret stays before the display-only marker.
        let boundary = raw.len()..raw.len();
        assert_eq!(display_selection(raw, &boundary, 0, true), (16, 16));
    }

    #[test]
    fn selected_text_spans_are_clipped_to_the_review_range() {
        // The review range includes the leading space needed to replace the
        // old text, but the word-level highlight starts at "relative". A
        // re-diff may also extend the word span into trailing whitespace.
        let spans = [0..2, 3..14];
        assert_eq!(selected_word_ranges(&spans, 2..13), vec![3..13]);
        assert!(selected_word_ranges(&spans, 14..16).is_empty());
    }

    #[test]
    fn replacement_preserves_unchanged_text() {
        assert_eq!(
            replacement_span("before\nold\nafter\n", "before\nnew\nafter\n"),
            Some((7..10, "new"))
        );
        assert_eq!(replacement_span("same", "same"), None);
    }

    #[test]
    fn replacement_uses_character_offsets_not_bytes() {
        assert_eq!(replacement_span("é🙂old終", "é🙂新終"), Some((2..5, "新")));
    }

    #[test]
    fn replacement_handles_insert_delete_and_overlapping_matches() {
        for (old, new, expected) in [
            ("", "abc", (0..0, "abc")),
            ("abc", "", (0..3, "")),
            ("ab", "a🙂b", (1..1, "🙂")),
            ("a🙂b", "ab", (1..2, "")),
            ("aaa", "aaaa", (3..3, "a")),
            ("aaaa", "aaa", (3..4, "")),
            ("a\r\nb", "a\nb", (1..2, "")),
        ] {
            let (range, text) = replacement_span(old, new).unwrap();
            assert_eq!((range.clone(), text), expected);
            let result: String = old
                .chars()
                .take(range.start as usize)
                .chain(text.chars())
                .chain(old.chars().skip(range.end as usize))
                .collect();
            assert_eq!(result, new);
        }
    }
}
