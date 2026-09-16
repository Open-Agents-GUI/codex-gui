use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, ContentMask, Element, ElementId, GlobalElementId,
    Hsla, InspectorElementId, IntoElement, LayoutId, ParentElement, Pixels, SharedString, Styled,
    Window, div, point, size,
};
use gpui_component::StyledExt as _;

const SHIMMER_DURATION: Duration = Duration::from_millis(1_350);
const SHIMMER_BANDS: [f32; 5] = [0.18, 0.42, 0.9, 0.42, 0.18];

/// Text with a light band sweeping from left to right, used for in-progress
/// titles. The element repaints every frame until it is unmounted.
pub(in crate::gui::chat_history) struct ShimmerText {
    id: ElementId,
    base: AnyElement,
    highlights: Vec<AnyElement>,
}

#[derive(Clone, Copy, Default)]
struct ShimmerTextState {
    started_at: Option<Instant>,
}

impl ShimmerText {
    pub(in crate::gui::chat_history) fn new(
        id: impl Into<ElementId>,
        text: SharedString,
        base_color: Hsla,
        highlight_color: Hsla,
    ) -> Self {
        let text_element = |color| {
            div()
                .text_sm()
                .font_semibold()
                .text_color(color)
                .child(text.clone())
                .into_any_element()
        };
        Self {
            id: id.into(),
            base: text_element(base_color),
            highlights: SHIMMER_BANDS
                .into_iter()
                .map(|opacity| text_element(highlight_color.opacity(opacity)))
                .collect(),
        }
    }
}

impl IntoElement for ShimmerText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ShimmerText {
    type RequestLayoutState = ();
    type PrepaintState = f32;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.base.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.base.prepaint(window, cx);
        let available = size(
            AvailableSpace::Definite(bounds.size.width),
            AvailableSpace::MinContent,
        );
        for highlight in &mut self.highlights {
            highlight.layout_as_root(available, window, cx);
            highlight.prepaint_at(bounds.origin, window, cx);
        }

        let now = Instant::now();
        let started_at = window.with_element_state(
            global_id.expect("ShimmerText must have an id"),
            |state: Option<ShimmerTextState>, _| {
                let mut state = state.unwrap_or_default();
                let started_at = *state.started_at.get_or_insert(now);
                (started_at, state)
            },
        );
        window.request_animation_frame();
        (now.saturating_duration_since(started_at).as_secs_f32() / SHIMMER_DURATION.as_secs_f32())
            .fract()
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        progress: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.base.paint(window, cx);

        let band_width = bounds.size.width * 0.085;
        let sweep_width = bounds.size.width + band_width * 8.;
        let center = bounds.origin.x - band_width * 4. + sweep_width * *progress;
        for (index, highlight) in self.highlights.iter_mut().enumerate() {
            let offset = (index as f32 - 2.) * band_width;
            let mask = ContentMask {
                bounds: Bounds {
                    origin: point(center + offset, bounds.origin.y),
                    size: size(band_width, bounds.size.height),
                },
            };
            window.with_content_mask(Some(mask), |window| highlight.paint(window, cx));
        }
    }
}
