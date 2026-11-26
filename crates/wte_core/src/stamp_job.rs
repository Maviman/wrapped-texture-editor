// Owner: round 5, package A (R5-1). The Full bake of one stamp as a job with owned inputs (ARCHITECTURE.md 5.20
// "Deferred Final bakes", 4.6): `Document::next_final_bake` makes it on the UI thread, `run` / `run_isolated` bakes it
// on any thread, `Document::apply_final_bake` applies the output. The synchronous Full bake of the document
// (`bake_and_apply(.., Full)`) runs the same job on the calling thread, so both paths give the same pixels.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crate::bvh::Bvh;
use crate::doc_stamps::raster_counter;
use crate::doc_stickers::{BaseIdentity, RenderEntry, StyledEntry};
use crate::error::EditError;
use crate::ids::{StampId, TextureSetId};
use crate::mesh::MeshData;
use crate::pixel::SparseBuf;
use crate::pixel_ops::PremulMips;
use crate::stamp::{BakeQuality, Placement, Projector, StampObject, StampSource, SurfaceMode};
use crate::stamp_bake::{bake_projected, bake_uv};
use crate::sticker::StickerDoc;
use crate::sticker::pipeline::{SourceFrame, Styling, bake_placement_of, styled_mips};
use crate::sticker::render::{StickerRenderInput, render};
use crate::texel_map::TexelMap;
use crate::text::{FaceFonts, RasterQuality, TextFonts, TextFrame, TextSpec, default_font_bytes, render_text};
use crate::wrap_bake::bake_wrapped;
use crate::wrap_param::{WrapMeshCell, WrapParam, phase_times, wrap_param_in};

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
    /// Stickers (2.6): a picture made already (a cached styled or sticker picture, a wrap Preview job's picture): the
    /// mips, their frame and the tint the bake applies.
    Ready { mips: Arc<PremulMips>, frame: SourceFrame, tint: [u8; 4] },
    /// Stickers (2.6, 2.7): a styled or sticker picture the job makes on its thread: its base, then the styles when the
    /// stamp has any (`styling`).
    Derived { base: JobBase, styling: Option<Box<Styling>> },
}

/// The base picture of a `JobPicture::Derived` (2.6 step 1).
pub(crate) enum JobBase {
    /// Mips the document had (an image, a cached full text picture, a cached sticker render).
    Mips { mips: Arc<PremulMips>, identity: BaseIdentity },
    /// A text the job draws in full.
    TextDraw { spec: Box<TextSpec>, own: Option<OwnFace>, fallback: Arc<fontdue::Font> },
    /// A sticker the job renders (`sticker::render`) at `scale`, Full quality.
    Sticker { input: Box<StickerRenderInput>, scale: f32 },
}

/// What a job drew besides a text (2.7, review A27): the sticker render and the styled picture, for the document's
/// caches (`apply_bake_output`), so that the next Preview or thumbnail does not draw them again.
pub(crate) struct DrawnPicture {
    pub(crate) render: Option<RenderEntry>,
    pub(crate) styled: Option<StyledEntry>,
}

