//! Minimal NBT skipping.
//!
//! Item stacks carry an optional NBT compound (enchants, display names, block
//! entity data). Nephele doesn't interpret item NBT yet, but it still has to
//! walk it to stay in sync with the rest of the packet, so this module parses
//! the tag tree and throws it away.

use crate::error::{Error, Result};
use crate::read::PacketReader;

const TAG_END: u8 = 0;
const TAG_LIST: u8 = 9;
const TAG_COMPOUND: u8 = 10;

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
    let _name = reader.read_string()?;
    skip_payload(reader, tag, 0)
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
        8 => reader.read_string().map(|_| ()),
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
            let _name = reader.read_string()?;
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

    #[test]
    fn skips_a_nested_compound() {
        let mut w = PacketWriter::new();
        // root compound
        w.write_u8(TAG_COMPOUND).write_string("display");
        // name: string inside
        w.write_u8(8).write_string("Name").write_string("Sword");
        // nested compound
        w.write_u8(TAG_COMPOUND).write_string("Enchantments");
        // list of compounds with one entry
        w.write_u8(TAG_LIST).write_string("List");
        w.write_u8(TAG_COMPOUND).write_i32(1);
        w.write_u8(2).write_string("id").write_u16(9);
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
