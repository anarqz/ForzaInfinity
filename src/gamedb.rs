//! Read-only catalog from the game's own database (media/stripped/gamedbRC.slt, plain SQLite):
//! car names, classes and tracks, so the UI can show "Toyota Supra RZ 1998" instead of 461.
//!
//! The official display names are hashed string ids ("_&3444158956") into compressed .str
//! tables, so for now names are built from MediaName ("TOY_SupraRZ_98") and the make table.

use std::{collections::BTreeMap, path::Path};

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Car {
    pub id: i64,
    pub name: String,
    pub make: String,
    pub model: String,
    pub year: i64,
    pub class_id: i64,
    /// Display PI (e.g. 608), from PerformanceIndex 0.608.
    pub pi: i64,
    pub media_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CarClass {
    pub id: i64,
    pub name: String,
    /// "#rrggbb"
    pub color: String,
    pub max_pi: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: i64,
    pub name: String,
    pub length_m: i64,
    pub reverse: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct Catalog {
    pub cars: BTreeMap<i64, Car>,
    pub classes: BTreeMap<i64, CarClass>,
    pub tracks: BTreeMap<i64, Track>,
    /// Data_Car_Buckets: race car buckets (drivatars must drive a car of the race's bucket).
    #[serde(skip)]
    pub car_bucket: BTreeMap<i64, i64>,
    /// bucket -> cars, hero cars first.
    #[serde(skip)]
    pub bucket_cars: BTreeMap<i64, Vec<i64>>,
}

impl Catalog {
    pub fn load(path: &Path) -> Result<Catalog, String> {
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let mut catalog = Catalog::default();

        let mut makes = BTreeMap::new();
        let mut stmt = db.prepare("SELECT ID, IconPathBase FROM List_CarMake").map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;
        for (id, base) in rows.flatten() {
            makes.insert(id, split_words(&base));
        }

        let mut stmt = db
            .prepare("SELECT Id, Year, MakeID, MediaName, ClassID, PerformanceIndex FROM Data_Car")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, f64>(5)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for (id, year, make_id, media, class_id, pi) in rows.flatten() {
            let make = makes.get(&make_id).cloned().unwrap_or_default();
            let model = model_from_media(&media);
            let name = format!("{make} {model} {year}").trim().to_owned();
            catalog.cars.insert(
                id,
                Car { id, name, make, model, year, class_id, pi: (pi * 1000.0).round() as i64, media_name: media },
            );
        }

        let mut stmt = db
            .prepare("SELECT Id, BadgeTexturePathPrefix, ClassColor, MaxDisplayPerformanceIndex FROM CarClasses")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?))
            })
            .map_err(|e| e.to_string())?;
        for (id, prefix, color, max_pi) in rows.flatten() {
            let name = prefix.trim_start_matches("CLASS_").to_owned();
            let rgb = (color as u32) & 0x00ff_ffff;
            catalog.classes.insert(id, CarClass { id, name, color: format!("#{rgb:06x}"), max_pi });
        }

        let mut stmt = db
            .prepare("SELECT CarId, CarBucket, BucketHero FROM Data_Car_Buckets ORDER BY BucketHero DESC, CarId")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
            .map_err(|e| e.to_string())?;
        for (car, bucket) in rows.flatten() {
            catalog.car_bucket.insert(car, bucket);
            catalog.bucket_cars.entry(bucket).or_default().push(car);
        }

        let mut stmt =
            db.prepare("SELECT id, MediaName, Length, IsReverse FROM Tracks").map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?))
            })
            .map_err(|e| e.to_string())?;
        for (id, media, length, reverse) in rows.flatten() {
            let mut name = split_words(&media);
            if reverse != 0 {
                name.push_str(" (reverso)");
            }
            catalog.tracks.insert(id, Track { id, name, length_m: length, reverse: reverse != 0 });
        }
        Ok(catalog)
    }

    pub fn car(&self, id: i64) -> Option<&Car> {
        self.cars.get(&id)
    }
}

/// "TOY_SupraRZ_98" -> "Supra RZ"
fn model_from_media(media: &str) -> String {
    let mut parts: Vec<&str> = media.split('_').collect();
    if parts.len() > 1 {
        parts.remove(0); // make code
    }
    if parts.len() > 1 && parts.last().is_some_and(|p| p.len() <= 2 && p.chars().all(|c| c.is_ascii_digit())) {
        parts.pop(); // year
    }
    split_words(&parts.join(" "))
}

/// "AstonMartin" -> "Aston Martin", "SupraRZ" -> "Supra RZ", "Dino246" -> "Dino 246".
fn split_words(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if i > 0 && c != ' ' && chars[i - 1] != ' ' {
            let prev = chars[i - 1];
            let next = chars.get(i + 1).copied();
            let boundary = (c.is_uppercase() && prev.is_lowercase())
                || (c.is_uppercase() && prev.is_uppercase() && next.is_some_and(|n| n.is_lowercase()))
                || (c.is_ascii_digit() && prev.is_alphabetic())
                || (c.is_alphabetic() && prev.is_ascii_digit() && c.is_uppercase());
            if boundary {
                out.push(' ');
            }
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(model_from_media("TOY_SupraRZ_98"), "Supra RZ");
        assert_eq!(model_from_media("FER_Dino246_69"), "Dino 246");
        assert_eq!(split_words("AstonMartin"), "Aston Martin");
        assert_eq!(split_words("LagunaSeca"), "Laguna Seca");
    }
}
