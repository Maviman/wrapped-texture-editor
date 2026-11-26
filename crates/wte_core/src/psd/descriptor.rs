//! A small walker over Photoshop "action descriptors" (STICKER_PLAN 13.2 "Effects"; ARCHITECTURE 7.9).
//!
//! The effects blocks `lfx2` and `lmfx` hold a descriptor. WTE needs only two things from it: the master switch
//! (`masterFXSwitch`) and whether any effect is enabled (`enab`). To find them the walker must be able to step over
//! every item type a descriptor can contain, so it parses the whole tree into a light [`Value`] and keeps booleans,
//! nested descriptors and lists; every other value is skipped by its layout.
//!
//! A descriptor the walker cannot read is an error; the caller counts it as "effects on" (the safe side: the file
//! comes back as one merged picture).

/// Deepest nesting of descriptors and lists the walker follows (a malicious file cannot exhaust the stack).
const MAX_DEPTH: usize = 32;
/// Largest number of items in one descriptor, list or reference.
const MAX_ITEMS: u32 = 1 << 16;

/// The parts of a descriptor value the walker keeps.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Descriptor(Descriptor),
    List(Vec<Value>),
    Bool(bool),
    /// Any other value (numbers, strings, enums, references, raw data), stepped over.
    Other,
}

/// A descriptor: its class id and its items in file order.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Descriptor {
    pub class: Vec<u8>,
    pub items: Vec<(Vec<u8>, Value)>,
}

impl Descriptor {
    /// The first item with this key.
    pub fn get(&self, key: &[u8]) -> Option<&Value> {
        self.items.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// The boolean item with this key.
    pub fn bool(&self, key: &[u8]) -> Option<bool> {
        match self.get(key) {
            Some(Value::Bool(b)) => Some(*b),
            _ => None,
        }
    }
}

/// Why a descriptor could not be read.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DescriptorError(pub &'static str);

struct Walker<'a> {
    data: &'a [u8],
    pos: usize,
}

type R<T> = Result<T, DescriptorError>;

