//! A list of [`Suggestions`] under an entry row, narrowed as the user types.
//! The row stays free text: a suggestion only fills it in when chosen.

use std::rc::Rc;

use adw::prelude::*;
use armarium_core::suggest::Suggestions;
use gtk::{gdk, glib};

/// Tallest the list grows before it scrolls.
const MAX_HEIGHT: i32 = 240;

/// Offers `suggestions` under `row`. Down and Up move through them, Enter or
/// a click fills the row, with Enter taking the first when none is selected,
/// and Escape closes only the list.
pub fn attach(row: &adw::EntryRow, suggestions: Suggestions) {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .activate_on_single_click(true)
        .can_focus(false)
        .css_classes(["navigation-sidebar"])
        .build();
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .max_content_height(MAX_HEIGHT)
        .propagate_natural_height(true)
        .child(&list)
        .build();
    // Without autohide the popover leaves keyboard focus in the row.
    let popover = gtk::Popover::builder()
        .autohide(false)
        .has_arrow(false)
        .position(gtk::PositionType::Bottom)
        .child(&scrolled)
        .build();
    popover.set_parent(row);
    row.connect_destroy(glib::clone!(
        #[weak]
        popover,
        move |_| popover.unparent()
    ));

    let suggestions = Rc::new(suggestions);
    row.connect_changed(glib::clone!(
        #[weak]
        popover,
        #[weak]
        list,
        move |row| {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            // Text set by code, such as the initial value, offers nothing.
            let focused = row.state_flags().contains(gtk::StateFlags::FOCUS_WITHIN);
            let matches = suggestions.matching(&row.text());
            if !focused || matches.is_empty() {
                popover.popdown();
                return;
            }
            for value in matches {
                let label = gtk::Label::builder()
                    .label(value)
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build();
                let item = gtk::ListBoxRow::builder()
                    .child(&label)
                    .can_focus(false)
                    .build();
                item.set_tooltip_text(Some(value));
                list.append(&item);
            }
            popover.set_width_request(row.width());
            popover.popup();
        }
    ));

    list.connect_row_activated(glib::clone!(
        #[weak]
        row,
        move |_, item| fill(&row, item)
    ));

    let keys = gtk::EventControllerKey::new();
    // Capture, so Escape reaches the list before the dialog closes on it.
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(glib::clone!(
        #[weak]
        row,
        #[weak]
        popover,
        #[weak]
        list,
        #[weak]
        scrolled,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, _| {
            if !popover.is_visible() {
                return glib::Propagation::Proceed;
            }
            let selected = list.selected_row().map(|r| r.index());
            let target = match key {
                gdk::Key::Down => Some(selected.map_or(0, |i| i + 1)),
                gdk::Key::Up => Some(selected.map_or(0, |i| i - 1).max(0)),
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    let Some(item) = list.selected_row().or_else(|| list.row_at_index(0)) else {
                        return glib::Propagation::Proceed;
                    };
                    fill(&row, &item);
                    return glib::Propagation::Stop;
                }
                gdk::Key::Escape => {
                    popover.popdown();
                    return glib::Propagation::Stop;
                }
                _ => return glib::Propagation::Proceed,
            };
            if let Some(item) = target.and_then(|i| list.row_at_index(i)) {
                list.select_row(Some(&item));
                scroll_to(&scrolled, &list, &item);
            }
            glib::Propagation::Stop
        }
    ));
    row.add_controller(keys);

    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave(glib::clone!(
        #[weak]
        popover,
        move |_| popover.popdown()
    ));
    row.add_controller(focus);
}

/// Puts the chosen suggestion in `row`, which then matches it exactly and so
/// closes the list.
fn fill(row: &adw::EntryRow, item: &gtk::ListBoxRow) {
    let Some(label) = item.child().and_downcast::<gtk::Label>() else {
        return;
    };
    row.set_text(&label.label());
    row.set_position(-1);
}

/// Scrolls just enough to show `item`.
fn scroll_to(scrolled: &gtk::ScrolledWindow, list: &gtk::ListBox, item: &gtk::ListBoxRow) {
    let Some(bounds) = item.compute_bounds(list) else {
        return;
    };
    let adj = scrolled.vadjustment();
    let (top, bottom) = (bounds.y() as f64, (bounds.y() + bounds.height()) as f64);
    if top < adj.value() {
        adj.set_value(top);
    } else if bottom > adj.value() + adj.page_size() {
        adj.set_value(bottom - adj.page_size());
    }
}
