//! The pixel-look criteria on real GPU renders of the demo painting. With the hard clip and the
//! masks read at texel centres, every pixel must be one of its own texel's three layer colours
//! and every texel must be drawn whole. Each way of breaking that must fail the same check, or
//! the check proves nothing.

use blendtest::check::{self, Report};
use blendtest::gpu::{Headless, Settings, View};
use blendtest::noise::{self, KINDS, NoiseParams};
use blendtest::{scene, textures};
use glam::{UVec2, Vec2};

const SIZE: UVec2 = UVec2::new(480, 360);

/// Each layer's default noise, changed as `change` says.
fn noise_params(change: impl Fn(NoiseParams) -> NoiseParams) -> [NoiseParams; 3] {
    std::array::from_fn(|layer| change(noise::layer_defaults(layer)))
}

fn render_with(
    headless: &mut Headless,
    origin: Vec2,
    zoom: u32,
    settings: Settings,
    params: &[NoiseParams; 3],
) -> Report {
    let layers = textures::placeholders();
    let alphas = noise::alpha_maps(params, &layers);
    let masks = scene::demo(scene::SPACING);
    let view = View::new(origin, zoom, SIZE);
    let image = headless
        .render(&layers, &alphas, &masks, &view, &settings)
        .expect("render");
    check::check(&image, &view, &layers, scene::WORLD)
}

fn render(headless: &mut Headless, origin: Vec2, zoom: u32, settings: Settings) -> Report {
    render_with(headless, origin, zoom, settings, &noise_params(|p| p))
}

fn headless() -> Headless {
    Headless::new().expect("a GPU adapter for the render tests")
}

#[test]
fn the_shader_validates() {
    let module = naga::front::wgsl::parse_str(blendtest::gpu::SHADER).expect("parse");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("validate");
}

#[test]
fn the_hard_clip_read_at_texel_centres_keeps_every_texel_whole() {
    let mut headless = headless();
    for (origin, zoom) in scene::VIEWS {
        let report = render(&mut headless, origin, zoom, Settings::default());
        assert_eq!(report.off_texel, 0, "zoom {zoom}: {report:?}");
        assert_eq!(report.split_texels, 0, "zoom {zoom}: {report:?}");
        // Every layer must win a fair share of the view, or there is no edge to check.
        for (layer, &won) in report.winners.iter().enumerate() {
            assert!(
                won * 20 > report.texels,
                "zoom {zoom} layer {layer}: {report:?}"
            );
        }
    }
}

#[test]
fn every_noise_kind_and_size_keeps_every_texel_whole() {
    let mut headless = headless();
    let (origin, zoom) = scene::VIEWS[2];
    for kind in KINDS {
        for size in [None, Some(80)] {
            let params = noise_params(|p| NoiseParams { kind, size, ..p });
            let report = render_with(&mut headless, origin, zoom, Settings::default(), &params);
            assert!(report.passes(), "{kind:?} {size:?}: {report:?}");
            assert!(
                report.winners.iter().all(|&won| won > 0),
                "{kind:?} {size:?}"
            );
        }
    }
}

#[test]
fn reading_the_masks_per_pixel_cuts_texels() {
    let mut headless = headless();
    let (origin, zoom) = scene::VIEWS[2];
    let settings = Settings {
        snap: false,
        ..Settings::default()
    };
    let report = render(&mut headless, origin, zoom, settings);
    assert!(report.split_texels > 0, "{report:?}");
}

#[test]
fn a_soft_clip_makes_colours_no_layer_has() {
    let mut headless = headless();
    let (origin, zoom) = scene::VIEWS[1];
    let settings = Settings {
        spread: 0.3,
        ..Settings::default()
    };
    let report = render(&mut headless, origin, zoom, settings);
    assert!(report.off_texel > 0, "{report:?}");
}

#[test]
fn the_gpu_picks_the_layer_the_cpu_model_picks() {
    let mut headless = headless();
    let layers = textures::placeholders();
    let masks = scene::demo(scene::SPACING);
    let world = scene::WORLD;
    let view = View::new(Vec2::ZERO, 1, UVec2::splat(world));
    let quantized = masks.clone().map(|mask| mask.quantized());
    // 80 texels matches no albedo's size, so the alpha must repeat on its own.
    for size in [None, Some(80)] {
        let params = noise_params(|p| NoiseParams { size, ..p });
        let alphas = noise::alpha_maps(&params, &layers);
        let image = headless
            .render(&layers, &alphas, &masks, &view, &Settings::default())
            .expect("render");
        let report = check::check(&image, &view, &layers, world);
        assert_eq!(report.texels, (world * world) as usize);
        let differ = report
            .texel_winners
            .iter()
            .filter(|&(&texel, &won)| check::model_winner(texel, &alphas, &quantized) != won)
            .count();
        // Only ties between a mask and an alpha step may round the other way on the GPU.
        assert!(
            differ * 1000 < report.texels,
            "{size:?}: {differ} of {} texels",
            report.texels
        );
    }
}
