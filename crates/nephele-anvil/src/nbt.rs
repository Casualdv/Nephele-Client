//! Minimal NBT reader for Anvil region data.
//!
//! NBT is big-endian tagged binary: every value is prefixed with its tag type
//! and, except inside lists and arrays, its name. Only reading is implemented;
//! that is all a world save or a parity check needs.

use indexmap::IndexMap;

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
    /// An empty NBT list still records its element tag id. Keep that id so an
    /// unknown empty list survives an edit/save round-trip byte-for-byte.
    EmptyList(u8),
    Compound(IndexMap<String, Tag>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Tag {
    pub fn as_compound(&self) -> Option<&IndexMap<String, Tag>> {
        match self {
            Tag::Compound(map) => Some(map),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Tag]> {
        match self {
            Tag::List(items) => Some(items),
            Tag::EmptyList(_) => Some(&[]),
            _ => None,
        }
    }

    pub fn as_byte_array(&self) -> Option<&[u8]> {
        match self {
            Tag::ByteArray(bytes) => Some(bytes),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i32> {
        match self {
            Tag::Int(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_long(&self) -> Option<i64> {
        match self {
            Tag::Long(value) => Some(*value),
            _ => None,
        }
    }

    pub fn get(&self, name: &str) -> Option<&Tag> {
        self.as_compound()?.get(name)
    }

    pub fn at(&self, path: &[&str]) -> Option<&Tag> {
        let mut current = self;
        for key in path {
            current = current.get(key)?;
        }
        Some(current)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NbtError {
    #[error("unexpected end of NBT data at offset {0}")]
    Eof(usize),
    #[error("unknown NBT tag {0} at offset {1}")]
    UnknownTag(u8, usize),
    #[error("NBT string is not valid UTF-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("NBT nesting exceeded {0} levels")]
    TooDeep(usize),
    #[error("NBT string is {0} bytes, longer than the 16-bit length limit")]
    StringTooLong(usize),
    #[error("NBT payload length {0} exceeds the 32-bit length limit")]
    LengthTooLarge(usize),
    #[error("NBT list contains different tag types")]
    HeterogeneousList,
    #[error("TAG_End cannot be used as a compound value")]
    EndTagValue,
    #[error("TAG_End cannot be an NBT root")]
    EndRoot,
}

const MAX_DEPTH: usize = 512;

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    depth: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], NbtError> {
        let end = self.pos.checked_add(count).ok_or(NbtError::Eof(self.pos))?;
        let slice = self
            .data
            .get(self.pos..end)
            .ok_or(NbtError::Eof(self.pos))?;
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, NbtError> {
        Ok(self.take(1)?[0])
    }

    fn i16(&mut self) -> Result<i16, NbtError> {
        Ok(i16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> Result<i32, NbtError> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i64(&mut self) -> Result<i64, NbtError> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn string(&mut self) -> Result<String, NbtError> {
        let length = self.i16()?;
        if length < 0 {
            return Ok(String::new());
        }
        let bytes = self.take(length as usize)?;
        Ok(std::str::from_utf8(bytes)?.to_string())
    }

    fn payload(&mut self, tag: u8) -> Result<Tag, NbtError> {
        Ok(match tag {
            0 => Tag::End,
            1 => Tag::Byte(self.u8()? as i8),
            2 => Tag::Short(self.i16()?),
            3 => Tag::Int(self.i32()?),
            4 => Tag::Long(self.i64()?),
            5 => Tag::Float(f32::from_be_bytes(self.take(4)?.try_into().unwrap())),
            6 => Tag::Double(f64::from_be_bytes(self.take(8)?.try_into().unwrap())),
            7 => {
                let length = self.i32()?.max(0) as usize;
                Tag::ByteArray(self.take(length)?.to_vec())
            }
            8 => Tag::String(self.string()?),
            9 => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return Err(NbtError::TooDeep(MAX_DEPTH));
                }
                let element = self.u8()?;
                let length = self.i32()?.max(0) as usize;
                if length == 0 {
                    self.depth -= 1;
                    return Ok(Tag::EmptyList(element));
                }
                let mut items = Vec::with_capacity(length.min(4096));
                for _ in 0..length {
                    items.push(self.payload(element)?);
                }
                self.depth -= 1;
                Tag::List(items)
            }
            10 => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return Err(NbtError::TooDeep(MAX_DEPTH));
                }
                let mut map = IndexMap::new();
                loop {
                    let kind = self.u8()?;
                    if kind == 0 {
                        break;
                    }
                    let name = self.string()?;
                    map.insert(name, self.payload(kind)?);
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
            other => return Err(NbtError::UnknownTag(other, self.pos)),
        })
    }
}

/// Reads a named root tag: `[type][name][payload]`.
pub fn parse(data: &[u8]) -> Result<(String, Tag), NbtError> {
    let mut reader = Reader {
        data,
        pos: 0,
        depth: 0,
    };
    let kind = reader.u8()?;
    if kind == 0 {
        return Ok((String::new(), Tag::End));
    }
    let name = reader.string()?;
    let tag = reader.payload(kind)?;
    Ok((name, tag))
}

/// Writes a named NBT root tag.
pub fn encode(name: &str, tag: &Tag) -> Result<Vec<u8>, NbtError> {
    let kind = tag_id(tag);
    if kind == 0 {
        return Err(NbtError::EndRoot);
    }

    let mut out = Vec::new();
    out.push(kind);
    write_string(&mut out, name)?;
    write_payload(&mut out, tag, 0)?;
    Ok(out)
}

fn tag_id(tag: &Tag) -> u8 {
    match tag {
        Tag::End => 0,
        Tag::Byte(_) => 1,
        Tag::Short(_) => 2,
        Tag::Int(_) => 3,
        Tag::Long(_) => 4,
        Tag::Float(_) => 5,
        Tag::Double(_) => 6,
        Tag::ByteArray(_) => 7,
        Tag::String(_) => 8,
        Tag::List(_) => 9,
        Tag::EmptyList(_) => 9,
        Tag::Compound(_) => 10,
        Tag::IntArray(_) => 11,
        Tag::LongArray(_) => 12,
    }
}

fn write_string(out: &mut Vec<u8>, value: &str) -> Result<(), NbtError> {
    let bytes = value.as_bytes();
    let length = u16::try_from(bytes.len()).map_err(|_| NbtError::StringTooLong(bytes.len()))?;
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

fn write_length(out: &mut Vec<u8>, length: usize) -> Result<(), NbtError> {
    let length = i32::try_from(length).map_err(|_| NbtError::LengthTooLarge(length))?;
    out.extend_from_slice(&length.to_be_bytes());
    Ok(())
}

fn write_payload(out: &mut Vec<u8>, tag: &Tag, depth: usize) -> Result<(), NbtError> {
    if depth > MAX_DEPTH {
        return Err(NbtError::TooDeep(MAX_DEPTH));
    }

    match tag {
        Tag::End => {}
        Tag::Byte(value) => out.push(*value as u8),
        Tag::Short(value) => out.extend_from_slice(&value.to_be_bytes()),
        Tag::Int(value) => out.extend_from_slice(&value.to_be_bytes()),
        Tag::Long(value) => out.extend_from_slice(&value.to_be_bytes()),
        Tag::Float(value) => out.extend_from_slice(&value.to_be_bytes()),
        Tag::Double(value) => out.extend_from_slice(&value.to_be_bytes()),
        Tag::ByteArray(values) => {
            write_length(out, values.len())?;
            out.extend_from_slice(values);
        }
        Tag::String(value) => write_string(out, value)?,
        Tag::List(values) => {
            let element_id = values.first().map(tag_id).unwrap_or(0);
            if element_id == 0 && !values.is_empty() {
                return Err(NbtError::EndTagValue);
            }
            if values.iter().any(|value| tag_id(value) != element_id) {
                return Err(NbtError::HeterogeneousList);
            }
            out.push(element_id);
            write_length(out, values.len())?;
            for value in values {
                write_payload(out, value, depth + 1)?;
            }
        }
        Tag::EmptyList(element_id) => {
            out.push(*element_id);
            write_length(out, 0)?;
        }
        Tag::Compound(values) => {
            for (name, value) in values {
                let kind = tag_id(value);
                if kind == 0 {
                    return Err(NbtError::EndTagValue);
                }
                out.push(kind);
                write_string(out, name)?;
                write_payload(out, value, depth + 1)?;
            }
            out.push(0);
        }
        Tag::IntArray(values) => {
            write_length(out, values.len())?;
            for value in values {
                out.extend_from_slice(&value.to_be_bytes());
            }
        }
        Tag::LongArray(values) => {
            write_length(out, values.len())?;
            for value in values {
                out.extend_from_slice(&value.to_be_bytes());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_compound_with_nested_values() {
        let mut bytes = vec![10u8]; // compound
        bytes.extend_from_slice(&[0, 4]); // name length
        bytes.extend_from_slice(b"root");
        // "a" -> int 7
        bytes.push(3);
        bytes.extend_from_slice(&[0, 1]);
        bytes.extend_from_slice(b"a");
        bytes.extend_from_slice(&7i32.to_be_bytes());
        // "list" -> list of 2 shorts
        bytes.push(9);
        bytes.extend_from_slice(&[0, 4]);
        bytes.extend_from_slice(b"list");
        bytes.push(2);
        bytes.extend_from_slice(&2i32.to_be_bytes());
        bytes.extend_from_slice(&1i16.to_be_bytes());
        bytes.extend_from_slice(&2i16.to_be_bytes());
        // end
        bytes.push(0);

        let (name, tag) = parse(&bytes).unwrap();
        assert_eq!(name, "root");
        assert_eq!(tag.get("a").and_then(Tag::as_int), Some(7));
        let list = tag.get("list").and_then(Tag::as_list).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], Tag::Short(1));
        assert_eq!(list[1], Tag::Short(2));
    }

    #[test]
    fn empty_payload_is_an_error_not_a_panic() {
        assert!(parse(&[]).is_err());
        assert!(parse(&[10]).is_err());
    }

    #[test]
    fn a_truncated_byte_array_reports_eof() {
        let mut bytes = vec![7u8, 0, 0];
        bytes.extend_from_slice(&16i32.to_be_bytes());
        bytes.extend_from_slice(&[1, 2, 3]);
        assert!(matches!(parse(&bytes), Err(NbtError::Eof(_))));
    }

    #[test]
    fn named_nbt_values_encode_and_parse_without_loss() {
        let mut nested = IndexMap::new();
        nested.insert("byte".to_string(), Tag::Byte(-3));
        nested.insert("short".to_string(), Tag::Short(1234));
        nested.insert("long".to_string(), Tag::Long(9_876_543_210));
        nested.insert("float".to_string(), Tag::Float(1.25));
        nested.insert("double".to_string(), Tag::Double(-2.5));
        nested.insert("bytes".to_string(), Tag::ByteArray(vec![0, 1, 255]));
        nested.insert("text".to_string(), Tag::String("Nephele".to_string()));
        nested.insert(
            "list".to_string(),
            Tag::List(vec![Tag::Int(5), Tag::Int(-8)]),
        );
        nested.insert("ints".to_string(), Tag::IntArray(vec![1, -2]));
        nested.insert("longs".to_string(), Tag::LongArray(vec![3, -4]));
        let root = Tag::Compound(nested);

        let bytes = encode("root", &root).unwrap();
        let (name, decoded) = parse(&bytes).unwrap();

        assert_eq!(name, "root");
        assert_eq!(decoded, root);
    }

    #[test]
    fn empty_list_element_type_is_preserved() {
        let mut root = IndexMap::new();
        root.insert("empty".into(), Tag::EmptyList(10));
        let original = encode("", &Tag::Compound(root)).unwrap();
        let (_, decoded) = parse(&original).unwrap();
        assert_eq!(encode("", &decoded).unwrap(), original);
    }

    #[test]
    fn rejects_mixed_type_lists() {
        let tag = Tag::List(vec![Tag::Int(1), Tag::String("two".to_string())]);
        assert!(matches!(
            encode("root", &tag),
            Err(NbtError::HeterogeneousList)
        ));
    }
}
