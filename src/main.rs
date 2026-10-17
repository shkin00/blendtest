//! blendtest: paint dirt and grass over soil and watch alpha-clipped blending at pixel scale.
//! Without a subcommand it opens the painting window.

mod app;
mod slider;

use std::path::PathBuf;

use anyhow::{Context, bail};
use blendtest::gpu::{Headless, Settings, View, ViewMode};
use blendtest::noise::{self, NoiseKind, NoiseParams};
use blendtest::{check, scene, textures};
use clap::{Parser, Subcommand, ValueEnum};
use glam::{UVec2, Vec2};

#[derive(Parser)]
#[command(about)]
struct Cli {
    /// The folder with the colour textures, `<layer>.raw.png`.
    #[arg(long, global = true, default_value_os_t = textures::assets_dir())]
    assets: PathBuf,
    /// Open the window with the demo painting instead of bare soil.
    #[arg(long)]
    demo: bool,
    /// Write the window's third frame to this PNG and exit.
    #[arg(long, value_name = "PNG")]
    window_shot: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Print how visible each texture's wrap seams are, and write a sheet with each texture
    /// repeated 2 x 2.
    Inspect {
        #[arg(required = true)]
        images: Vec<PathBuf>,
        /// Where to write the sheet.
        #[arg(long)]
        sheet: Option<PathBuf>,
        #[arg(long, default_value_t = 2)]
        scale: u32,
    },
    /// Write placeholder colour textures as `<layer>.raw.png`.
    Placeholders {
        /// Replace raw textures that are already there.
        #[arg(long)]
        force: bool,
    },
    /// Render the demo painting without a window, write it as a PNG and run the pixel-look
    /// check on it.
    Screenshot {
        out: PathBuf,
        #[arg(long, default_value_t = 3)]
        zoom: u32,
        /// The world pixel at the left edge; by default the view is centred on the world.
        #[arg(long)]
        x: Option<f32>,
        /// The world pixel at the top edge.
        #[arg(long)]
        y: Option<f32>,
        #[arg(long, default_value_t = 960)]
        width: u32,
        #[arg(long, default_value_t = 640)]
        height: u32,
        /// 0 is the hard clip; UFO uses 0.3.
        #[arg(long, default_value_t = 0.0)]
        spread: f32,
        /// Read the masks at every pixel instead of at texel centres.
        #[arg(long)]
        no_snap: bool,
        #[arg(long, value_enum, default_value_t = Mode::Final)]
        mode: Mode,
        /// Mask texels every this many world pixels.
        #[arg(long, default_value_t = scene::SPACING)]
        spacing: u32,
        /// Use the built-in placeholders instead of the textures in the assets folder.
        #[arg(long)]
        placeholders: bool,
        /// The alpha noise of every layer.
        #[arg(long, value_enum, default_value_t = Noise::Fbm)]
        noise: Noise,
        /// The alpha maps' side in texels; by default each layer's albedo size.
        #[arg(long)]
        noise_size: Option<u32>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Noise {
    Fbm,
    White,
    Cellular,
    Luminance,
}

#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    Final,
    Mask,
    Alpha,
    Winner,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None => app::run(
            &cli.assets,
            app::Options {
                demo: cli.demo,
                shot: cli.window_shot,
            },
        ),
        Some(Command::Inspect {
            images,
            sheet,
            scale,
        }) => {
            let mut loaded = Vec::new();
            for path in &images {
                let image = image::open(path)
                    .with_context(|| format!("load {}", path.display()))?
                    .to_rgba8();
                let (across, down) = textures::seam_ratios(&image);
                let colours: std::collections::HashSet<_> =
                    image.pixels().map(|p| [p.0[0], p.0[1], p.0[2]]).collect();
                let see_through = image.pixels().filter(|p| p.0[3] < 255).count();
                println!(
                    "{}: {}x{}, {} colours, {see_through} not opaque, seam across {across:.2}, down {down:.2}",
                    path.display(),
                    image.width(),
                    image.height(),
                    colours.len(),
                );
                loaded.push(image);
            }
            if let Some(sheet) = sheet {
                textures::contact_sheet(&loaded, scale).save(&sheet)?;
                println!("{}", sheet.display());
            }
            Ok(())
        }
        Some(Command::Placeholders { force }) => textures::write_placeholders(&cli.assets, force),
        Some(Command::Screenshot {
            out,
            zoom,
            x,
            y,
            width,
            height,
            spread,
            no_snap,
            mode,
            spacing,
            placeholders,
            noise,
            noise_size,
        }) => {
            let layers = if placeholders {
                textures::placeholders()
            } else {
                textures::load(&cli.assets)?
            };
            let kind = match noise {
                Noise::Fbm => NoiseKind::Fbm,
                Noise::White => NoiseKind::White,
                Noise::Cellular => NoiseKind::Cellular,
                Noise::Luminance => NoiseKind::Luminance,
            };
            let params = std::array::from_fn(|layer| NoiseParams {
                kind,
                size: noise_size,
                ..noise::layer_defaults(layer)
            });
            let alphas = noise::alpha_maps(&params, &layers);
            let masks = scene::demo(spacing);
            let size = UVec2::new(width, height);
            let centred = (Vec2::splat(scene::WORLD as f32) - size.as_vec2() / zoom as f32) / 2.0;
            let origin = Vec2::new(x.unwrap_or(centred.x), y.unwrap_or(centred.y));
            let view = View::new(origin, zoom, size);
            let settings = Settings {
                spread,
                snap: !no_snap,
                mode: match mode {
                    Mode::Final => ViewMode::Final,
                    Mode::Mask => ViewMode::Mask,
                    Mode::Alpha => ViewMode::Alpha,
                    Mode::Winner => ViewMode::Winner,
                },
                ..Settings::default()
            };
            let image = Headless::new()?.render(&layers, &alphas, &masks, &view, &settings)?;
            image.save(&out)?;
            println!("{}", out.display());
            if matches!(mode, Mode::Final) {
                let report = check::check(&image, &view, &layers, scene::WORLD);
                println!("{report:?}");
                if !report.passes() {
                    bail!("the pixel look does not hold");
                }
                println!("pixel look holds");
            }
            Ok(())
        }
    }
}
