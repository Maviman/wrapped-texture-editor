//! Pure decoding of one Windows pointer history entry of a pen (branch `pen-input`, ARCHITECTURE 8.13 / PEN_INPUT_PLAN
//! 4.2): pointer flags, pen flags, pen mask and raw pressure -> the fields of a `PenRecord`. No FFI here, so every rule is
//! unit-tested on any machine.

use super::{PenKind, PenRecord};

/// `POINTER_FLAGS` (winuser.h).
#[allow(dead_code, reason = "documented flag, not read")]
pub const POINTER_FLAG_INRANGE: u32 = 0x2;
pub const POINTER_FLAG_INCONTACT: u32 = 0x4;
#[allow(dead_code, reason = "documented flag, not read")]
pub const POINTER_FLAG_FIRSTBUTTON: u32 = 0x10;
pub const POINTER_FLAG_SECONDBUTTON: u32 = 0x20;
pub const POINTER_FLAG_THIRDBUTTON: u32 = 0x40;
#[allow(dead_code, reason = "documented flag, not read")]
pub const POINTER_FLAG_CANCELED: u32 = 0x8000;
pub const POINTER_FLAG_DOWN: u32 = 0x1_0000;
pub const POINTER_FLAG_UPDATE: u32 = 0x2_0000;
pub const POINTER_FLAG_UP: u32 = 0x4_0000;
#[allow(dead_code, reason = "documented flag, not read")]
pub const POINTER_FLAG_CAPTURECHANGED: u32 = 0x20_0000;
/// `PEN_FLAGS`.
pub const PEN_FLAG_BARREL: u32 = 0x1;
pub const PEN_FLAG_INVERTED: u32 = 0x2;
pub const PEN_FLAG_ERASER: u32 = 0x4;
/// `PEN_MASK`: the pressure field is valid.
pub const PEN_MASK_PRESSURE: u32 = 0x1;
/// The pen pressure range of Windows Ink (`POINTER_PEN_INFO::pressure`, 0..=1024).
pub const MAX_PRESSURE: u32 = 1024;

/// The raw fields of one `POINTER_PEN_INFO` history entry that the app reads.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct RawPen {
    pub pointer_flags: u32,
    pub pen_flags: u32,
    pub pen_mask: u32,
    pub pressure: u32,
}

