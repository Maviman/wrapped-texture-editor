// Owner: round 5, package A (R5-1). The Full bake of one stamp as a job with owned inputs (ARCHITECTURE.md 5.20
// "Deferred Final bakes", 4.6): `Document::next_final_bake` makes it on the UI thread, `run` / `run_isolated` bakes it
// on any thread, `Document::apply_final_bake` applies the output. The synchronous Full bake of the document
// (`bake_and_apply(.., Full)`) runs the same job on the calling thread, so both paths give the same pixels.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crate::bvh::Bvh;
use crate::doc_stamps::raster_counter;
use crate::error::EditError;
use crate::ids::{StampId, TextureSetId};
use crate::mesh::MeshData;
use crate::pixel::SparseBuf;
use crate::pixel_ops::PremulMips;
use crate::stamp::{BakeQuality, Placement, Projector, StampObject, StampSource, SurfaceMode};
use crate::stamp_bake::{bake_projected, bake_uv};
use crate::texel_map::TexelMap;
use crate::text::{FaceFonts, RasterQuality, TextFonts, TextFrame, TextSpec, default_font_bytes, render_text};
use crate::wrap_bake::bake_wrapped;
use crate::wrap_param::{WrapParam, wrap_param};

/// The text's own face as owned parts: parsed font, file bytes, face index.
pub(crate) type OwnFace = (Arc<fontdue::Font>, Arc<[u8]>, u32);

/// `TextFonts` borrowed from owned parts: the text's own face (when present) and the bundled fallback.
pub(crate) fn text_fonts_of<'a>(own: &'a Option<OwnFace>, fallback: &'a fontdue::Font) -> TextFonts<'a> {
    TextFonts {
        font: own.as_ref().map(|(metrics, bytes, face_index)| FaceFonts { metrics, bytes, face_index: *face_index }),
        fallback: FaceFonts { metrics: fallback, bytes: default_font_bytes(), face_index: 0 },
    }
}

/// The source picture of a job.
pub(crate) enum JobPicture {
    /// Decoded mips of an image blob.
    Image(Arc<PremulMips>),
    /// The cached full picture of a text (`text_mips`).
    TextCached(Arc<PremulMips>),
    /// A text whose full picture is drawn by the job (`render_text(.., RasterQuality::Full)`).
    TextDraw { spec: Box<TextSpec>, own: Option<OwnFace>, fallback: Arc<fontdue::Font> },
}

/// Owned inputs of one Full bake of one stamp. `Send + 'static`: everything is owned or shared by `Arc`.
pub struct StampBakeJob {
    pub(crate) id: StampId,
    pub(crate) object: StampObject,
    pub(crate) mesh: Arc<MeshData>,
    pub(crate) bvh: Arc<Bvh>,
    /// The texel maps of all sets; index == set id.
    pub(crate) maps: Vec<Arc<TexelMap>>,
    pub(crate) picture: JobPicture,
    /// Round 6: the stamp's cached parameterization (wrapped stamps); `run` uses it when it serves the placement.
    pub(crate) wrap: Option<Arc<WrapParam>>,
}

/// The parameterization a wrapped bake used and whether it was built for it (`true`) or reused.
pub(crate) struct UsedWrap {
    pub(crate) param: Arc<WrapParam>,
    pub(crate) fresh: bool,
}

/// The full picture a job drew for a text: what `store_text_picture` keeps.
pub(crate) struct DrawnText {
    pub(crate) spec: TextSpec,
    pub(crate) frame: TextFrame,
    pub(crate) full: bool,
    pub(crate) mips: Arc<PremulMips>,
}

/// Result of a job. `Send + 'static`. Carries the job's identity (stamp id and object, mesh and texel map `Arc`s), the
/// baked buffers or the error, and for a drawn text its picture.
pub struct StampBakeOutput {
    pub(crate) id: StampId,
    pub(crate) object: StampObject,
    pub(crate) mesh: Arc<MeshData>,
    pub(crate) maps: Vec<Arc<TexelMap>>,
    pub(crate) result: Result<Vec<(TextureSetId, SparseBuf)>, EditError>,
    pub(crate) picture: Option<DrawnText>,
    pub(crate) elapsed: Duration,
    /// Round 6: the parameterization of a wrapped bake (`apply_final_bake` caches it).
    pub(crate) wrap: Option<UsedWrap>,
}

/// What `Document::apply_final_bake` did with an output.
#[derive(Debug)]
pub enum FinalBake {
    /// The caches of stamp `id` were replaced; `empty`: no linked layer received a texel.
    Applied { id: StampId, empty: bool },
    /// The bake failed (warning in the log); the stamp keeps its earlier caches and is no longer waiting.
    Failed { id: StampId },
    /// The output belongs to an earlier state (the stamp changed, was removed or baked otherwise): dropped.
    Stale,
    /// A stroke is open: nothing changed; apply the output again after the stroke.
    Busy(Box<StampBakeOutput>),
}

// Compile-time thread-safety checks (section 4.6).
const _: fn() = || {
    fn send_static<T: Send + 'static>() {}
    send_static::<StampBakeJob>();
    send_static::<StampBakeOutput>();
};

/// The core's bake pool (`run_isolated`): `max(1, available_parallelism - 1)` threads named `stamp-bake-N`.
fn bake_pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: OnceLock<Option<rayon::ThreadPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        let threads = cores.saturating_sub(1).max(1);
        match rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|i| format!("stamp-bake-{i}")).build() {
            Ok(pool) => Some(pool),
            Err(e) => {
                log::warn!("the stamp bake pool cannot be started ({e}); bakes use the global pool");
                None
            }
        }
    })
    .as_ref()
}

