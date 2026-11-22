// Owner: T7 text tools (round 4).
//
// The text flows that are not panel code (FEATURE_TEXT 5.13): the sample job of the font picker and the atlas
// textures its samples are packed into (`AppState::font_samples`). `dispatch.rs` and `io_flows.rs` call into this
// file with one line each.

use std::collections::HashMap;
use std::time::Instant;

use eframe::egui;
use wte_core::{FontCatalog, Progress};

use crate::app::WteApp;
use crate::jobs::{FontSample, JobOutput};
use crate::state::{FontSampleCell, FontSampleState};

/// What a sample shows (FEATURE_TEXT 5.13), at `SAMPLE_PX` pixels per em.
pub(crate) const SAMPLE_TEXT: &str = "Abc 123";
pub(crate) const SAMPLE_PX: f32 = 16.0;
/// Atlas textures of 512 x 1024 px with cells of 128 x 20 px: 4 columns x 51 rows = 204 samples each.
pub(crate) const ATLAS_W: usize = 512;
pub(crate) const ATLAS_H: usize = 1024;
pub(crate) const CELL_W: usize = 128;
pub(crate) const CELL_H: usize = 20;
const COLS: usize = ATLAS_W / CELL_W;
const ROWS: usize = ATLAS_H / CELL_H;
/// Samples per atlas.
pub(crate) const CELLS_PER_ATLAS: usize = COLS * ROWS;

/// True when `LoadFontSamples` should start a job: none runs, and the samples were not made for a catalog of this
/// many families yet (the catalog grows once, when the system scan arrives).
pub(crate) fn needs_samples(samples: &FontSampleState, families: usize) -> bool {
    !samples.loading && samples.requested_for != Some(families)
}

/// `AppAction::LoadFontSamples`: starts the non-modal job that draws one sample per family of the catalog
/// (`FontCatalog::sample`), once per catalog (`FontSampleState::requested_for`).
pub fn load_font_samples(app: &mut WteApp) {
    let families = app.state.fonts.families().len();
    if !needs_samples(&app.state.font_samples, families) {
        return;
    }
    app.state.font_samples.requested_for = Some(families);
    app.state.font_samples.loading = true;
    // The job draws from its own copy of the catalog (the face list; font files are read in place, not copied).
    let catalog = app.state.fonts.clone();
    app.jobs.spawn(&app.ctx, "Drawing font samples", false, move |p| JobOutput::FontSamples(draw_samples(&catalog, p)));
}

/// One sample per family of `catalog`, in catalog order, in the face the font combos would pick for a new text
/// (`pick_style` with "Regular": that style, else the first face).
pub(crate) fn draw_samples(catalog: &FontCatalog, progress: &dyn Progress) -> Vec<FontSample> {
    let started = Instant::now();
    let families = catalog.families();
    let mut out = Vec::with_capacity(families.len());
    for (i, family) in families.iter().enumerate() {
        if progress.cancelled() {
            break;
        }
        let style = crate::panels::text_block::pick_style(catalog, family, "Regular");
        let image = catalog.find(family, &style).and_then(|face| catalog.sample(face.id, SAMPLE_TEXT, SAMPLE_PX));
        out.push(FontSample { family: family.clone(), image });
        if i % 16 == 0 {
            progress.report(i as f32 / families.len().max(1) as f32, "Drawing font samples");
        }
    }
    let drawn = out.iter().filter(|s| s.image.is_some()).count();
    log::info!("font samples: {drawn} of {} families drawn in {:.1} ms", out.len(), started.elapsed().as_secs_f64() * 1e3);
    out
}

/// The samples packed into atlas images, and each family's cell.
pub(crate) struct PackedSamples {
    pub atlases: Vec<egui::ColorImage>,
    pub cells: HashMap<String, FontSampleCell>,
}

/// Packs the samples into atlases of `ATLAS_W` x `ATLAS_H` px, one cell of `CELL_W` x `CELL_H` per drawn sample, row by
/// row in sample order. A sample is white with its coverage as alpha (premultiplied), so the picker tints it with
/// the text colour; a family without a sample gets no cell. A sample larger than a cell is cut to it.
pub(crate) fn pack_samples(samples: &[FontSample]) -> PackedSamples {
    let mut atlases: Vec<egui::ColorImage> = Vec::new();
    let mut cells = HashMap::new();
    let mut slot = 0usize;
    for sample in samples {
        let Some((sw, sh, bytes)) = &sample.image else { continue };
        let (stride, rows) = (*sw as usize, *sh as usize);
        let (w, h) = (stride.min(CELL_W), rows.min(CELL_H));
        if w == 0 || h == 0 || bytes.len() < stride * rows {
            continue;
        }
        let atlas = slot / CELLS_PER_ATLAS;
        let index = slot % CELLS_PER_ATLAS;
        if atlas == atlases.len() {
            atlases.push(egui::ColorImage::filled([ATLAS_W, ATLAS_H], egui::Color32::TRANSPARENT));
        }
        let (x0, y0) = ((index % COLS) * CELL_W, (index / COLS) * CELL_H);
        let image = &mut atlases[atlas];
        for y in 0..h {
            for x in 0..w {
                image.pixels[(y0 + y) * ATLAS_W + x0 + x] = egui::Color32::from_white_alpha(bytes[y * stride + x]);
            }
        }
        let rect = egui::Rect::from_min_size(egui::pos2(x0 as f32, y0 as f32), egui::vec2(w as f32, h as f32));
        cells.insert(sample.family.clone(), FontSampleCell { atlas, rect });
        slot += 1;
    }
    PackedSamples { atlases, cells }
}

