//! The pixel-look check on a render: is every pixel one of its own texel's layer colours, and
//! is every texel drawn in one colour?

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fmt;

use glam::{IVec2, UVec2, Vec2};
use image::RgbaImage;

use crate::blend::{clip, texel_alpha};
use crate::gpu::View;
use crate::mask::Mask;
use crate::noise::AlphaMap;

#[derive(Default)]
pub struct Report {
    /// Pixels inside the world.
    pub pixels: usize,
    /// Pixels in a colour none of their texel's layers has: blended colours.
    pub off_texel: usize,
    /// Texels in view.
    pub texels: usize,
    /// Texels drawn in more than one colour: cut by a mask edge.
    pub split_texels: usize,
    /// Texels each layer won, judged by the colour of the texel's first pixel.
    pub winners: [usize; 3],
    /// The layer that won each texel, where its first pixel is a layer colour; the topmost
    /// layer when two layers share the colour.
    pub texel_winners: HashMap<IVec2, usize>,
}

impl fmt::Debug for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Report")
            .field("pixels", &self.pixels)
            .field("off_texel", &self.off_texel)
            .field("texels", &self.texels)
            .field("split_texels", &self.split_texels)
            .field("winners", &self.winners)
            .finish()
    }
}

impl Report {
    /// Both criteria hold.
    pub fn passes(&self) -> bool {
        self.off_texel == 0 && self.split_texels == 0
    }
}

pub fn check(image: &RgbaImage, view: &View, layers: &[RgbaImage; 3], world: u32) -> Report {
    let mut report = Report::default();
    let mut first = HashMap::new();
    let mut split = HashSet::new();
    for (x, y, pixel) in image.enumerate_pixels() {
        let at = view.world_at(UVec2::new(x, y));
        if at.cmplt(Vec2::ZERO).any() || at.cmpge(Vec2::splat(world as f32)).any() {
            continue;
        }
        report.pixels += 1;
        let texel = at.floor().as_ivec2();
        let rgb = [pixel.0[0], pixel.0[1], pixel.0[2]];
        let won = (0..3)
            .rev()
            .find(|&layer| texel_of(&layers[layer], texel)[..3] == rgb);
        if won.is_none() {
            report.off_texel += 1;
        }
        match first.entry(texel) {
            Entry::Vacant(entry) => {
                entry.insert(rgb);
                if let Some(layer) = won {
                    report.texel_winners.insert(texel, layer);
                    report.winners[layer] += 1;
                }
            }
            Entry::Occupied(entry) => {
                if *entry.get() != rgb {
                    split.insert(texel);
                }
            }
        }
    }
    report.texels = first.len();
    report.split_texels = split.len();
    report
}

/// The layer the hard clip should show at a texel, with the masks read at its centre.
pub fn model_winner(texel: IVec2, alphas: &[AlphaMap; 3], masks: &[Mask; 2]) -> usize {
    let centre = texel.as_vec2() + 0.5;
    let mut won = 0;
    for layer in 1..3 {
        let alpha = texel_alpha(alphas[layer].get(texel));
        if clip(masks[layer - 1].sample(centre), alpha, 0.0) >= 1.0 {
            won = layer;
        }
    }
    won
}

/// A texel of a texture that repeats over the world.
fn texel_of(image: &RgbaImage, texel: IVec2) -> [u8; 4] {
    let x = texel.x.rem_euclid(image.width() as i32) as u32;
    let y = texel.y.rem_euclid(image.height() as i32) as u32;
    image.get_pixel(x, y).0
}
