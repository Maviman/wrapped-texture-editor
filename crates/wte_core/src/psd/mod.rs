//! PSD and PSB reading for the editor link (stickers P5a; STICKER_PLAN 5.2, 13.2; ARCHITECTURE 5.28, 7.9).
//!
//! One reader for every writer (PhotoCraft, Photoshop, GIMP, ...): it does not branch on who wrote a file. It returns
//! the layers bottom to top as straight RGBA8, the user masks, the merged picture un-matted from white, and a set of
//! [`PsdFeatures`] per layer that says what WTE cannot draw. The link logic (P5c) decides from
//! [`PsdDoc::forcing`] whether a file comes back layer by layer or as one merged picture.
//!
//! `export/psd.rs` (the texture exporter) is a separate writer with its own strict test reader and is not used here,
//! except for the PackBits row decoder.
//!
//! Nothing here panics on any input: every length is checked against the bytes that remain, a half-written file is
//! an error (never a picture), and allocations are bounded by the bytes that could produce them.

pub mod descriptor;
pub mod read;

#[cfg(test)]
mod fixture_tests;
#[cfg(test)]
mod read_tests;
#[cfg(test)]
pub(crate) mod testkit;

pub use read::read;

use crate::layer::BlendMode;
use crate::math::IRect;
use crate::pixel::{AlphaBuf, CroppedBuf, PixelBuf};

/// Size limits of [`read`]. The defaults are the ones of STICKER_PLAN 5.2.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PsdLimits {
    /// Largest canvas width or height that is parsed at all; above it the file is refused ([`PsdError::TooLarge`]).
    pub max_side: u32,
    /// Pixels (layers and merged picture) are decoded only when both canvas sides are at most this ...
    pub max_decode_side: u32,
    /// ... and the canvas has at most this many pixels. Otherwise [`PsdDoc::too_large`] is set, `merged` is `None`
    /// and no layer has pixels. A single layer whose rectangle holds more pixels than this is not decoded either.
    pub max_decode_pixels: u64,
}

impl Default for PsdLimits {
    fn default() -> PsdLimits {
        PsdLimits { max_side: 30_000, max_decode_side: 8192, max_decode_pixels: 67_108_864 }
    }
}

/// Colour mode of a file WTE reads. Every other mode is refused with [`PsdRefusal`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PsdColorMode {
    Rgb,
    /// One grey channel, expanded to RGB.
    Grayscale,
}

/// A layer's blend mode: one of WTE's seven, or another key ([`PsdFeatures::BLEND_OTHER`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PsdBlend {
    Known(BlendMode),
    Other([u8; 4]),
}

impl PsdBlend {
    /// Maps a PSD blend key; `lddg` (Linear Dodge (Add)) is WTE's Add.
    pub fn from_key(key: [u8; 4]) -> PsdBlend {
        match &key {
            b"norm" => PsdBlend::Known(BlendMode::Normal),
            b"mul " => PsdBlend::Known(BlendMode::Multiply),
            b"scrn" => PsdBlend::Known(BlendMode::Screen),
            b"over" => PsdBlend::Known(BlendMode::Overlay),
            b"lddg" => PsdBlend::Known(BlendMode::Add),
            b"dark" => PsdBlend::Known(BlendMode::Darken),
            b"lite" => PsdBlend::Known(BlendMode::Lighten),
            _ => PsdBlend::Other(key),
        }
    }
}

bitflags::bitflags! {
    /// What a layer carries besides plain pixels (STICKER_PLAN 5.2, 13.2).
    #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
    pub struct PsdFeatures: u16 {
        /// Layer effects that are switched on (`lfx2` / `lmfx` decide; `lrFX` alone counts as on).
        const EFFECTS = 1 << 0;
        /// A group marker (`lsct` / `lsdk` types 1, 2, 3) or an artboard (`artb`).
        const GROUP = 1 << 1;
        /// An adjustment layer (`levl`, `curv`, `hue2`, ...).
        const ADJUSTMENT = 1 << 2;
        /// A vector mask (`vmsk` / `vsms`) on an ordinary pixel layer.
        const VECTOR_MASK = 1 << 3;
        /// A second ("real") user mask (channel -3) or mask parameters (density, feather).
        const MASK_EXTRA = 1 << 4;
        /// Fill opacity (`iOpa`) below 255.
        const FILL_OPACITY = 1 << 5;
        /// A blend key that is not one of WTE's seven (also `pass` on a group).
        const BLEND_OTHER = 1 << 6;
        /// Blending ranges ("Blend If") that are not all the default pair.
        const BLEND_IF = 1 << 7;
        /// A shape, fill or adjustment layer that stored no pixels (13.2 "Empty layers and NO_PIXELS").
        const NO_PIXELS = 1 << 8;
        /// A text layer (`TySh`); its pixels are current.
        const TEXT = 1 << 9;
        /// A smart object (`SoLd` / `PlLd` / `SoLE`); its pixels are current.
        const SMART = 1 << 10;
        /// A shape or fill layer (`SoCo`, `GdFl`, `PtFl`, `vscg`, `vstk`).
        const FILL = 1 << 11;
    }
}