/// Round 8 (3.4): what a job does.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StampJobKind {
    /// The Full bake of a stamp (round 5).
    Full,
    /// The Preview bake of a wrapped stamp off the UI thread: its parameterization (the cached one when it serves)
    /// and `bake_wrapped(.., Preview)` (`Document::next_wrap_preview_job`, `apply_wrap_preview`).
    Preview,
    /// The parameterization of a wrapped stamp only, no pixels (`Document::wrap_param_job`, `apply_wrap_param`).
    ParamOnly,
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
    /// Round 8: what the job does, and the document's pending-edit serial it was made in (Preview jobs).
    pub(crate) kind: StampJobKind,
    pub(crate) serial: u64,
    /// Round 9 (package B, T7): the document's welded mesh graph (built by the first job that needs a fresh
    /// parameterization, or by the warm-up job; a Project bake never builds it).
    pub(crate) graph: Arc<WrapMeshCell>,
    /// Stickers (2.7, review A1): the sticker value the job draws (`None` for image and text stamps). The output is
    /// fresh only while the document's sticker is this `Arc` (`Document::job_inputs_current`).
    pub(crate) sticker: Option<Arc<StickerDoc>>,
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
    /// Objects-tabs: the BVH (and so the hidden objects) the job baked against. A parameterization in `wrap` is cached
    /// only while the document still has this BVH (`Document::set_hidden_objects` replaces it).
    pub(crate) bvh: Arc<Bvh>,
    pub(crate) maps: Vec<Arc<TexelMap>>,
    pub(crate) result: Result<Vec<(TextureSetId, SparseBuf)>, EditError>,
    pub(crate) picture: Option<DrawnText>,
    pub(crate) elapsed: Duration,
    /// Round 6: the parameterization of a wrapped bake (`apply_final_bake` caches it).
    pub(crate) wrap: Option<UsedWrap>,
    /// Round 8: the job's kind and pending-edit serial.
    pub(crate) kind: StampJobKind,
    pub(crate) serial: u64,
    /// Stickers: the job's sticker value (`StampBakeJob::sticker`) and what it drew besides a text.
    pub(crate) sticker: Option<Arc<StickerDoc>>,
    pub(crate) drawn: Option<DrawnPicture>,
}

