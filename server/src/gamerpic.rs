//! Player pictures, for the Xbox profile settings (GameDisplayPicRaw) and the web UI.
//!
//! Order: a PNG dropped by hand at data/gamerpics/<xuid>.png, then the public Xbox 360 avatar
//! service by gamertag (https://avatar-ssl.xboxlive.com/avatar/<gamertag>/avatarpic-l.png, no
//! login; 500 for unknown gamertags), cached on disk, then a generated placeholder: a
//! symmetric "identicon" seeded by the XUID, so it is random-looking but stable per player.
//!
//! The server is std-only (no TLS), so the download uses Windows' own curl.exe.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime},
};

/// Host the game is told to load pictures from; the hook forwards it to us like any other.
pub const HOST: &str = "gamerpic.fh2online";
/// After a failed lookup (unknown gamertag, no network), try again after this long.
const RETRY_AFTER: Duration = Duration::from_secs(3600);

pub fn url_for(xuid: u64) -> String {
    format!("http://{HOST}/pic?xuid={xuid}")
}

/// Where the picture came from, for the UI and logs.
pub enum Source {
    Manual,
    Xbox,
    Placeholder,
}

pub fn get(root: &Path, xuid: u64, gamertag: &str) -> (Vec<u8>, Source) {
    let dir = root.join("data").join("gamerpics");
    if let Ok(bytes) = fs::read(dir.join(format!("{xuid}.png"))) {
        return (bytes, Source::Manual);
    }
    if !gamertag.trim().is_empty() {
        if let Some(bytes) = xbox_avatar(&dir, gamertag.trim()) {
            return (bytes, Source::Xbox);
        }
    }
    (identicon(xuid, 208), Source::Placeholder)
}

fn safe_name(gamertag: &str) -> String {
    gamertag.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect()
}

fn xbox_avatar(dir: &Path, gamertag: &str) -> Option<Vec<u8>> {
    let cached: PathBuf = dir.join(format!("xbox-{}.png", safe_name(gamertag)));
    let failed = dir.join(format!("xbox-{}.none", safe_name(gamertag)));
    if let Ok(bytes) = fs::read(&cached) {
        return Some(bytes);
    }
    let recent_failure = fs::metadata(&failed)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < RETRY_AFTER);
    if recent_failure {
        return None;
    }
    let _ = fs::create_dir_all(dir);
    let encoded: String = gamertag
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect();
    let url = format!("https://avatar-ssl.xboxlive.com/avatar/{encoded}/avatarpic-l.png");
    let tmp = dir.join(format!("xbox-{}.tmp", safe_name(gamertag)));
    let ok = Command::new("curl.exe")
        .args(["-s", "-f", "-L", "--max-time", "8", "-o"])
        .arg(&tmp)
        .arg(&url)
        .status()
        .is_ok_and(|s| s.success());
    let bytes = if ok { fs::read(&tmp).ok().filter(|b| b.starts_with(b"\x89PNG")) } else { None };
    let _ = fs::remove_file(&tmp);
    match bytes {
        Some(b) => {
            let _ = fs::write(&cached, &b);
            Some(b)
        }
        None => {
            let _ = fs::write(&failed, gamertag);
            None
        }
    }
}

/// A 5x5 mirrored pattern on a gradient, colors from the XUID.
pub fn identicon(xuid: u64, size: usize) -> Vec<u8> {
    let mut h = xuid ^ 0x9e37_79b9_7f4a_7c15;
    let mut next = || {
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^= h >> 33;
        h
    };
    let hue = (next() % 360) as f32;
    let fg = hsl(hue, 0.75, 0.62);
    let bg_top = hsl((hue + 200.0) % 360.0, 0.45, 0.20);
    let bg_bottom = hsl((hue + 230.0) % 360.0, 0.50, 0.10);
    let bits = next();
    let cell = |x: usize, y: usize| {
        let col = if x < 3 { x } else { 4 - x }; // mirror
        bits >> (y * 3 + col) & 1 == 1
    };
    let margin = size / 8;
    let grid = (size - 2 * margin) / 5;
    let mut rgb = Vec::with_capacity(size * size * 3);
    for y in 0..size {
        let t = y as f32 / size as f32;
        let bg = [0, 1, 2].map(|i| (bg_top[i] as f32 * (1.0 - t) + bg_bottom[i] as f32 * t) as u8);
        for x in 0..size {
            let (gx, gy) = (x.wrapping_sub(margin) / grid.max(1), y.wrapping_sub(margin) / grid.max(1));
            let on = x >= margin && y >= margin && gx < 5 && gy < 5 && cell(gx, gy);
            rgb.extend_from_slice(if on { &fg } else { &bg });
        }
    }
    png_rgb(size, size, &rgb)
}

fn hsl(h: f32, s: f32, l: f32) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as u32 / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r, g, b].map(|v| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8)
}

/// Minimal PNG writer: 8-bit RGB, zlib with stored (uncompressed) deflate blocks.
fn png_rgb(width: usize, height: usize, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width * 3 + 1) * height);
    for row in rgb.chunks(width * 3) {
        raw.push(0); // filter: none
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, block) in blocks.iter().enumerate() {
        z.push((i + 1 == blocks.len()) as u8);
        let len = block.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &raw {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in [(b"IHDR", &ihdr), (b"IDAT", &z), (b"IEND", &Vec::new())] {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = png.len();
        png.extend_from_slice(kind);
        png.extend_from_slice(data);
        let crc = crc32(&png[start..]);
        png.extend_from_slice(&crc.to_be_bytes());
    }
    png
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    #[test]
    fn crc_reference() {
        // Standard check value of CRC-32/ISO-HDLC.
        assert_eq!(super::crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn placeholder_is_png() {
        let png = super::identicon(2533274790395904, 64);
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(png, super::identicon(2533274790395904, 64)); // stable per xuid
    }
}
