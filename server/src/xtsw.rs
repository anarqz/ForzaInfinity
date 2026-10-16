//! `bin/xtsw` codec for the Xls web services (Forza.WebServices / Xls.WebServices).
//!
//! The wire format was read from the game's own reader (forza_durango_release_final.exe):
//! - everything is big-endian and positional: fields follow the type descriptor order, no tags;
//! - `string` / `datetime`: u32 byte length (<= 0x2000) + UTF-8;
//! - `blob`: u32 length (<= 0x80000) + bytes; `array`: u32 count (<= 0x2000) + elements;
//! - `guid`: Data1 u32, Data2 u16, Data3 u16 (big-endian), Data4 8 raw bytes;
//! - a request body is the `_Parameters` struct;
//! - a response body is a 12-byte header `[u32 status][u32 code][u32 message length]`, then the
//!   message (only read when status != 0), then the `_Results` struct (only when status == 0).
//!   status 2 makes the client report HRESULT `0x80510000 | code`.
//!
//! Values are mapped to JSON: integers and floats as numbers, `u64`/`i64` as numbers, guids as
//! "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx", blobs as lowercase hex, datetimes as the ISO string
//! the game parses (`YYYY-MM-DD[THH:MM:SS[.mmm]][Z|+hh:mm|-hh:mm]`).

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;
use serde_json::{Map, Value};

const MAX_STRING: usize = 0x2000;
/// What the game's reader accepts (replies we send).
const MAX_BLOB: usize = 0x80000;
/// What we accept from the game (uploads such as ghosts can be bigger).
const MAX_BLOB_IN: usize = 32 << 20;
const MAX_ARRAY: usize = 0x2000;
/// Any valid date: the game rejects the whole response if a datetime fails to parse.
pub const DEFAULT_DATETIME: &str = "2000-01-01T00:00:00Z";

#[derive(Debug, Deserialize)]
pub struct Schema {
    pub services: BTreeMap<String, Service>,
    pub types: HashMap<String, TypeDef>,
}

#[derive(Debug, Deserialize)]
pub struct Service {
    pub methods: BTreeMap<String, MethodDef>,
}

#[derive(Debug, Deserialize)]
pub struct MethodDef {
    pub params: Option<String>,
    pub results: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TypeDef {
    pub fields: Vec<Field>,
}

#[derive(Debug, Deserialize)]
pub struct Field {
    pub name: String,
    pub kind: String,
    #[serde(rename = "type")]
    pub ty: Option<String>,
    pub elem: Option<Elem>,
    pub max_chars: Option<usize>,
    pub max_count: Option<usize>,
    pub max_len: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct Elem {
    pub kind: String,
    #[serde(rename = "type")]
    pub ty: Option<String>,
}

impl Schema {
    pub fn load_embedded() -> Schema {
        serde_json::from_str(include_str!("../schema.json")).expect("schema.json is invalid")
    }

    /// `class` is the X-ClassName without the assembly ("Forza.WebServices.User").
    pub fn method(&self, class: &str, method: &str) -> Option<&MethodDef> {
        self.services.get(class)?.methods.get(method)
    }

    pub fn decode(&self, ty: &str, bytes: &[u8]) -> Result<Value, String> {
        let mut r = Reader { b: bytes, pos: 0 };
        let v = self.read_struct(ty, &mut r)?;
        if r.pos != bytes.len() {
            return Err(format!("{} trailing bytes after {ty}", bytes.len() - r.pos));
        }
        Ok(v)
    }

    /// Missing fields are written as zero / empty / DEFAULT_DATETIME.
    pub fn encode(&self, ty: &str, value: &Value) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        self.write_struct(ty, value, &mut out, ty)?;
        Ok(out)
    }

    fn typedef(&self, ty: &str) -> Result<&TypeDef, String> {
        self.types.get(ty).ok_or_else(|| format!("unknown type {ty}"))
    }

    fn read_struct(&self, ty: &str, r: &mut Reader) -> Result<Value, String> {
        let mut obj = Map::new();
        for f in &self.typedef(ty)?.fields {
            let v = self
                .read_value(&f.kind, f.ty.as_deref().or(f.elem.as_ref().and_then(|e| e.ty.as_deref())), f.elem.as_ref(), r)
                .map_err(|e| format!("{ty}.{}: {e}", f.name))?;
            obj.insert(f.name.clone(), v);
        }
        Ok(Value::Object(obj))
    }