/// Draws the full picture of a text on the job's thread (counted, FEATURE_TEXT 8.1).
fn draw_text(spec: TextSpec, own: &Option<OwnFace>, fallback: &fontdue::Font) -> Result<(Arc<PremulMips>, DrawnText), EditError> {
    raster_counter::bump();
    let raster = render_text(&text_fonts_of(own, fallback), &spec, RasterQuality::Full)?;
    let mips = Arc::new(PremulMips::build(&raster.image));
    let drawn = DrawnText { spec, frame: raster.frame, full: raster.full, mips: Arc::clone(&mips) };
    Ok((mips, drawn))
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

/// Round 8 (3.4): what `Document::apply_wrap_preview` did with the output of a wrapped stamp's Preview job.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WrapPreview {
    /// The pixels were applied (the last good picture). `due_again`: the stamp changed since the job was made (it is
    /// waiting for its next Preview job: `next_wrap_preview_job`).
    Applied { due_again: bool },
    /// The output belongs to an earlier state (Esc, undo, redo, a Final, another document, a removed stamp): dropped.
    Stale,
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

    /// Round 8: what the job does.
    pub fn kind(&self) -> StampJobKind {
        self.kind
    }

    /// Bakes on the calling thread's rayon pool (the synchronous path). Never creates a pool.
    pub fn run(self) -> StampBakeOutput {
        let started = Instant::now();
        let StampBakeJob { id, object, mesh, bvh, maps, picture, wrap, kind, serial, graph, sticker } = self;
        let quality = if kind == StampJobKind::Preview { BakeQuality::Preview } else { BakeQuality::Full };
        let mut drawn = None;
        let mut used = None;
        let mut extra: Option<DrawnPicture> = None;
        let result = (|| -> Result<Vec<(TextureSetId, SparseBuf)>, EditError> {
            let plain = |m: &Arc<PremulMips>| SourceFrame::plain(m.size());
            let (mips, frame, tint) = match picture {
                JobPicture::Image(mips) | JobPicture::TextCached(mips) => {
                    let frame = plain(&mips);
                    (mips, frame, object.tint)
                }
                JobPicture::TextDraw { spec, own, fallback } => {
                    let (mips, text) = draw_text(*spec, &own, &fallback)?;
                    drawn = Some(text);
                    let frame = plain(&mips);
                    (mips, frame, object.tint)
                }
                JobPicture::Ready { mips, frame, tint } => (mips, frame, tint),
                JobPicture::Derived { base, styling } => {
                    let mut made = DrawnPicture { render: None, styled: None };
                    let (base_mips, identity) = match base {
                        JobBase::Mips { mips, identity } => (mips, identity),
                        JobBase::TextDraw { spec, own, fallback } => {
                            let (mips, text) = draw_text(*spec, &own, &fallback)?;
                            drawn = Some(text);
                            let identity = BaseIdentity::Text(Arc::clone(&mips));
                            (mips, identity)
                        }
                        JobBase::Sticker { input, scale } => {
                            let picture = render(&input, scale, RasterQuality::Full);
                            let mips = Arc::new(PremulMips::build(&picture));
                            made.render = Some(RenderEntry {
                                doc: Arc::clone(&input.doc),
                                scale_bits: scale.to_bits(),
                                quality: BakeQuality::Full,
                                picture: Arc::new(picture),
                                mips: Arc::clone(&mips),
                            });
                            (mips, BaseIdentity::Sticker(Arc::clone(&input.doc), scale.to_bits()))
                        }
                    };
                    let out = match styling {
                        Some(st) => {
                            let (styled, frame, scale) = styled_mips(&base_mips, &st);
                            let mips = Arc::new(styled);
                            made.styled = Some(StyledEntry {
                                base: identity,
                                styles: st.styles,
                                tint: st.tint,
                                scale_bits: scale.to_bits(),
                                quality: st.quality,
                                frame,
                                mips: Arc::clone(&mips),
                            });
                            (mips, frame, [255; 4])
                        }
                        None => {
                            let frame = plain(&base_mips);
                            (base_mips, frame, object.tint)
                        }
                    };
                    extra = Some(made);
                    out
                }
            };
            // 2.6 "Two placements": the bake places the whole picture, margin included, on the bake placement.
            Ok(match bake_placement_of(&object.placement, &frame) {
                Placement::Projected(p) if p.mode == SurfaceMode::Wrap && kind == StampJobKind::ParamOnly => {
                    used = wrap_param_only(id, &mesh, &bvh, &graph, &maps, mips.size(), &p, wrap);
                    Vec::new()
                }
                Placement::Projected(p) if p.mode == SurfaceMode::Wrap => {
                    let (baked, w) = wrap_bake(id, &mesh, &bvh, &graph, &maps, &mips, tint, &p, wrap, quality);
                    used = w;
                    baked
                }
                _ if kind == StampJobKind::ParamOnly => Vec::new(),
                Placement::Projected(p) => bake_projected(&mesh, &bvh, &maps, &mips, tint, &p, quality),
                Placement::Uv(p) => {
                    let map = maps.get(p.set.index()).ok_or(EditError::UnknownSet)?;
                    let buf = bake_uv(&mips, tint, &p, map);
                    if buf.is_empty() { Vec::new() } else { vec![(p.set, buf)] }
                }
            })
        })();
        StampBakeOutput {
            id,
            object,
            mesh,
            bvh,
            maps,
            result,
            picture: drawn,
            elapsed: started.elapsed(),
            wrap: used,
            kind,
            serial,
            sticker,
            drawn: extra,
        }
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
/// one (round 9: from the mesh graph `graph`, built here on first use); then `bake_wrapped`. Logs `wrap: stamp N param ...` (info for Full bakes, debug for Preview). Returns the
/// bake and the parameterization it used (`None` when the projector or the surface gives none: nothing is baked).
#[allow(clippy::too_many_arguments)]
pub(crate) fn wrap_bake(
    id: StampId,
    mesh: &Arc<MeshData>,
    bvh: &Bvh,
    graph: &WrapMeshCell,
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
        None => fresh_param(id, mesh, bvh, graph, maps, mips.size(), p),
    };
    let Some(param) = param else {
        log::debug!("wrap: stamp {} has no surface under it", id.0);
        return (Vec::new(), None);
    };
    let param_ms = started.elapsed().as_secs_f64() * 1e3;
    let baked = bake_wrapped(mesh, maps, &param, mips, tint, p, quality);
    let s = param.stats_for_src(p, mips.size());
    let line = format!(
        "wrap: stamp {} param {} verts {} tris in {:.1} ms ({}), covered {:.1} %, cut {}, turns {}, jumps {}, relaxed {}, seams {}, stretch {:.1} %, rejected {:.2} %, cg_iters {}, passed {}, ends {:.1} %, lost_cut {:.2} %; bake {:.1} ms ({:?})",
        id.0,
        s.verts,
        s.tris,
        if fresh { s.micros as f64 / 1e3 } else { param_ms },
        if fresh { "fresh" } else { "reused" },
        s.covered * 100.0,
        s.cut_tris,
        s.turns,
        s.jumps,
        s.relaxed,
        s.seams,
        s.stretch * 100.0,
        s.rejected * 100.0,
        s.cg_iters,
        s.passed,
        s.ends_share * 100.0,
        s.lost_cut * 100.0,
        started.elapsed().as_secs_f64() * 1e3 - param_ms,
        quality,
    );
    match quality {
        BakeQuality::Full => log::info!("{line}"),
        BakeQuality::Preview => log::debug!("{line}"),
    }
    (baked, Some(UsedWrap { param, fresh }))
}

/// Round 8 (3.4): the parameterization of stamp `id` only (a `ParamOnly` job): the cached one when it serves `p`, else
/// a fresh one (logged as `wrap: stamp N param ... (fresh, param only)`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn wrap_param_only(
    id: StampId,
    mesh: &Arc<MeshData>,
    bvh: &Bvh,
    graph: &WrapMeshCell,
    maps: &[Arc<TexelMap>],
    src_size: [u32; 2],
    p: &Projector,
    cached: Option<Arc<WrapParam>>,
) -> Option<UsedWrap> {
    if let Some(w) = cached.filter(|w| w.serves_inputs(mesh, maps, src_size, p)) {
        return Some(UsedWrap { param: w, fresh: false });
    }
    let param = fresh_param(id, mesh, bvh, graph, maps, src_size, p)?;
    let s = param.stats_for_src(p, src_size);
    log::info!(
        "wrap: stamp {} param {} verts {} tris in {:.1} ms (fresh, param only), covered {:.1} %, cut {}, relaxed {}, seams {}, stretch {:.1} %, rejected {:.2} %, cg_iters {}, passed {}, ends {:.1} %, lost_cut {:.2} %",
        id.0,
        s.verts,
        s.tris,
        s.micros as f64 / 1e3,
        s.covered * 100.0,
        s.cut_tris,
        s.relaxed,
        s.seams,
        s.stretch * 100.0,
        s.rejected * 100.0,
        s.cg_iters,
        s.passed,
        s.ends_share * 100.0,
        s.lost_cut * 100.0
    );
    Some(UsedWrap { param, fresh: true })
}