impl<'a> Walker<'a> {
    fn take(&mut self, n: usize) -> R<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(DescriptorError("length overflow"))?;
        let s = self.data.get(self.pos..end).ok_or(DescriptorError("descriptor ends early"))?;
        self.pos = end;
        Ok(s)
    }

    fn u8(&mut self) -> R<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> R<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn os_type(&mut self) -> R<[u8; 4]> {
        let b = self.take(4)?;
        Ok([b[0], b[1], b[2], b[3]])
    }

    /// A count of items, bounded so that a damaged count cannot make the walker loop for long.
    fn count(&mut self) -> R<u32> {
        let n = self.u32()?;
        // Every item needs at least one byte; more items than bytes left is damage.
        if n > MAX_ITEMS || n as usize > self.data.len() - self.pos {
            return Err(DescriptorError("item count larger than the descriptor"));
        }
        Ok(n)
    }

    /// Unicode string: a count of UTF-16 units, then the units.
    fn unicode(&mut self) -> R<()> {
        let n = self.u32()? as usize;
        self.take(n.checked_mul(2).ok_or(DescriptorError("length overflow"))?)?;
        Ok(())
    }

    /// "Length and key": a length, then that many bytes, or a 4-byte key when the length is 0.
    fn key(&mut self) -> R<Vec<u8>> {
        let n = self.u32()? as usize;
        let n = if n == 0 { 4 } else { n };
        Ok(self.take(n)?.to_vec())
    }

    /// Raw data with a 4-byte length.
    fn raw(&mut self) -> R<()> {
        let n = self.u32()? as usize;
        self.take(n)?;
        Ok(())
    }

    fn descriptor(&mut self, depth: usize) -> R<Descriptor> {
        if depth > MAX_DEPTH {
            return Err(DescriptorError("descriptor nested too deeply"));
        }
        self.unicode()?;
        let class = self.key()?;
        let n = self.count()?;
        let mut items = Vec::new();
        for _ in 0..n {
            let key = self.key()?;
            let ty = self.os_type()?;
            let value = self.value(ty, depth + 1)?;
            items.push((key, value));
        }
        Ok(Descriptor { class, items })
    }

    fn value(&mut self, ty: [u8; 4], depth: usize) -> R<Value> {
        if depth > MAX_DEPTH {
            return Err(DescriptorError("descriptor nested too deeply"));
        }
        Ok(match &ty {
            b"Objc" | b"GlbO" => Value::Descriptor(self.descriptor(depth + 1)?),
            b"VlLs" => {
                let n = self.count()?;
                let mut list = Vec::new();
                for _ in 0..n {
                    let ty = self.os_type()?;
                    list.push(self.value(ty, depth + 1)?);
                }
                Value::List(list)
            }
            b"bool" => Value::Bool(self.u8()? != 0),
            b"doub" => {
                self.take(8)?;
                Value::Other
            }
            b"UntF" => {
                self.take(4 + 8)?;
                Value::Other
            }
            b"UnFl" => {
                self.take(4)?;
                let n = self.count()? as usize;
                self.take(n * 8)?;
                Value::Other
            }
            b"TEXT" => {
                self.unicode()?;
                Value::Other
            }
            b"enum" => {
                self.key()?;
                self.key()?;
                Value::Other
            }
            b"long" => {
                self.take(4)?;
                Value::Other
            }
            b"comp" => {
                self.take(8)?;
                Value::Other
            }
            b"type" | b"GlbC" => {
                self.unicode()?;
                self.key()?;
                Value::Other
            }
            b"alis" | b"tdta" | b"Pth " => {
                self.raw()?;
                Value::Other
            }
            b"ObAr" => {
                // An object array: a count, a name, a class id, then items like a descriptor's.
                self.u32()?;
                self.unicode()?;
                self.key()?;
                let n = self.count()?;
                let mut items = Vec::new();
                for _ in 0..n {
                    let key = self.key()?;
                    let ty = self.os_type()?;
                    items.push((key, self.value(ty, depth + 1)?));
                }
                Value::Descriptor(Descriptor { class: b"ObAr".to_vec(), items })
            }
            b"obj " => {
                self.reference()?;
                Value::Other
            }
            _ => return Err(DescriptorError("unknown item type")),
        })
    }

    /// A reference (`obj `): a count, then items of type `prop`, `Clss`, `Enmr`, `rele`, `Idnt`, `indx`, `name`.
    fn reference(&mut self) -> R<()> {
        let n = self.count()?;
        for _ in 0..n {
            match &self.os_type()? {
                b"prop" => {
                    self.unicode()?;
                    self.key()?;
                    self.key()?;
                }
                b"Clss" => {
                    self.unicode()?;
                    self.key()?;
                }
                b"Enmr" => {
                    self.unicode()?;
                    self.key()?;
                    self.key()?;
                    self.key()?;
                }
                b"rele" => {
                    self.unicode()?;
                    self.key()?;
                    self.take(4)?;
                }
                b"Idnt" | b"indx" => {
                    self.take(4)?;
                }
                b"name" => {
                    self.unicode()?;
                    self.key()?;
                    self.unicode()?;
                }
                _ => return Err(DescriptorError("unknown reference item")),
            }
        }
        Ok(())
    }
}

/// Parses one descriptor (unicode name, class id, items) from the start of `data`. Bytes after it are ignored.
pub fn parse(data: &[u8]) -> Result<Descriptor, DescriptorError> {
    Walker { data, pos: 0 }.descriptor(0)
}

/// Parses an effects block body (`lfx2` or `lmfx`): a 4-byte object-effects version, a 4-byte descriptor version
/// (16), then the descriptor.
pub fn parse_effects_block(data: &[u8]) -> Result<Descriptor, DescriptorError> {
    let body = data.get(8..).ok_or(DescriptorError("effects block too short"))?;
    if data[4..8] != [0, 0, 0, 16] {
        return Err(DescriptorError("unknown descriptor version"));
    }
    parse(body)
}

/// True when the effects of an `lfx2` / `lmfx` body are switched on: the master switch is on (missing = on) and at
/// least one effect is enabled. An effect is a nested descriptor at the top level, or one inside a top-level list
/// (`dropShadowMulti`, ...); one without an `enab` item counts as enabled. A body the walker cannot read counts as
/// on (STICKER_PLAN 13.2).
pub fn effects_enabled(data: &[u8]) -> bool {
    let Ok(desc) = parse_effects_block(data) else {
        return true;
    };
    if desc.bool(b"masterFXSwitch") == Some(false) {
        return false;
    }
    let enabled = |v: &Value| matches!(v, Value::Descriptor(d) if d.bool(b"enab") != Some(false));
    desc.items.iter().any(|(_, v)| match v {
        Value::List(list) => list.iter().any(enabled),
        v => enabled(v),
    })
}
