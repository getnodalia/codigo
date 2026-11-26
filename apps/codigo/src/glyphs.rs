// SPDX-License-Identifier: MIT OR Apache-2.0
//! The few icons the debugger needs that the Nodalia set does not have, drawn with strokes in the
//! ink the caller picks. They are original work, like every icon drawn in code.

use egui::{vec2, Color32, Painter, Rect, Shape, Stroke};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Glyph {
    Bug,
    StepOver,
    StepOut,
}

pub fn paint(painter: &Painter, rect: Rect, glyph: Glyph, ink: Color32) {
    let u = rect.width().min(rect.height()) / 24.0;
    let at = |x: f32, y: f32| rect.center() + vec2((x - 12.0) * u, (y - 12.0) * u);
    let stroke = Stroke::new(1.8 * u, ink);
    match glyph {
        Glyph::Bug => {
            painter.circle_filled(at(12.0, 14.0), 5.0 * u, ink);
            painter.circle_filled(at(12.0, 7.0), 2.6 * u, ink);
            for (x0, y0, x1, y1) in [
                (7.0, 11.0, 3.5, 9.0),
                (7.0, 14.5, 3.0, 14.5),
                (7.5, 18.0, 4.0, 20.5),
                (17.0, 11.0, 20.5, 9.0),
                (17.0, 14.5, 21.0, 14.5),
                (16.5, 18.0, 20.0, 20.5),
                (10.5, 5.0, 8.5, 2.5),
                (13.5, 5.0, 15.5, 2.5),
            ] {
                painter.line_segment([at(x0, y0), at(x1, y1)], stroke);
            }
        }
        Glyph::StepOver => {
            painter.circle_filled(at(12.0, 19.0), 2.3 * u, ink);
            let arc: Vec<_> = (0..=16)
                .map(|i| {
                    let t = std::f32::consts::PI * (1.0 - i as f32 / 16.0);
                    at(12.0 + 7.5 * t.cos(), 12.0 - 7.0 * t.sin())
                })
                .collect();
            painter.add(Shape::line(arc, stroke));
            painter.line_segment([at(19.5, 12.0), at(17.0, 8.8)], stroke);
            painter.line_segment([at(19.5, 12.0), at(22.0, 8.8)], stroke);
        }
        Glyph::StepOut => {
            painter.circle_filled(at(12.0, 19.0), 2.3 * u, ink);
            painter.line_segment([at(12.0, 15.0), at(12.0, 5.0)], stroke);
            painter.line_segment([at(12.0, 4.5), at(7.5, 9.0)], stroke);
            painter.line_segment([at(12.0, 4.5), at(16.5, 9.0)], stroke);
        }
    }
}