/// The record of one history entry; `None` for an entry without DOWN / UP / UPDATE (winit skips those too, so the
/// records and winit's `Touch` events stay in step). DOWN wins over UP over UPDATE, as in winit.
pub fn decode(pointer_id: u32, raw: &RawPen) -> Option<PenRecord> {
    let has = |flag: u32| raw.pointer_flags & flag != 0;
    let kind = if has(POINTER_FLAG_DOWN) {
        PenKind::Down
    } else if has(POINTER_FLAG_UP) {
        PenKind::Up
    } else if has(POINTER_FLAG_UPDATE) {
        PenKind::Update
    } else {
        return None;
    };
    let in_contact = has(POINTER_FLAG_INCONTACT);
    let pressure = if raw.pen_mask & PEN_MASK_PRESSURE == 0 {
        1.0
    } else if in_contact {
        raw.pressure.clamp(1, MAX_PRESSURE) as f32 / MAX_PRESSURE as f32
    } else {
        0.0
    };
    Some(PenRecord {
        pointer_id,
        kind,
        in_contact,
        pressure,
        barrel: raw.pen_flags & PEN_FLAG_BARREL != 0 || has(POINTER_FLAG_SECONDBUTTON),
        third: has(POINTER_FLAG_THIRDBUTTON),
        inverted: raw.pen_flags & (PEN_FLAG_INVERTED | PEN_FLAG_ERASER) != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RANGE: u32 = POINTER_FLAG_INRANGE;
    const CONTACT: u32 = POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT | POINTER_FLAG_FIRSTBUTTON;

    fn raw(pointer_flags: u32, pen_flags: u32, pressure: u32) -> RawPen {
        RawPen { pointer_flags, pen_flags, pen_mask: PEN_MASK_PRESSURE, pressure }
    }

    #[test]
    fn tip_down_and_contact_pressure() {
        let r = decode(7, &raw(CONTACT | POINTER_FLAG_DOWN, 0, 512)).unwrap();
        assert_eq!(r.kind, PenKind::Down);
        assert!(r.in_contact && !r.barrel && !r.third && !r.inverted);
        assert_eq!((r.pointer_id, r.pressure), (7, 0.5));
        let r = decode(7, &raw(CONTACT | POINTER_FLAG_UPDATE, 0, 1024)).unwrap();
        assert_eq!((r.kind, r.pressure), (PenKind::Update, 1.0));
        let r = decode(7, &raw(RANGE | POINTER_FLAG_UP, 0, 0)).unwrap();
        assert_eq!((r.kind, r.in_contact, r.pressure), (PenKind::Up, false, 0.0));
    }

    #[test]
    fn hover_barrel_and_eraser() {
        let hover = decode(3, &raw(RANGE | POINTER_FLAG_UPDATE, 0, 0)).unwrap();
        assert_eq!((hover.kind, hover.in_contact, hover.pressure), (PenKind::Update, false, 0.0));
        let barrel_hover = decode(3, &raw(RANGE | POINTER_FLAG_UPDATE, PEN_FLAG_BARREL, 0)).unwrap();
        assert!(barrel_hover.barrel && !barrel_hover.in_contact);
        // Barrel held at tip-down: Windows reports the second button.
        let barrel_contact = decode(3, &raw(CONTACT | POINTER_FLAG_SECONDBUTTON | POINTER_FLAG_DOWN, 0, 300)).unwrap();
        assert!(barrel_contact.barrel && barrel_contact.in_contact);
        let third = decode(3, &raw(CONTACT | POINTER_FLAG_THIRDBUTTON | POINTER_FLAG_DOWN, 0, 300)).unwrap();
        assert!(third.third && !third.barrel);
        let inverted_hover = decode(3, &raw(RANGE | POINTER_FLAG_UPDATE, PEN_FLAG_INVERTED, 0)).unwrap();
        assert!(inverted_hover.inverted && !inverted_hover.in_contact);
        let eraser_contact = decode(3, &raw(CONTACT | POINTER_FLAG_DOWN, PEN_FLAG_ERASER, 900)).unwrap();
        assert!(eraser_contact.inverted && eraser_contact.in_contact);
        assert!((eraser_contact.pressure - 900.0 / 1024.0).abs() < 1e-6);
    }

    #[test]
    fn pressure_edges() {
        // No pressure in the mask: full pressure (a pen without pressure paints like a mouse).
        let no_mask = RawPen { pointer_flags: CONTACT | POINTER_FLAG_UPDATE, pen_flags: 0, pen_mask: 0, pressure: 0 };
        assert_eq!(decode(1, &no_mask).unwrap().pressure, 1.0);
        // 0 in contact is the lightest touch, not "no pressure".
        assert_eq!(decode(1, &raw(CONTACT | POINTER_FLAG_UPDATE, 0, 0)).unwrap().pressure, 1.0 / 1024.0);
        assert_eq!(decode(1, &raw(CONTACT | POINTER_FLAG_UPDATE, 0, 50_000)).unwrap().pressure, 1.0, "clamped");
    }

    #[test]
    fn entries_without_a_phase_are_skipped() {
        assert_eq!(decode(1, &raw(RANGE, 0, 0)), None);
        assert_eq!(decode(1, &raw(CONTACT | POINTER_FLAG_CAPTURECHANGED, 0, 10)), None);
        // DOWN wins over UP over UPDATE (winit's order).
        let both = decode(1, &raw(CONTACT | POINTER_FLAG_DOWN | POINTER_FLAG_UPDATE, 0, 10)).unwrap();
        assert_eq!(both.kind, PenKind::Down);
        let up = decode(1, &raw(POINTER_FLAG_UP | POINTER_FLAG_UPDATE | POINTER_FLAG_CANCELED, 0, 10)).unwrap();
        assert_eq!(up.kind, PenKind::Up);
    }
}