impl PsdFeatures {
    /// Features WTE cannot draw: one of them on a visible layer makes the file come back as one merged picture.
    /// SMART is added by [`PsdDoc::forcing`] when the caller says so.
    pub const FORCING: PsdFeatures = PsdFeatures::EFFECTS
        .union(PsdFeatures::GROUP)
        .union(PsdFeatures::ADJUSTMENT)
        .union(PsdFeatures::VECTOR_MASK)
        .union(PsdFeatures::MASK_EXTRA)
        .union(PsdFeatures::FILL_OPACITY)
        .union(PsdFeatures::BLEND_OTHER)
        .union(PsdFeatures::BLEND_IF)
        .union(PsdFeatures::NO_PIXELS);

    /// The editor's own layer kinds that come back as pixels (`editor_native`).
    pub const NATIVE: PsdFeatures = PsdFeatures::TEXT.union(PsdFeatures::SMART).union(PsdFeatures::FILL);

    /// Plain words for each set feature, in a fixed order, for the sentence that says why a file came back merged
    /// ("layer effects", "groups", ...).
    pub fn words(self) -> Vec<&'static str> {
        const WORDS: [(PsdFeatures, &str); 12] = [
            (PsdFeatures::EFFECTS, "layer effects"),
            (PsdFeatures::GROUP, "groups"),
            (PsdFeatures::ADJUSTMENT, "adjustment layers"),
            (PsdFeatures::SMART, "smart objects"),
            (PsdFeatures::VECTOR_MASK, "vector masks"),
            (PsdFeatures::MASK_EXTRA, "mask density or feather"),
            (PsdFeatures::FILL_OPACITY, "fill opacity below 100 %"),
            (PsdFeatures::BLEND_OTHER, "other blend modes"),
            (PsdFeatures::BLEND_IF, "blend-if ranges"),
            (PsdFeatures::NO_PIXELS, "shape or fill layers without pixels"),
            (PsdFeatures::TEXT, "text layers"),
            (PsdFeatures::FILL, "shape or fill layers"),
        ];
        WORDS.iter().filter(|(f, _)| self.contains(*f)).map(|(_, w)| *w).collect()
    }
}

/// What a layer record is in the layer stack (`lsct` / `lsdk`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PsdSection {
    /// An ordinary layer (no section block, or section type 0).
    Layer,
    /// Start of an open group (type 1).
    GroupOpen,
    /// Start of a closed group (type 2).
    GroupClosed,
    /// The hidden end marker of a group (type 3, "</Layer group>").
    GroupEnd,
}

impl PsdSection {
    /// True for the three group marker kinds.
    pub fn is_group_marker(self) -> bool {
        self != PsdSection::Layer
    }
}

/// A layer's user mask (channel -2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PsdMask {
    /// The mask's rectangle in canvas pixels (`EMPTY` when the mask has no pixels).
    pub rect: IRect,
    /// The mask value outside `rect` (0 or 255).
    pub default: u8,
    /// `rect.width() x rect.height()` mask values.
    pub pixels: AlphaBuf,
    /// Mask flags bit 1: the mask is switched off.
    pub disabled: bool,
    /// Mask flags bit 3: the mask was rendered from other data (a vector mask).
    pub rendered: bool,
}

impl PsdMask {
    /// Mask value at canvas pixel `(x, y)`.
    pub fn value(&self, x: i32, y: i32) -> u8 {
        if self.rect.contains(x, y) {
            self.pixels.get((x - self.rect.x0) as u32, (y - self.rect.y0) as u32)
        } else {
            self.default
        }
    }
}

/// One layer record, bottom to top in [`PsdDoc::layers`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PsdLayer {
    /// `luni` when present, else the Pascal name (UTF-8 when valid, else Windows-1252).
    pub name: String,
    /// The layer id (`lyid`), when the writer stored one.
    pub lyid: Option<u32>,
    /// The layer's rectangle in canvas pixels; may reach outside the canvas; `EMPTY` for an empty layer (also when
    /// the file has `bottom < top` or `right < left`).
    pub rect: IRect,
    /// Straight RGBA8 over `rect`. `Some(CroppedBuf::empty())` for an ordinary empty layer; `None` for group
    /// markers, NO_PIXELS layers, a layer without colour channels and when pixels were not decoded (limits).
    pub pixels: Option<CroppedBuf>,
    /// The user mask; `None` also for a rendered (vector) mask of a shape or fill layer, which is already in its
    /// pixels (13.2).
    pub mask: Option<PsdMask>,
    /// Clipping byte != 0: clipped to the layer below.
    pub clipped: bool,
    /// Flags bit 1 clear.
    pub visible: bool,
    pub opacity: u8,
    pub blend: PsdBlend,
    pub features: PsdFeatures,
    pub section: PsdSection,
    /// The record's raw flags byte.
    pub flags: u8,
    /// Fill opacity (`iOpa`), 255 when absent.
    pub fill_opacity: u8,
}