    fn read_value(&self, kind: &str, ty: Option<&str>, elem: Option<&Elem>, r: &mut Reader) -> Result<Value, String> {
        Ok(match kind {
            "u8" => r.take(1)?[0].into(),
            "bool" => (r.take(1)?[0] != 0).into(),
            "i16" => i16::from_be_bytes(r.arr()?).into(),
            "u16" => u16::from_be_bytes(r.arr()?).into(),
            "i32" => i32::from_be_bytes(r.arr()?).into(),
            "u32" => u32::from_be_bytes(r.arr()?).into(),
            "i64" => i64::from_be_bytes(r.arr()?).into(),
            "u64" => u64::from_be_bytes(r.arr()?).into(),
            "f32" => f32::from_be_bytes(r.arr()?).into(),
            "f64" => f64::from_be_bytes(r.arr()?).into(),
            "string" | "datetime" => {
                let n = r.len(MAX_STRING)?;
                String::from_utf8_lossy(r.take(n)?).into_owned().into()
            }
            "blob" => {
                let n = r.len(MAX_BLOB_IN)?;
                hex(r.take(n)?).into()
            }
            "guid" => guid_to_string(r.take(16)?).into(),
            "struct" => self.read_struct(ty.ok_or("struct without type")?, r)?,
            "array" => {
                let elem = elem.ok_or("array without element type")?;
                let n = r.len(MAX_ARRAY)?;
                let mut items = Vec::with_capacity(n);
                for i in 0..n {
                    items.push(
                        self.read_value(&elem.kind, elem.ty.as_deref(), None, r).map_err(|e| format!("[{i}] {e}"))?,
                    );
                }
                Value::Array(items)
            }
            other => return Err(format!("kind {other} is not supported by the game's reader")),
        })
    }

    fn write_struct(&self, ty: &str, value: &Value, out: &mut Vec<u8>, path: &str) -> Result<(), String> {
        let obj = match value {
            Value::Object(o) => Some(o),
            Value::Null => None,
            _ => return Err(format!("{path}: expected an object")),
        };
        for f in &self.typedef(ty)?.fields {
            let v = obj.and_then(|o| o.get(&f.name)).unwrap_or(&Value::Null);
            let p = format!("{path}.{}", f.name);
            self.write_field(f, v, out, &p)?;
        }
        Ok(())
    }

    fn write_field(&self, f: &Field, v: &Value, out: &mut Vec<u8>, path: &str) -> Result<(), String> {
        match f.kind.as_str() {
            "string" => {
                let s = v.as_str().unwrap_or("");
                let s: String = match f.max_chars {
                    // the game truncates too, but keep what we send identical to what it keeps
                    Some(max) if s.encode_utf16().count() > max => {
                        eprintln!("xtsw: {path} truncated to {max} UTF-16 units");
                        truncate_utf16(s, max)
                    }
                    _ => s.to_owned(),
                };
                put_bytes(out, s.as_bytes(), MAX_STRING, path)
            }
            "blob" => {
                let bytes = blob_from(v).map_err(|e| format!("{path}: {e}"))?;
                if let Some(max) = f.max_len {
                    if bytes.len() > max {
                        return Err(format!("{path}: blob of {} bytes, the game accepts at most {max}", bytes.len()));
                    }
                }
                put_bytes(out, &bytes, MAX_BLOB, path)
            }
            "array" => {
                let elem = f.elem.as_ref().ok_or_else(|| format!("{path}: array without element type"))?;
                let items: &[Value] = match v {
                    Value::Array(a) => a,
                    Value::Null => &[],
                    _ => return Err(format!("{path}: expected an array")),
                };
                let max = f.max_count.unwrap_or(MAX_ARRAY).min(MAX_ARRAY);
                if items.len() > max {
                    // the game would stop parsing and leave the rest of the reply unread
                    return Err(format!("{path}: {} items, the game accepts at most {max}", items.len()));
                }
                out.extend_from_slice(&(items.len() as u32).to_be_bytes());
                for (i, item) in items.iter().enumerate() {
                    let p = format!("{path}[{i}]");
                    if elem.kind == "struct" {
                        self.write_struct(elem.ty.as_deref().ok_or("struct without type")?, item, out, &p)?;
                    } else {
                        write_scalar(&elem.kind, item, out, &p)?;
                    }
                }
                Ok(())
            }
            "struct" => self.write_struct(f.ty.as_deref().ok_or_else(|| format!("{path}: no type"))?, v, out, path),
            kind => write_scalar(kind, v, out, path),
        }
    }
}

fn write_scalar(kind: &str, v: &Value, out: &mut Vec<u8>, path: &str) -> Result<(), String> {
    let err = |e: String| format!("{path}: {e}");
    match kind {
        "u8" => out.push(int(v).map_err(err)? as u8),
        "bool" => out.push(match v {
            Value::Bool(b) => *b as u8,
            other => (int(other).map_err(err)? != 0) as u8,
        }),
        "i16" | "u16" => out.extend_from_slice(&(int(v).map_err(err)? as u16).to_be_bytes()),
        "i32" | "u32" => out.extend_from_slice(&(int(v).map_err(err)? as u32).to_be_bytes()),
        "i64" | "u64" => out.extend_from_slice(&(int(v).map_err(err)? as u64).to_be_bytes()),
        "f32" => out.extend_from_slice(&(v.as_f64().unwrap_or(0.0) as f32).to_be_bytes()),
        "f64" => out.extend_from_slice(&v.as_f64().unwrap_or(0.0).to_be_bytes()),
        "datetime" => put_bytes(out, v.as_str().unwrap_or(DEFAULT_DATETIME).as_bytes(), MAX_STRING, path)?,
        "guid" => out.extend_from_slice(&guid_from(v).map_err(err)?),
        "string" | "blob" | "array" | "struct" => unreachable!(),
        other => return Err(format!("{path}: kind {other} is not supported by the game's reader")),
    }
    Ok(())
}

/// The 12-byte success header followed by the results.
pub fn ok_envelope(results: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; 12];
    out.extend_from_slice(results);
    out
}

