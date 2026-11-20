//! Whole-line selection from a view's left line-number gutter.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gtk::glib;
use sourceview5::View;
use sourceview5::prelude::*;

struct GutterSelection {
    view: glib::WeakRef<View>,
    anchor: Cell<Option<i32>>,
    dragging: Cell<bool>,
}

/// Make primary-button clicks and drags on `view`'s left gutter select whole
/// logical lines. The gesture claims the press so the text view does not start
/// an arbitrary character selection instead.
pub(crate) fn install_line_selection(view: &View) {
    let state = Rc::new(GutterSelection {
        view: glib::WeakRef::new(),
        anchor: Cell::new(None),
        dragging: Cell::new(false),
    });
    state.view.set(Some(view));

    let click = gtk::GestureClick::new();
    click.set_button(gtk::gdk::BUTTON_PRIMARY);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let state = state.clone();
        click.connect_pressed(move |gesture, _, x, y| {
            let Some(view) = state.view.upgrade() else {
                return;
            };
            if !over_gutter(&view, x) {
                return;
            }
            let line = line_at(&view, y);
            state.anchor.set(Some(line));
            state.dragging.set(true);
            select_lines(&view.buffer(), line, line);
            view.grab_focus();
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
    }
    {
        let state = state.clone();
        click.connect_released(move |_, _, _, _| {
            state.dragging.set(false);
            state.anchor.set(None);
        });
    }
    view.add_controller(click);

    let motion = gtk::EventControllerMotion::new();
    motion.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let state = state.clone();
        motion.connect_motion(move |_, _, y| {
            if !state.dragging.get() {
                return;
            }
            let Some(anchor) = state.anchor.get() else {
                return;
            };
            let Some(view) = state.view.upgrade() else {
                return;
            };
            let current = line_at(&view, y);
            select_lines(&view.buffer(), anchor, current);
        });
    }
    view.add_controller(motion);
}

/// True when `x` is inside the view's left line-number gutter.
fn over_gutter(view: &View, x: f64) -> bool {
    let gutter = sourceview5::prelude::ViewExt::gutter(view, gtk::TextWindowType::Left);
    gutter.compute_bounds(view).is_some_and(|bounds| {
        let start = f64::from(bounds.x());
        let end = start + f64::from(bounds.width());
        x >= start && x < end
    })
}

fn line_at(view: &View, y: f64) -> i32 {
    // Gesture and motion events report widget coordinates, while `line_at_y`
    // expects buffer coordinates. Without this conversion a scrolled document
    // resolves every click to a line measured from the top of the file.
    let (_, buffer_y) =
        view.window_to_buffer_coords(gtk::TextWindowType::Widget, 0, y.round() as i32);
    view.line_at_y(buffer_y.max(0)).0.line()
}

/// Select every line between `anchor` and `current`, inclusive, using
/// half-open line indexes. A selection through line `n` ends at the start of
/// line `n + 1`, so its terminator is included.
fn select_lines(buffer: &gtk::TextBuffer, anchor: i32, current: i32) {
    let range = line_range(anchor, current, buffer.line_count());
    let start = buffer
        .iter_at_line(range.start as i32)
        .unwrap_or_else(|| buffer.start_iter());
    let end = buffer
        .iter_at_line(range.end as i32)
        .unwrap_or_else(|| buffer.end_iter());
    buffer.select_range(&start, &end);
}

fn line_range(anchor: i32, current: i32, line_count: i32) -> Range<usize> {
    let last = line_count.max(1) - 1;
    let first = anchor.min(current).clamp(0, last) as usize;
    let end = anchor.max(current).clamp(0, last) as usize + 1;
    first..end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_covers_every_touched_line() {
        assert_eq!(line_range(2, 2, 5), 2..3);
        assert_eq!(line_range(1, 3, 5), 1..4);
        assert_eq!(line_range(3, 1, 5), 1..4);
    }

    #[test]
    fn selection_clamps_to_the_buffer() {
        assert_eq!(line_range(-2, 1, 5), 0..2);
        assert_eq!(line_range(4, 9, 5), 4..5);
        assert_eq!(line_range(0, 0, 1), 0..1);
    }
}