/// A parsed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PsdDoc {
    /// Canvas width and height.
    pub size: [u32; 2],
    /// Bits per channel in the file: 8 or 16. Pixels are always returned as 8-bit.
    pub depth: u8,
    /// Version 2 (PSB).
    pub psb: bool,
    pub mode: PsdColorMode,
    /// Image resource 1057 byte 4 (`hasRealMergedData`); true when the resource is missing and always true for a
    /// flat file (no layers).
    pub has_real_merged: bool,
    /// The merged picture, straight RGBA8, un-matted from white; `None` when not real or not decoded.
    pub merged: Option<PixelBuf>,
    /// Layers bottom to top (from the layer info, or from the `Lr16` block of a 16-bit file).
    pub layers: Vec<PsdLayer>,
    /// The canvas is over the decode limits: no pixels were decoded.
    pub too_large: bool,
    /// The description of the embedded ICC profile (resource 1039), when there is one.
    pub icc_description: Option<String>,
}

impl PsdDoc {
    /// True when the file has no layers at all (after `Lr16` / `Lr32`): its merged picture is the document.
    pub fn is_flat(&self) -> bool {
        self.layers.is_empty()
    }

    /// The union of the forcing features of the file (STICKER_PLAN 13.2): every forcing feature of a visible layer,
    /// GROUP on any layer (a group's end marker is always hidden), and SMART when `smart_forces` is true (every
    /// link whose editor is not PhotoCraft). Empty means the file can come back layer by layer.
    pub fn forcing(&self, smart_forces: bool) -> PsdFeatures {
        let mut mask = PsdFeatures::FORCING;
        if smart_forces {
            mask |= PsdFeatures::SMART;
        }
        let mut out = PsdFeatures::empty();
        for layer in &self.layers {
            if layer.visible {
                out |= layer.features & mask;
            }
            out |= layer.features & PsdFeatures::GROUP;
        }
        out
    }

    /// Some layer is a text, smart-object or shape / fill layer of the editor: a rewrite from WTE would turn it into
    /// pixels (STICKER_PLAN 5.5).
    pub fn editor_native(&self) -> bool {
        self.layers.iter().any(|l| l.features.intersects(PsdFeatures::NATIVE))
    }

    /// The colour-profile note of 13.2: the profile's description when an RGB file embeds a profile whose
    /// description does not contain "sRGB". Grayscale files are exempt (Photoshop puts a grey profile into all).
    pub fn profile_note(&self) -> Option<&str> {
        if self.mode != PsdColorMode::Rgb {
            return None;
        }
        self.icc_description.as_deref().filter(|d| !d.contains("sRGB"))
    }
}

/// A file WTE does not read, with the sentence the user sees (STICKER_PLAN 13.2, decision D2).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PsdRefusal {
    /// Colour mode 0 (or depth 1).
    Bitmap,
    Indexed,
    Cmyk,
    Multichannel,
    Duotone,
    Lab,
    /// Any other colour mode number.
    OtherMode(u16),
    /// 32 bits per channel (floating point).
    Depth32,
    /// Any other depth than 1, 8, 16 or 32.
    OtherDepth(u16),
    /// A channel compression method other than 0 to 3.
    Compression(u16),
}

impl PsdRefusal {
    /// The sentence for the user; `name` is the file's name as the user knows it.
    pub fn sentence(&self, name: &str) -> String {
        let mode = |m: &str| {
            format!("{name} is in {m} mode. In Photoshop choose Image > Mode > RGB Color and save again.")
        };
        match self {
            PsdRefusal::Bitmap => mode("Bitmap"),
            PsdRefusal::Indexed => mode("Indexed Color"),
            PsdRefusal::Cmyk => mode("CMYK Color"),
            PsdRefusal::Multichannel => mode("Multichannel"),
            PsdRefusal::Duotone => mode("Duotone"),
            PsdRefusal::Lab => mode("Lab Color"),
            PsdRefusal::OtherMode(m) => {
                format!("{name} uses a colour mode WTE cannot read ({m}). In Photoshop choose Image > Mode > RGB Color and save again.")
            }
            PsdRefusal::Depth32 => format!(
                "{name} has 32 bits per channel. In Photoshop choose Image > Mode > 8 Bits/Channel and save again."
            ),
            PsdRefusal::OtherDepth(d) => format!(
                "{name} has {d} bits per channel. In Photoshop choose Image > Mode > 8 Bits/Channel and save again."
            ),
            PsdRefusal::Compression(c) => {
                format!("{name} uses a compression method WTE cannot read ({c}). Save it again with default settings.")
            }
        }
    }
}

/// Why [`read`] failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PsdError {
    /// The file ends before a part its lengths announce (a half-written file).
    #[error("the PSD file is incomplete")]
    Truncated,
    /// The bytes are not a well-formed PSD / PSB.
    #[error("the PSD file is damaged ({0})")]
    Corrupt(&'static str),
    /// A well-formed file of a kind WTE does not read; [`PsdRefusal::sentence`] says what to do.
    #[error("unsupported PSD file: {0:?}")]
    Unsupported(PsdRefusal),
    /// The canvas is larger than [`PsdLimits::max_side`].
    #[error("the PSD canvas is too large ({width} x {height})")]
    TooLarge { width: u32, height: u32 },
}