/// Error reply: the client reports HRESULT 0x80510000 | (code & 0xffff) and logs `message`.
pub fn error_envelope(code: u32, message: &str) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&2u32.to_be_bytes());
    out.extend_from_slice(&code.to_be_bytes());
    out.extend_from_slice(&(message.len() as u32).to_be_bytes());
    out.extend_from_slice(message.as_bytes());
    out
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.b.len()).ok_or_else(|| {
            format!("needs {n} bytes at offset {}, only {} left", self.pos, self.b.len() - self.pos)
        })?;
        let s = &self.b[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn arr<const N: usize>(&mut self) -> Result<[u8; N], String> {
        Ok(self.take(N)?.try_into().unwrap())
    }

    fn len(&mut self, max: usize) -> Result<usize, String> {
        let n = u32::from_be_bytes(self.arr()?) as usize;
        if n > max { Err(format!("length {n} above the limit {max}")) } else { Ok(n) }
    }
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8], max: usize, path: &str) -> Result<(), String> {
    if bytes.len() > max {
        return Err(format!("{path}: {} bytes, limit {max}", bytes.len()));
    }
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

/// Integer from a JSON number, a decimal string or a "0x" hex string. Null is 0.
fn int(v: &Value) -> Result<i128, String> {
    match v {
        Value::Null => Ok(0),
        Value::Bool(b) => Ok(*b as i128),
        Value::Number(n) => n
            .as_i64()
            .map(i128::from)
            .or_else(|| n.as_u64().map(i128::from))
            .or_else(|| n.as_f64().map(|f| f as i128))
            .ok_or_else(|| "bad number".into()),
        Value::String(s) => {
            let s = s.trim();
            let parsed = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                Some(h) => i128::from_str_radix(h, 16),
                None => s.parse(),
            };
            parsed.map_err(|_| format!("not an integer: {s:?}"))
        }
        _ => Err("expected a number".into()),
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Result<Vec<u8>, String> {
    let s: String = s.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    if s.len() % 2 != 0 {
        return Err("odd number of hex digits".into());
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| "bad hex".into())).collect()
}

fn blob_from(v: &Value) -> Result<Vec<u8>, String> {
    match v {
        Value::Null => Ok(Vec::new()),
        Value::String(s) => unhex(s),
        Value::Array(a) => a.iter().map(|x| int(x).map(|n| n as u8)).collect(),
        _ => Err("blob must be a hex string or a byte array".into()),
    }
}

fn guid_to_string(b: &[u8]) -> String {
    let h = hex(b);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

fn guid_from(v: &Value) -> Result<[u8; 16], String> {
    match v {
        Value::Null => Ok([0; 16]),
        Value::String(s) => {
            let b = unhex(s.trim_matches(['{', '}']))?;
            b.try_into().map_err(|_| "a guid has 16 bytes".into())
        }
        _ => Err("guid must be a string".into()),
    }
}

fn truncate_utf16(s: &str, max: usize) -> String {
    let mut n = 0;
    s.chars()
        .take_while(|c| {
            n += c.len_utf16();
            n <= max
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine, engine::general_purpose::STANDARD as B64};

    /// The User.Logon body from captures/2026-10-13_first-capture.jsonl.
    const LOGON: &str = "AAkAAAAAAAAAAAAIUGxheWVyIDEAAAAAAAAAAAAAAAAAAAAWAAAAAAAAAAAAAAAAAAAABgAAAAAAAAMAAAAYaYjF3dxElp1sWN8FtsfXp3OQgKR1qmEgAAAAAAAAAAAAAD2OAUYBRgAAAAAAAAAA";

    #[test]
    fn logon_roundtrip() {
        let schema = Schema::load_embedded();
        let m = schema.method("Forza.WebServices.User", "Logon").unwrap();
        assert_eq!(m.params.as_deref(), Some("ForzaUserLogon_Parameters"));
        let body = B64.decode(LOGON).unwrap();
        let v = schema.decode("ForzaUserLogon_Parameters", &body).unwrap();
        let d = &v["loginData"];
        assert_eq!(d["xuid"], 0x0009000000000000u64);
        assert_eq!(d["gamerTag"], "Player 1");
        assert_eq!(d["LCID"], 22);
        assert_eq!(d["ageGroup"], 3);
        assert_eq!(d["clientProfile"]["profileId"].as_str().unwrap().len(), 48);
        assert_eq!(schema.encode("ForzaUserLogon_Parameters", &v).unwrap(), body);
    }

    #[test]
    fn empty_results_encode() {
        let schema = Schema::load_embedded();
        let bytes = schema.encode("ForzaUserLogon_Results", &Value::Null).unwrap();
        let back = schema.decode("ForzaUserLogon_Results", &bytes).unwrap();
        assert_eq!(back["responseData"]["allCarDiscount"]["StartDate"], DEFAULT_DATETIME);
    }
}