impl StampBakeJob {
    /// The stamp this job bakes.
    pub fn stamp(&self) -> StampId {
        self.id
    }

    /// True for a text stamp.
    pub fn is_text(&self) -> bool {
        matches!(self.object.source, StampSource::Text(_))
    }

    /// Bakes on the calling thread's rayon pool (the synchronous path). Never creates a pool.
    pub fn run(self) -> StampBakeOutput {
        let started = Instant::now();
        let StampBakeJob { id, object, mesh, bvh, maps, picture, wrap } = self;
        let mut drawn = None;
        let mut used = None;
        let result = (|| -> Result<Vec<(TextureSetId, SparseBuf)>, EditError> {
            let mips = match picture {
                JobPicture::Image(mips) | JobPicture::TextCached(mips) => mips,
                JobPicture::TextDraw { spec, own, fallback } => {
                    // Counted on the thread that draws (FEATURE_TEXT 8.1).
                    raster_counter::bump();
                    let raster = render_text(&text_fonts_of(&own, &fallback), &spec, RasterQuality::Full)?;
                    let mips = Arc::new(PremulMips::build(&raster.image));
                    drawn = Some(DrawnText { spec: *spec, frame: raster.frame, full: raster.full, mips: Arc::clone(&mips) });
                    mips
                }
            };
            Ok(match object.placement {
                Placement::Projected(p) if p.mode == SurfaceMode::Wrap => {
                    let (baked, w) = wrap_bake(id, &mesh, &bvh, &maps, &mips, object.tint, &p, wrap, BakeQuality::Full);
                    used = w;
                    baked
                }
                Placement::Projected(p) => bake_projected(&mesh, &bvh, &maps, &mips, object.tint, &p, BakeQuality::Full),
                Placement::Uv(p) => {
                    let map = maps.get(p.set.index()).ok_or(EditError::UnknownSet)?;
                    let buf = bake_uv(&mips, object.tint, &p, map);
                    if buf.is_empty() { Vec::new() } else { vec![(p.set, buf)] }
                }
            })
        })();
        StampBakeOutput { id, object, mesh, maps, result, picture: drawn, elapsed: started.elapsed(), wrap: used }
    }

    /// `run` inside the core's bake pool (`available_parallelism - 1` threads, at least 1): the bake's data-parallel
    /// loops do not queue the UI thread's own loops behind them. What the app's worker calls.
    pub fn run_isolated(self) -> StampBakeOutput {
        match bake_pool() {
            Some(pool) => pool.install(|| self.run()),
            None => self.run(),
        }
    }
}

/// The wrap bake of stamp `id` (round 6, 4.6): the cached parameterization `cached` when it serves `p`, else a fresh
/// one; then `bake_wrapped`. Logs `wrap: stamp N param ...` (info for Full bakes, debug for Preview). Returns the
/// bake and the parameterization it used (`None` when the projector or the surface gives none: nothing is baked).
#[allow(clippy::too_many_arguments)]
pub(crate) fn wrap_bake(
    id: StampId,
    mesh: &Arc<MeshData>,
    bvh: &Bvh,
    maps: &[Arc<TexelMap>],
    mips: &PremulMips,
    tint: [u8; 4],
    p: &Projector,
    cached: Option<Arc<WrapParam>>,
    quality: BakeQuality,
) -> (Vec<(TextureSetId, SparseBuf)>, Option<UsedWrap>) {
    let started = Instant::now();
    let reused = cached.filter(|w| w.serves_inputs(mesh, maps, mips.size(), p));
    let fresh = reused.is_none();
    let param = match reused {
        Some(w) => Some(w),
        None => wrap_param(mesh, bvh, maps, mips.size(), p).map(Arc::new),
    };
    let Some(param) = param else {
        log::debug!("wrap: stamp {} has no surface under it", id.0);
        return (Vec::new(), None);
    };
    let param_ms = started.elapsed().as_secs_f64() * 1e3;
    let baked = bake_wrapped(mesh, maps, &param, mips, tint, p, quality);
    let s = param.stats_for(p);
    let line = format!(
        "wrap: stamp {} param {} verts {} tris in {:.1} ms ({}), covered {:.1} %, cut {}, turns {}, jumps {}; bake {:.1} ms ({:?})",
        id.0,
        s.verts,
        s.tris,
        if fresh { s.micros as f64 / 1e3 } else { param_ms },
        if fresh { "fresh" } else { "reused" },
        s.covered * 100.0,
        s.cut_tris,
        s.turns,
        s.jumps,
        started.elapsed().as_secs_f64() * 1e3 - param_ms,
        quality,
    );
    match quality {
        BakeQuality::Full => log::info!("{line}"),
        BakeQuality::Preview => log::debug!("{line}"),
    }
    (baked, Some(UsedWrap { param, fresh }))
}

impl std::fmt::Debug for StampBakeOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StampBakeOutput")
            .field("stamp", &self.id)
            .field("sets", &self.result.as_ref().map(|b| b.iter().map(|(s, _)| *s).collect::<Vec<_>>()))
            .field("drew_text", &self.picture.is_some())
            .field("elapsed", &self.elapsed)
            .finish()
    }
}

impl StampBakeOutput {
    /// The stamp this output belongs to.
    pub fn stamp(&self) -> StampId {
        self.id
    }

    /// True for a text stamp.
    pub fn is_text(&self) -> bool {
        matches!(self.object.source, StampSource::Text(_))
    }

    /// Time `run` took (picture and bake).
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// True when the bake failed.
    pub fn is_error(&self) -> bool {
        self.result.is_err()
    }
}
