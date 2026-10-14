//! The game's string tables (media/stripped/stringtables/<language>.zip, one LSB2 .str per
//! table), so database names like DisplayName "_&3381852044" read "Nice Aérodrome Cross
//! Country".
//!
//! A reference "_&<n>" is a u32: the high 16 bits name the table, the low 16 bits are the key
//! of the string in it. LSB2 layout (little endian): "LSB2", header, at 0x28 the u32 entry
//! count, then entries of u16 key + u32 offset, 6 bytes of padding, then the UTF-16 texts
//! (each NUL-terminated); an offset counts UTF-16 units from the first text.

use std::collections::BTreeMap;

/// One file of a zip archive (stored or deflated), by name.
pub fn zip_entry(zip: &[u8], name: &str) -> Option<Vec<u8>> {
    let u16_at = |at: usize| zip.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let u32_at = |at: usize| zip.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    // end of central directory: the last "PK\x05\x06" (a comment may follow it)
    let end = (0..zip.len().saturating_sub(21)).rev().find(|&i| zip[i..].starts_with(b"PK\x05\x06"))?;
    let (count, mut at) = (u16_at(end + 10)?, u32_at(end + 16)?);
    for _ in 0..count {
        if !zip.get(at..)?.starts_with(b"PK\x01\x02") {
            return None;
        }
        let (method, packed, size) = (u16_at(at + 10)?, u32_at(at + 20)?, u32_at(at + 24)?);
        let (name_len, extra, comment, local) = (u16_at(at + 28)?, u16_at(at + 30)?, u16_at(at + 32)?, u32_at(at + 42)?);
        if zip.get(at + 46..at + 46 + name_len)? == name.as_bytes() {
            if !zip.get(local..)?.starts_with(b"PK\x03\x04") {
                return None;
            }
            let data = local + 30 + u16_at(local + 26)? + u16_at(local + 28)?;
            let raw = zip.get(data..data + packed)?;
            let out = match method {
                0 => raw.to_vec(),
                8 => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, size).ok()?,
                _ => return None,
            };
            return (out.len() == size).then_some(out);
        }
        at += 46 + name_len + extra + comment;
    }
    None
}

/// The strings of an LSB2 table by key.
pub fn parse(table: &[u8]) -> BTreeMap<u16, String> {
    let mut out = BTreeMap::new();
    if !table.starts_with(b"LSB2") || table.len() < 0x2c {
        return out;
    }
    let count = u32::from_le_bytes([table[0x28], table[0x29], table[0x2a], table[0x2b]]) as usize;
    let texts = 0x2c + 6 * count + 6;
    for i in 0..count {
        let e = 0x2c + 6 * i;
        let Some(entry) = table.get(e..e + 6) else { break };
        let key = u16::from_le_bytes([entry[0], entry[1]]);
        let offset = u32::from_le_bytes([entry[2], entry[3], entry[4], entry[5]]) as usize;
        let units: Vec<u16> = table
            .get(texts + 2 * offset..)
            .unwrap_or_default()
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        out.insert(key, String::from_utf16_lossy(&units));
    }
    out
}

/// The key inside its table of a "_&<n>" reference.
pub fn key(reference: &str) -> Option<u16> {
    reference.strip_prefix("_&")?.parse::<u32>().ok().map(|n| (n & 0xffff) as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsb2() {
        // two entries: key 0x72 -> "Ab", key 0xf2 -> "C"
        let mut t = b"LSB2".to_vec();
        t.resize(0x28, 0);
        t.extend_from_slice(&2u32.to_le_bytes());
        t.extend_from_slice(&[0x72, 0, 0, 0, 0, 0, 0xf2, 0, 3, 0, 0, 0]);
        t.extend_from_slice(&[0xff, 0xff, 0, 0, 0, 0]);
        for u in "Ab\0C\0".encode_utf16() {
            t.extend_from_slice(&u.to_le_bytes());
        }
        let m = parse(&t);
        assert_eq!(m.get(&0x72).map(String::as_str), Some("Ab"));
        assert_eq!(m.get(&0xf2).map(String::as_str), Some("C"));
        assert_eq!(key("_&3381852044"), Some(0xf78c));
        assert_eq!(key("Anthem"), None);
    }
}
