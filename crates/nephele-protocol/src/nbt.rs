//! Minimal NBT reading.
//!
//! Item stacks carry an optional NBT compound, and block entities arrive as a
//! named NBT root. This module walks the tag tree; callers that need a value
//! (a sign's text, a banner's base colour, a chest's contents) use [`Tag`].

use std::collections::BTreeMap;

use crate::error::{Error, Result};
use crate::read::PacketReader;

const TAG_END: u8 = 0;
const TAG_LIST: u8 = 9;
const TAG_COMPOUND: u8 = 10;

/// A decoded NBT value. Compound keys use a `BTreeMap` so iteration is stable
/// in tests and debug output.
#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
    End,
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<u8>),
    String(String),
    List(Vec<Tag>),
    Compound(BTreeMap<String, Tag>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Tag {
    pub fn as_compound(&self) -> Option<&BTreeMap<String, Tag>> {
        match self {
            Tag::Compound(map) => Some(map),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Tag]> {
        match self {
            Tag::List(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Tag::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i32> {
        match self {
            Tag::Int(value) => Some(*value),
            Tag::Byte(value) => Some(*value as i32),
            Tag::Short(value) => Some(*value as i32),
            _ => None,
        }
    }

    pub fn as_byte(&self) -> Option<i8> {
        match self {
            Tag::Byte(value) => Some(*value),
            _ => None,
        }
    }

    pub fn get(&self, name: &str) -> Option<&Tag> {
        self.as_compound()?.get(name)
    }
}

/// Read one named root tag from `data` (the form used by `S35`).
pub fn parse_named_tag(data: &[u8]) -> Result<(String, Tag)> {
    let mut reader = NbtReader {
        data,
        pos: 0,
        depth: 0,
    };
    let kind = reader.u8()?;
    if kind == TAG_END {
        return Ok((String::new(), Tag::End));
    }
    let name = reader.string()?;
    let tag = reader.payload(kind)?;
    Ok((name, tag))
}

struct NbtReader<'a> {
    data: &'a [u8],
    pos: usize,
    depth: usize,
}

impl<'a> NbtReader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(count)
            .ok_or_else(|| Error::Protocol("NBT length overflow".into()))?;
        let bytes = self
            .data
            .get(self.pos..end)
            .ok_or(Error::UnexpectedEof { needed: count })?;
        self.pos = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn string(&mut self) -> Result<String> {
        let length = self.i16()?;
        if length < 0 {
            return Ok(String::new());
        }
        let bytes = self.take(length as usize)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|error| Error::Protocol(format!("NBT string is not UTF-8: {error}")))
    }

    fn payload(&mut self, kind: u8) -> Result<Tag> {
        Ok(match kind {
            TAG_END => Tag::End,
            1 => Tag::Byte(self.u8()? as i8),
            2 => Tag::Short(self.i16()?),
            3 => Tag::Int(self.i32()?),
            4 => Tag::Long(self.i64()?),
            5 => Tag::Float(f32::from_bits(self.i32()? as u32)),
            6 => Tag::Double(f64::from_bits(self.i64()? as u64)),
            7 => {
                let length = self.i32()?.max(0) as usize;
                Tag::ByteArray(self.take(length)?.to_vec())
            }
            8 => Tag::String(self.string()?),
            TAG_LIST => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return Err(Error::Protocol("NBT nested too deeply".into()));
                }
                let element = self.u8()?;
                let length = self.i32()?.max(0) as usize;
                let mut items = Vec::with_capacity(length.min(4096));
                for _ in 0..length {
                    items.push(self.payload(element)?);
                }
                self.depth -= 1;
                Tag::List(items)
            }
            TAG_COMPOUND => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return Err(Error::Protocol("NBT nested too deeply".into()));
                }
                let mut map = BTreeMap::new();
                loop {
                    let child = self.u8()?;
                    if child == TAG_END {
                        break;
                    }
                    let name = self.string()?;
                    map.insert(name, self.payload(child)?);
                }
                self.depth -= 1;
                Tag::Compound(map)
            }
            11 => {
                let length = self.i32()?.max(0) as usize;
                let mut values = Vec::with_capacity(length);
                for _ in 0..length {
                    values.push(self.i32()?);
                }
                Tag::IntArray(values)
            }
            12 => {
                let length = self.i32()?.max(0) as usize;
                let mut values = Vec::with_capacity(length);
                for _ in 0..length {
                    values.push(self.i64()?);
                }
                Tag::LongArray(values)
            }
            other => return Err(Error::Protocol(format!("unknown NBT tag type {other}"))),
        })
    }
}

/// Maximum nesting depth accepted while skipping. Vanilla's own limit is 512;
/// anything deeper than this is either corrupt or hostile.
const MAX_DEPTH: usize = 64;

/// Skip a *named* tag (the form used by item stacks): a type byte, and when it
/// isn't `TAG_End`, a name followed by the payload.
pub fn skip_named_tag(reader: &mut PacketReader<'_>) -> Result<()> {
    let tag = reader.read_u8()?;
    if tag == TAG_END {
        return Ok(());
    }
    skip_nbt_string(reader)?;
    skip_payload(reader, tag, 0)
}

