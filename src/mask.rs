//! Layer masks: one value per grid corner, bilinear between them, painted with a round brush.

use glam::Vec2;

use crate::blend::smoothstep;

/// A layer's mask over a square world. Texel (i, j) holds the value at world point
/// (i * spacing, j * spacing): the texels sit on the grid corners, as Krust's corner map does,
/// so a world `size` px across needs `size / spacing + 1` of them each way.
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    spacing: u32,
    side: u32,
    values: Vec<f32>,
}

/// A round brush in world pixels.
#[derive(Clone, Copy, Debug)]
pub struct Brush {
    pub radius: f32,
    /// The fraction of the radius at full strength; beyond it the strength falls smoothly to
    /// zero at the radius.
    pub hardness: f32,
}

impl Mask {
    pub fn new(world: u32, spacing: u32) -> Self {
        assert!(
            spacing > 0 && world.is_multiple_of(spacing),
            "a {world} px world does not divide into {spacing} px cells"
        );
        let side = world / spacing + 1;
        Self {
            spacing,
            side,
            values: vec![0.0; (side * side) as usize],
        }
    }

    pub fn spacing(&self) -> u32 {
        self.spacing
    }

    /// Texels per side.
    pub fn side(&self) -> u32 {
        self.side
    }

    pub fn get(&self, i: u32, j: u32) -> f32 {
        self.values[(j * self.side + i) as usize]
    }

    pub fn fill(&mut self, value: f32) {
        self.values.fill(value.clamp(0.0, 1.0));
    }

    /// Where a world point falls in the mask texture, as `terrain.wgsl` computes it: corner i
    /// is the centre of texel i.
    pub fn uv(&self, world: Vec2) -> Vec2 {
        (world / self.spacing as f32 + 0.5) / self.side as f32
    }

    /// The value at a world point: bilinear between the corners, clamped at the edges, as the
    /// GPU's linear clamp-to-edge sampler reads the texture at `uv`.
    pub fn sample(&self, world: Vec2) -> f32 {
        let last = (self.side - 1) as f32;
        let p = (world / self.spacing as f32).clamp(Vec2::ZERO, Vec2::splat(last));
        let corner = p.floor();
        let f = p - corner;
        let (i, j) = (corner.x as u32, corner.y as u32);
        let (i1, j1) = ((i + 1).min(self.side - 1), (j + 1).min(self.side - 1));
        let top = lerp(self.get(i, j), self.get(i1, j), f.x);
        let bottom = lerp(self.get(i, j1), self.get(i1, j1), f.x);
        lerp(top, bottom, f.y)
    }

    /// The texels as uploaded: one byte each, rows top to bottom.
    pub fn to_r8(&self) -> Vec<u8> {
        self.values
            .iter()
            .map(|v| (v * 255.0).round() as u8)
            .collect()
    }

    /// The same mask with every value rounded to what its byte holds.
    pub fn quantized(&self) -> Mask {
        Mask {
            values: self
                .to_r8()
                .into_iter()
                .map(|v| f32::from(v) / 255.0)
                .collect(),
            ..self.clone()
        }
    }

    /// The same painting at another spacing, read off this one at the new corners.
    pub fn resampled(&self, world: u32, spacing: u32) -> Mask {
        let mut mask = Mask::new(world, spacing);
        for j in 0..mask.side {
            for i in 0..mask.side {
                let at = Vec2::new(i as f32, j as f32) * spacing as f32;
                mask.values[(j * mask.side + i) as usize] = self.sample(at);
            }
        }
        mask
    }

    /// Add `amount` times the brush's falloff to every texel under the brush, clamped to 0..1;
    /// a negative amount erases. Returns whether any texel changed.
    pub fn stamp(&mut self, centre: Vec2, brush: Brush, amount: f32) -> bool {
        let spacing = self.spacing as f32;
        let last = (self.side - 1) as f32;
        let low = ((centre - brush.radius) / spacing).ceil().max(Vec2::ZERO);
        let high = ((centre + brush.radius) / spacing)
            .floor()
            .min(Vec2::splat(last));
        if high.x < low.x || high.y < low.y {
            return false;
        }
        let mut changed = false;
        for j in low.y as u32..=high.y as u32 {
            for i in low.x as u32..=high.x as u32 {
                let at = Vec2::new(i as f32, j as f32) * spacing;
                let weight = brush.falloff(at.distance(centre));
                if weight <= 0.0 {
                    continue;
                }
                let value = &mut self.values[(j * self.side + i) as usize];
                let painted = (*value + amount * weight).clamp(0.0, 1.0);
                changed |= painted != *value;
                *value = painted;
            }
        }
        changed
    }
}