/// Round 9 (package B): a fresh parameterization from the shared mesh graph (built here on first use), with a
/// `wrap phases:` debug line (graph wait, collect, spine + front, relax, owner grids; candidates handed to the patch).
#[allow(clippy::too_many_arguments)]
fn fresh_param(
    id: StampId,
    mesh: &Arc<MeshData>,
    bvh: &Bvh,
    graph: &WrapMeshCell,
    maps: &[Arc<TexelMap>],
    src_size: [u32; 2],
    p: &Projector,
) -> Option<Arc<WrapParam>> {
    let started = Instant::now();
    let g = graph.get(mesh);
    let graph_ms = started.elapsed().as_secs_f64() * 1e3;
    let _ = phase_times::take();
    let param = wrap_param_in(mesh, bvh, g, maps, src_size, p).map(Arc::new);
    let [collect, front, relax, owner] = phase_times::take().map(|us| us as f64 / 1e3);
    log::info!(
        "wrap phases: stamp {} graph {graph_ms:.1} ms, collect {collect:.1} ms ({} candidates), front {front:.1} ms, relax {relax:.1} ms, owner {owner:.1} ms",
        id.0,
        phase_times::last_candidates()
    );
    param
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

    /// Round 8: the kind of the job that made it.
    pub fn kind(&self) -> StampJobKind {
        self.kind
    }
}