/// NBT strings use an unsigned-short byte length, unlike protocol strings,
/// which use VarInts.
fn skip_nbt_string(reader: &mut PacketReader<'_>) -> Result<()> {
    let length = reader.read_u16()? as usize;
    reader.take(length).map(|_| ())
}

fn skip_payload(reader: &mut PacketReader<'_>, tag: u8, depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(Error::Protocol("NBT nested too deeply".into()));
    }
    match tag {
        TAG_END => Ok(()),
        1 => reader.read_u8().map(|_| ()),
        2 => reader.read_u16().map(|_| ()),
        3 => reader.read_i32().map(|_| ()),
        4 => reader.read_i64().map(|_| ()),
        5 => reader.read_f32().map(|_| ()),
        6 => reader.read_f64().map(|_| ()),
        7 => {
            let length = reader.read_i32()?.max(0) as usize;
            reader.take(length).map(|_| ())
        }
        8 => skip_nbt_string(reader),
        TAG_LIST => {
            let element_type = reader.read_u8()?;
            let length = reader.read_i32()?.max(0) as usize;
            for _ in 0..length {
                skip_payload(reader, element_type, depth + 1)?;
            }
            Ok(())
        }
        TAG_COMPOUND => loop {
            let child = reader.read_u8()?;
            if child == TAG_END {
                return Ok(());
            }
            skip_nbt_string(reader)?;
            skip_payload(reader, child, depth + 1)?;
        },
        11 => {
            let length = reader.read_i32()?.max(0) as usize;
            reader.take(length * 4).map(|_| ())
        }
        other => Err(Error::Protocol(format!("unknown NBT tag type {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write::PacketWriter;

    fn write_nbt_string(writer: &mut PacketWriter, value: &str) {
        writer
            .write_u16(value.len() as u16)
            .write_bytes(value.as_bytes());
    }

    #[test]
    fn skips_a_nested_compound() {
        let mut w = PacketWriter::new();
        // root compound
        w.write_u8(TAG_COMPOUND);
        write_nbt_string(&mut w, "display");
        // name: string inside
        w.write_u8(8);
        write_nbt_string(&mut w, "Name");
        write_nbt_string(&mut w, "Sword");
        // nested compound
        w.write_u8(TAG_COMPOUND);
        write_nbt_string(&mut w, "Enchantments");
        // list of compounds with one entry
        w.write_u8(TAG_LIST);
        write_nbt_string(&mut w, "List");
        w.write_u8(TAG_COMPOUND).write_i32(1);
        w.write_u8(2);
        write_nbt_string(&mut w, "id");
        w.write_u16(9);
        w.write_u8(TAG_END); // ends the list entry
        w.write_u8(TAG_END); // ends "Enchantments"
        w.write_u8(TAG_END); // ends root
        w.write_i32(0x1234_5678);

        let bytes = w.into_bytes();
        let mut r = PacketReader::new(&bytes);
        skip_named_tag(&mut r).unwrap();
        assert_eq!(r.read_i32().unwrap(), 0x1234_5678);
    }

    #[test]
    fn empty_tag_is_a_no_op() {
        let mut w = PacketWriter::new();
        w.write_u8(TAG_END).write_i32(7);
        let bytes = w.into_bytes();
        let mut r = PacketReader::new(&bytes);
        skip_named_tag(&mut r).unwrap();
        assert_eq!(r.read_i32().unwrap(), 7);
    }

    #[test]
    fn unknown_tag_type_is_an_error() {
        let mut w = PacketWriter::new();
        w.write_u8(42).write_i32(0);
        let bytes = w.into_bytes();
        let mut r = PacketReader::new(&bytes);
        assert!(skip_named_tag(&mut r).is_err());
    }
}

#[cfg(test)]
mod value_tests {
    use super::*;
    use crate::write::PacketWriter;

    #[test]
    fn parses_named_compound_values() {
        let mut w = PacketWriter::new();
        w.write_u8(10).write_u16(4).write_bytes(b"root");
        w.write_u8(8).write_u16(2).write_bytes(b"id");
        w.write_u16(4).write_bytes(b"Sign");
        w.write_u8(3).write_u16(5).write_bytes(b"Level");
        w.write_i32(3);
        w.write_u8(9).write_u16(8).write_bytes(b"Patterns");
        w.write_u8(10).write_i32(1);
        w.write_u8(8).write_u16(7).write_bytes(b"Pattern");
        w.write_u16(4).write_bytes(b"base");
        w.write_u8(0);
        w.write_u8(0);
        let (name, tag) = parse_named_tag(&w.into_bytes()).unwrap();
        assert_eq!(name, "root");
        assert_eq!(tag.get("id").and_then(Tag::as_str), Some("Sign"));
        assert_eq!(tag.get("Level").and_then(Tag::as_int), Some(3));
        let patterns = tag.get("Patterns").and_then(Tag::as_list).unwrap();
        assert_eq!(patterns.len(), 1);
        assert_eq!(
            patterns[0].get("Pattern").and_then(Tag::as_str),
            Some("base")
        );
    }
}