impl Brush {
    /// The strength at this distance from the centre: 1 inside `hardness * radius`, 0 from the
    /// radius out, a smoothstep between.
    pub fn falloff(&self, distance: f32) -> f32 {
        let inner = self.radius * self.hardness;
        if distance >= self.radius {
            0.0
        } else if distance <= inner {
            1.0
        } else {
            1.0 - smoothstep(inner, self.radius, distance)
        }
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp() -> Mask {
        // 64 px world at 16 px: 5 x 5 corners, value i / 4 along x.
        let mut mask = Mask::new(64, 16);
        for j in 0..5 {
            for i in 0..5 {
                mask.values[(j * 5 + i) as usize] = i as f32 / 4.0;
            }
        }
        mask
    }

    #[test]
    fn a_world_has_one_more_corner_than_cells_each_way() {
        let mask = Mask::new(1024, 16);
        assert_eq!(mask.side(), 65);
        assert_eq!(mask.to_r8().len(), 65 * 65);
        assert!(mask.to_r8().iter().all(|&v| v == 0));
    }

    #[test]
    fn corner_i_is_the_centre_of_texel_i() {
        let mask = Mask::new(64, 16);
        assert_eq!(mask.uv(Vec2::ZERO), Vec2::splat(0.5 / 5.0));
        assert_eq!(
            mask.uv(Vec2::new(32.0, 64.0)),
            Vec2::new(2.5 / 5.0, 4.5 / 5.0)
        );
    }

    #[test]
    fn sampling_is_exact_on_corners_linear_between_and_clamped_outside() {
        let mask = ramp();
        assert_eq!(mask.sample(Vec2::new(16.0, 7.0)), 0.25);
        assert_eq!(mask.sample(Vec2::new(24.0, 40.0)), 0.375);
        assert_eq!(mask.sample(Vec2::new(4.0, 0.0)), 0.0625);
        assert_eq!(mask.sample(Vec2::new(-10.0, 0.0)), 0.0);
        assert_eq!(mask.sample(Vec2::new(70.0, 70.0)), 1.0);
    }

    #[test]
    fn falloff_is_full_inside_the_hard_part_and_zero_from_the_radius() {
        let brush = Brush {
            radius: 10.0,
            hardness: 0.5,
        };
        assert_eq!(brush.falloff(0.0), 1.0);
        assert_eq!(brush.falloff(5.0), 1.0);
        assert_eq!(brush.falloff(10.0), 0.0);
        assert_eq!(brush.falloff(20.0), 0.0);
        let mid = brush.falloff(7.5);
        assert!(mid > 0.0 && mid < 1.0, "{mid}");
        assert!(brush.falloff(6.0) > brush.falloff(9.0));
    }

    #[test]
    fn a_stamp_paints_under_the_brush_only_and_clamps() {
        let mut mask = Mask::new(64, 16);
        let brush = Brush {
            radius: 20.0,
            hardness: 0.0,
        };
        assert!(mask.stamp(Vec2::new(32.0, 32.0), brush, 0.4));
        assert_eq!(mask.get(2, 2), 0.4);
        let near = mask.get(3, 2);
        assert!(near > 0.0 && near < 0.4, "{near}");
        assert_eq!(mask.get(0, 0), 0.0);
        assert_eq!(mask.get(4, 2), 0.0);
        for _ in 0..5 {
            mask.stamp(Vec2::new(32.0, 32.0), brush, 0.4);
        }
        assert_eq!(mask.get(2, 2), 1.0);
        mask.stamp(Vec2::new(32.0, 32.0), brush, -5.0);
        assert!(mask.values.iter().all(|&v| v == 0.0));
        assert!(!mask.stamp(Vec2::new(32.0, 32.0), brush, -1.0));
    }

    #[test]
    fn a_stamp_off_the_world_changes_nothing() {
        let mut mask = Mask::new(64, 16);
        let brush = Brush {
            radius: 10.0,
            hardness: 0.5,
        };
        assert!(!mask.stamp(Vec2::new(-40.0, -40.0), brush, 1.0));
        assert!(!mask.stamp(Vec2::new(200.0, 30.0), brush, 1.0));
    }

    #[test]
    fn bytes_round_to_the_nearest_step() {
        let mut mask = Mask::new(16, 16);
        mask.values = vec![0.0, 0.5, 1.0, 0.1];
        assert_eq!(mask.to_r8(), vec![0, 128, 255, 26]);
        assert_eq!(mask.quantized().values[3], 26.0 / 255.0);
    }

    #[test]
    fn resampling_keeps_the_values_on_shared_corners() {
        let fine = ramp().resampled(64, 8);
        assert_eq!(fine.side(), 9);
        for i in 0..5 {
            assert_eq!(fine.get(i * 2, 3), i as f32 / 4.0);
        }
        assert_eq!(fine.get(1, 0), 0.125);
        assert_eq!(fine.resampled(64, 16), ramp());
    }
}
