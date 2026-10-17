//! Sliders the mouse wheel also moves while the cursor is over them. egui's own sliders ignore
//! the wheel, and in the scrolling side panel the wheel would scroll the panel instead.

use std::ops::RangeInclusive;

use egui::emath::Numeric;

/// What one wheel notch does to a slider's value.
#[derive(Clone, Copy, Debug)]
pub enum Notch {
    /// Add this; the slider is linear.
    Add(f64),
    /// Multiply by this; the slider is logarithmic.
    Times(f64),
}

/// A slider that one wheel notch moves by `notch` while the cursor is over it. While it is
/// hovered it also takes the scroll egui is still smoothing out, so the panel does not scroll.
pub fn slider<N: Numeric>(
    ui: &mut egui::Ui,
    enabled: bool,
    value: &mut N,
    range: RangeInclusive<N>,
    text: &str,
    notch: Notch,
) -> egui::Response {
    let (low, high) = (range.start().to_f64(), range.end().to_f64());
    let widget = egui::Slider::new(value, range)
        .logarithmic(matches!(notch, Notch::Times(_)))
        .text(text);
    let mut response = ui.add_enabled(enabled, widget);
    if enabled && response.hovered() {
        let turned = ui.input_mut(|input| {
            input.smooth_scroll_delta = egui::Vec2::ZERO;
            notches(&input.events)
        });
        if turned != 0.0 {
            let next = stepped(value.to_f64(), turned, notch, low, high, N::INTEGRAL);
            *value = N::from_f64(next);
            response.mark_changed();
        }
    }
    response
}

/// The value `notches` wheel notches on from `now`, kept in `low..=high` and whole if
/// `integral`.
pub fn stepped(now: f64, notches: f64, notch: Notch, low: f64, high: f64, integral: bool) -> f64 {
    let next = match notch {
        Notch::Add(step) => now + step * notches,
        Notch::Times(factor) => now * factor.powf(notches),
    };
    let next = if integral { next.round() } else { next };
    next.clamp(low, high)
}

