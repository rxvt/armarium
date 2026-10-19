//! The content beside the details sidebar, laid out at the width it will have
//! once the sidebar finishes sliding in or out.
//!
//! `AdwOverlaySplitView` narrows its content a little on every frame of the
//! slide, and the cover grid redistributes its columns each time, so the
//! covers shake. This gives its child the final width straight away instead:
//! one reflow, then the sidebar slides over (or uncovers) settled content.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SteadyContent;

    #[glib::object_subclass]
    impl ObjectSubclass for SteadyContent {
        const NAME: &'static str = "ArmariumSteadyContent";
        type Type = super::SteadyContent;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for SteadyContent {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_overflow(gtk::Overflow::Hidden);
            // AdwBin lays out through a layout manager, which would bypass
            // size_allocate below.
            obj.set_layout_manager(None::<gtk::LayoutManager>);
        }
    }

    impl WidgetImpl for SteadyContent {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            self.obj()
                .child()
                .map_or((0, 0, -1, -1), |child| child.measure(orientation, for_size))
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            let obj = self.obj();
            let Some(child) = obj.child() else {
                return;
            };
            let settled = obj
                .ancestor(adw::OverlaySplitView::static_type())
                .and_downcast::<adw::OverlaySplitView>()
                .filter(|split| !split.is_collapsed())
                .map_or(width, |split| match split.sidebar() {
                    // The split view allocates the sidebar at its full width
                    // before the content, so this frame's width is known.
                    Some(sidebar) if split.shows_sidebar() => {
                        width.min(split.width() - sidebar.width())
                    }
                    _ => width.max(split.width()),
                });
            child.allocate(settled, height, baseline, None);
        }
    }

    impl BinImpl for SteadyContent {}
}

glib::wrapper! {
    pub struct SteadyContent(ObjectSubclass<imp::SteadyContent>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}
