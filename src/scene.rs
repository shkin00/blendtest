//! A fixed painting for screenshots and tests: a soft dirt road across the world and grass
//! patches beside and over it, so every mask value from 0 to 1 is somewhere on screen.

use glam::Vec2;

use crate::mask::{Brush, Mask};

/// The world is square, this many pixels a side: 64 Krust tiles.
pub const WORLD: u32 = 1024;

/// One mask texel per Krust tile corner.
pub const SPACING: u32 = 16;

/// Views of the painting for the checks: the world pixel at the top-left, and the zoom.
pub const VIEWS: [(Vec2, u32); 3] = [
    (Vec2::new(120.0, 280.0), 1),
    (Vec2::new(560.0, 440.0), 3),
    (Vec2::new(470.0, 500.0), 4),
];

pub fn demo(spacing: u32) -> [Mask; 2] {
    let mut dirt = Mask::new(WORLD, spacing);
    let mut grass = Mask::new(WORLD, spacing);
    let road = Brush {
        radius: 96.0,
        hardness: 0.2,
    };
    for step in 0..=80 {
        let x = 64.0 + step as f32 * 11.2;
        let y = 520.0 + 90.0 * (x / 150.0).sin();
        dirt.stamp(Vec2::new(x, y), road, 0.25);
    }
    // One patch that reaches full cover, one that never does, one across the road.
    let wide = Brush {
        radius: 220.0,
        hardness: 0.0,
    };
    grass.stamp(Vec2::new(300.0, 280.0), wide, 1.0);
    let faint = Brush {
        radius: 160.0,
        hardness: 0.3,
    };
    grass.stamp(Vec2::new(760.0, 260.0), faint, 0.55);
    let over = Brush {
        radius: 140.0,
        hardness: 0.1,
    };
    for _ in 0..3 {
        grass.stamp(Vec2::new(620.0, 560.0), over, 0.4);
    }
    [dirt, grass]
}