/// The wheel notches in these events, up positive. A line is one notch; touchpads report
/// points, about 50 to a notch.
pub fn notches(events: &[egui::Event]) -> f64 {
    events
        .iter()
        .map(|event| match event {
            egui::Event::MouseWheel { unit, delta, .. } => f64::from(match unit {
                egui::MouseWheelUnit::Line => delta.y,
                egui::MouseWheelUnit::Point => delta.y / 50.0,
                egui::MouseWheelUnit::Page => delta.y * 3.0,
            }),
            _ => 0.0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notch_adds_or_multiplies_and_stays_in_range() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(close(
            stepped(0.5, 1.0, Notch::Add(0.05), 0.0, 1.0, false),
            0.55
        ));
        assert!(close(
            stepped(0.5, -2.0, Notch::Add(0.05), 0.0, 1.0, false),
            0.4
        ));
        assert_eq!(stepped(0.98, 1.0, Notch::Add(0.05), 0.0, 1.0, false), 1.0);
        assert_eq!(stepped(0.02, -1.0, Notch::Add(0.05), 0.0, 1.0, false), 0.0);
        assert!(close(
            stepped(48.0, 1.0, Notch::Times(1.1), 2.0, 256.0, false),
            52.8
        ));
        assert!(close(
            stepped(48.0, -1.0, Notch::Times(1.1), 2.0, 256.0, false),
            48.0 / 1.1
        ));
        assert_eq!(
            stepped(250.0, 3.0, Notch::Times(1.1), 2.0, 256.0, false),
            256.0
        );
        assert_eq!(stepped(3.0, 1.0, Notch::Add(1.0), 1.0, 6.0, true), 4.0);
        assert_eq!(stepped(3.0, 0.4, Notch::Add(1.0), 1.0, 6.0, true), 3.0);
    }

    #[test]
    fn lines_are_notches_and_points_count_fifty_to_one() {
        let wheel = |unit, y| egui::Event::MouseWheel {
            unit,
            delta: egui::vec2(0.0, y),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        };
        assert_eq!(notches(&[]), 0.0);
        assert_eq!(notches(&[wheel(egui::MouseWheelUnit::Line, 1.0)]), 1.0);
        assert_eq!(
            notches(&[
                wheel(egui::MouseWheelUnit::Line, -1.0),
                wheel(egui::MouseWheelUnit::Line, -1.0),
                egui::Event::PointerGone,
            ]),
            -2.0
        );
        assert_eq!(notches(&[wheel(egui::MouseWheelUnit::Point, 25.0)]), 0.5);
    }

    /// Where the scroll area sits: in a side panel in the root `Ui`, as in the app, or in an
    /// `Area` of its own.
    #[derive(Clone, Copy, Debug)]
    enum Holder {
        Panel,
        Area,
    }

    /// A slider near the top of a scroll area much taller than the 400 x 300 window. Returns
    /// the slider's rect, so a scroll shows as the rect moving.
    fn frame(
        ctx: &egui::Context,
        holder: Holder,
        value: &mut f32,
        events: Vec<egui::Event>,
    ) -> egui::Rect {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 300.0),
            )),
            events,
            ..Default::default()
        };
        let mut rect = egui::Rect::NOTHING;
        let mut content = |ui: &mut egui::Ui| {
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    ui.label("above");
                    let notch = Notch::Add(0.05);
                    rect = slider(ui, true, value, 0.0..=1.0, "value", notch).rect;
                    ui.add_space(2000.0);
                });
        };
        let mut output = ctx.run_ui(input, |ui| match holder {
            Holder::Panel => {
                egui::Panel::left("panel")
                    .default_size(300.0)
                    .resizable(false)
                    .show(ui, |ui| content(ui));
            }
            Holder::Area => {
                egui::Area::new(egui::Id::new("panel"))
                    .fixed_pos(egui::Pos2::ZERO)
                    .show(ui.ctx(), |ui| content(ui));
            }
        });
        output.textures_delta.clear();
        rect
    }

    /// Lay out, rest the pointer at `pos` for a few frames, turn the wheel one notch down
    /// there, and let egui's smoothing run out. Returns the slider's rect before and after.
    fn wheel_at(
        ctx: &egui::Context,
        holder: Holder,
        value: &mut f32,
        pos: Option<egui::Pos2>,
    ) -> (egui::Rect, egui::Rect) {
        let before = frame(ctx, holder, value, vec![]);
        let pos = pos.unwrap_or(before.center());
        for _ in 0..3 {
            frame(ctx, holder, value, vec![egui::Event::PointerMoved(pos)]);
        }
        let notch = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        };
        frame(ctx, holder, value, vec![notch]);
        let mut after = before;
        for _ in 0..60 {
            after = frame(ctx, holder, value, vec![]);
        }
        (before, after)
    }

    #[test]
    fn the_wheel_over_a_slider_moves_it_and_not_the_panel() {
        for holder in [Holder::Panel, Holder::Area] {
            let mut value = 0.5;
            let (before, after) = wheel_at(&egui::Context::default(), holder, &mut value, None);
            assert!((value - 0.45).abs() < 1e-6, "{holder:?}: {value}");
            assert_eq!(after, before, "{holder:?}");
        }
    }

    #[test]
    fn the_wheel_elsewhere_scrolls_the_panel_and_leaves_the_slider() {
        for holder in [Holder::Panel, Holder::Area] {
            let mut value = 0.5;
            let below = Some(egui::pos2(50.0, 250.0));
            let (before, after) = wheel_at(&egui::Context::default(), holder, &mut value, below);
            assert_eq!(value, 0.5, "{holder:?}");
            assert!(
                after.min.y < before.min.y,
                "{holder:?}: {after:?} {before:?}"
            );
        }
    }
}