/// `JobOutput::FontSamples`: packs the samples into atlas textures of 512 x 1024 px and records each family's cell.
pub fn on_font_samples(app: &mut WteApp, samples: Vec<FontSample>) {
    let packed = pack_samples(&samples);
    let atlases: Vec<egui::TextureHandle> = packed
        .atlases
        .into_iter()
        .enumerate()
        .map(|(i, image)| app.ctx.load_texture(format!("font_samples_{i}"), image, egui::TextureOptions::LINEAR))
        .collect();
    log::info!("font samples: {} cells in {} atlas(es) for {} families", packed.cells.len(), atlases.len(), samples.len());
    let state = &mut app.state.font_samples;
    state.cells = packed.cells;
    state.atlases = atlases;
    state.loading = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoProgress;
    impl Progress for NoProgress {
        fn report(&self, _fraction: f32, _stage: &str) {}
        fn cancelled(&self) -> bool {
            false
        }
    }

    fn sample(family: &str, w: u32, h: u32, ink: u8) -> FontSample {
        FontSample { family: family.to_owned(), image: Some((w, h, vec![ink; (w * h) as usize])) }
    }

    #[test]
    fn the_bundled_families_get_a_sample_each() {
        let catalog = FontCatalog::bundled();
        let samples = draw_samples(&catalog, &NoProgress);
        assert_eq!(samples.len(), catalog.families().len());
        for s in &samples {
            let (w, h, bytes) = s.image.as_ref().unwrap_or_else(|| panic!("{} has no sample", s.family));
            assert!(*w as usize <= CELL_W && *h as usize <= CELL_H, "{}: {w}x{h}", s.family);
            assert_eq!(bytes.len(), (*w * *h) as usize);
            assert!(bytes.iter().any(|&b| b > 128), "{} has no ink", s.family);
        }
        // A copy of the catalog draws the same samples.
        assert_eq!(draw_samples(&catalog.clone(), &NoProgress), samples);
    }

    #[test]
    fn samples_are_packed_into_atlases_of_204_cells() {
        assert_eq!(CELLS_PER_ATLAS, 204);
        let mut samples: Vec<FontSample> = (0..300).map(|i| sample(&format!("F{i:03}"), 100 + (i % 20), 15, (i % 250) as u8 + 1)).collect();
        samples.insert(5, FontSample { family: "NoInk".to_owned(), image: None });
        samples.push(sample("Wide", 200, 30, 9));
        let packed = pack_samples(&samples);
        assert_eq!(packed.atlases.len(), 2);
        assert!(packed.atlases.iter().all(|a| a.size == [ATLAS_W, ATLAS_H]));
        assert_eq!(packed.cells.len(), 301);
        assert!(!packed.cells.contains_key("NoInk"));
        // Cell 0 and the last cell of the first atlas, then the first of the second.
        assert_eq!(packed.cells["F000"], FontSampleCell { atlas: 0, rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 15.0)) });
        let last = &packed.cells["F203"];
        assert_eq!((last.atlas, last.rect.min), (0, egui::pos2(384.0, 1000.0)));
        let next = &packed.cells["F204"];
        assert_eq!((next.atlas, next.rect.min), (1, egui::Pos2::ZERO));
        // A sample larger than a cell is cut to it.
        assert_eq!(packed.cells["Wide"].rect.size(), egui::vec2(128.0, 20.0));
        // The pixels: white with the coverage as alpha, inside the cell only.
        let c = &packed.cells["F007"];
        let img = &packed.atlases[c.atlas];
        let at = |x: f32, y: f32| img.pixels[y as usize * ATLAS_W + x as usize];
        assert_eq!(at(c.rect.min.x, c.rect.min.y), egui::Color32::from_white_alpha(8));
        assert_eq!(at(c.rect.max.x - 1.0, c.rect.max.y - 1.0), egui::Color32::from_white_alpha(8));
        assert_eq!(at(c.rect.min.x, c.rect.max.y), egui::Color32::TRANSPARENT);
    }

    #[test]
    fn samples_are_requested_once_per_catalog() {
        let mut s = FontSampleState::default();
        assert!(needs_samples(&s, 2));
        s.requested_for = Some(2);
        assert!(!needs_samples(&s, 2));
        // The system scan arrived: the catalog grew.
        assert!(needs_samples(&s, 78));
        s.loading = true;
        assert!(!needs_samples(&s, 78), "one job at a time");
    }

    /// FEATURE_TEXT 8.4: the sample job for every family of this machine at most 0.5 s with a warm file cache.
    /// Reads the fonts of this machine; run with `--ignored`.
    #[test]
    #[ignore = "reads the installed fonts of this machine; timing"]
    fn sample_job_for_the_machine_fonts() {
        if !std::path::Path::new(r"C:\Windows\Fonts").is_dir() {
            eprintln!("SKIP: no C:\\Windows\\Fonts");
            return;
        }
        let mut catalog = FontCatalog::bundled();
        catalog.load_system();
        let _warm = draw_samples(&catalog, &NoProgress);
        let started = Instant::now();
        let samples = draw_samples(&catalog, &NoProgress);
        let secs = started.elapsed().as_secs_f64();
        let packed = pack_samples(&samples);
        eprintln!(
            "sample job: {} families, {} drawn, {:.1} ms warm, {} atlases",
            samples.len(),
            packed.cells.len(),
            secs * 1e3,
            packed.atlases.len()
        );
        assert!(samples.len() >= 50);
        assert!(secs <= 0.5, "{secs} s");
    }
}
