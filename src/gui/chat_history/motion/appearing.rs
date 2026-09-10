use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, ContentMask, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Style, Window, div, prelude::*, relative,
    size,
};
use gpui_component::animation::ease_out_cubic;

const APPEAR_DURATION: Duration = Duration::from_millis(240);

#[derive(Clone, Copy, Default)]
struct AppearingState {
    natural_height: Option<Pixels>,
}

/// Which layout strategy `Appearing` used for the current frame.
///
/// `request_layout` picks one and passes it to `prepaint` so both phases agree:
/// a fully revealed child is laid out as a normal child (its size is known this
/// frame), while an animating child is a leaf measured in `prepaint`.
pub enum AppearingMode {
    /// Content is fully revealed; the child participates in this frame's layout.
    Full,
    /// Content is animating in; the height grows toward the measured natural size.
    Animating,
}

/// Reveals newly inserted transcript content from zero to its natural height.
///
/// While the content is animating in, the child remains laid out at full size
/// while a shrinking layout box and content mask expose only the animated
/// portion. This keeps text and controls from reflowing during the transition.
///
/// Once the content is fully revealed the child is laid out normally, so its
/// size is known in the same frame it first appears. This avoids a transient
/// zero-height frame when an already-revealed block scrolls back into view.
pub struct Appearing {
    id: ElementId,
    child: AnyElement,
    appeared_at: Option<Instant>,
    animate: bool,
}

impl Appearing {
    pub fn new(
        id: String,
        child: impl IntoElement,
        appeared_at: Option<Instant>,
        animate: bool,
    ) -> Self {
        let progress = appear_progress(appeared_at, animate, Instant::now());
        let child = div()
            .w_full()
            .opacity(progress)
            .child(child)
            .into_any_element();

        Self {
            id: id.into(),
            child,
            appeared_at,
            animate,
        }
    }
}

impl IntoElement for Appearing {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Appearing {
    type RequestLayoutState = AppearingMode;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let progress = appear_progress(self.appeared_at, self.animate, Instant::now());

        let mut style = Style::default();
        style.size.width = relative(1.).into();

        // Fully revealed: there is nothing left to animate, so let the child take
        // part in this frame's layout. Its natural height is then known
        // immediately instead of arriving one frame late.
        if progress >= 1. {
            let child_layout = self.child.request_layout(window, cx);
            return (
                window.request_layout(style, Some(child_layout), cx),
                AppearingMode::Full,
            );
        }

        // Animating: the natural height is measured in `prepaint` and only
        // available on the following frame, so this node stays a leaf whose
        // height grows from zero toward the previously measured height.
        let natural_height = window.with_element_state(
            global_id.expect("Appearing must have an id"),
            |state: Option<AppearingState>, _| {
                let state = state.unwrap_or_default();
                (state.natural_height, state)
            },
        );
        match natural_height {
            None => style.size.height = gpui::px(0.).into(),
            Some(height) => style.size.height = (height * progress).into(),
        }

        (
            window.request_layout(style, None, cx),
            AppearingMode::Animating,
        )
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        mode: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let full = matches!(mode, AppearingMode::Full);

        if !full {
            let available = size(
                AvailableSpace::Definite(bounds.size.width),
                AvailableSpace::MinContent,
            );
            let measured = self.child.layout_as_root(available, window, cx);
            let now = Instant::now();
            let changed = window.with_element_state(
                global_id.expect("Appearing must have an id"),
                |state: Option<AppearingState>, _| {
                    let mut state = state.unwrap_or_default();
                    let changed = state.natural_height != Some(measured.height);
                    state.natural_height = Some(measured.height);
                    (changed, state)
                },
            );

            if changed || appear_progress(self.appeared_at, self.animate, now) < 1. {
                window.request_animation_frame();
            }
        }

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if full {
                // The child is a layout child of this node, so its bounds are
                // already absolute. Prepainting at `bounds.origin` would apply
                // the parent origin a second time.
                self.child.prepaint(window, cx);
            } else {
                self.child.prepaint_at(bounds.origin, window, cx);
            }
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            self.child.paint(window, cx);
        });
    }
}

fn appear_progress(started_at: Option<Instant>, animate: bool, now: Instant) -> f32 {
    if !animate {
        return 1.;
    }
    let Some(started_at) = started_at else {
        return 1.;
    };
    let linear = (now.saturating_duration_since(started_at).as_secs_f32()
        / APPEAR_DURATION.as_secs_f32())
    .clamp(0., 1.);
    ease_out_cubic(linear)
}
