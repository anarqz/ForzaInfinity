//! server.cfg: an INI file next to the exe (or --config PATH). Every key is optional; a missing
//! file means all defaults.
//!
//! Syntax: `[Section]`, `Key=Value`, comments from `;` or `#` at the start of a line, and inline
//! comments from a `;` that follows whitespace (so `Name=Mundo #1` and `Message=a;b` keep their
//! text). Sections and keys are case-insensitive. A UTF-8 BOM (Notepad) is skipped. A bad value
//! is reported and the default kept.
//!
//! F0 uses MaxPlayers and Port. The rest is read and checked now so the file format is settled:
//! Visibility/Password/Announce are F1 (the world list), Allow/Ban are F2, UPnP is F3.

use std::{fmt, fs, io, path::Path};

pub const DEFAULT_PORT: u16 = 5101;
pub const DEFAULT_MAX_PLAYERS: u8 = 8;
/// The variant's maxAllowedPlayers.
pub const MAX_PLAYERS_LIMIT: u8 = 12;
pub const DEFAULT_ANNOUNCE: &str = "https://infinity.anarqz.dev";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Unlisted,
    Friends,
    Off,
}

impl Visibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Unlisted => "unlisted",
            Visibility::Friends => "friends",
            Visibility::Off => "off",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Config {
    pub name: String,
    pub message: String,
    pub visibility: Visibility,
    /// Never logged (Debug prints whether there is one).
    pub password: String,
    pub max_players: u8,
    pub port: u16,
    pub upnp: bool,
    pub announce: String,
    /// Gamertags; empty = everyone.
    pub allow: Vec<String>,
    pub ban: Vec<String>,
    /// [Log] Verbose: every protocol message in the log, as --verbose (the hook starts the
    /// relay with --config only, so a test turns it on here).
    pub verbose: bool,
    /// [Events] Races: the relay runs the races of the world's events (relay::Settings::races); on
    /// unless the host turns it off.
    pub races: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            name: String::new(),
            message: String::new(),
            visibility: Visibility::Public,
            password: String::new(),
            max_players: DEFAULT_MAX_PLAYERS,
            port: DEFAULT_PORT,
            upnp: true,
            announce: DEFAULT_ANNOUNCE.to_string(),
            allow: Vec::new(),
            ban: Vec::new(),
            verbose: false,
            races: true,
        }
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("name", &self.name)
            .field("message", &self.message)
            .field("visibility", &self.visibility)
            .field("password", &if self.password.is_empty() { "(none)" } else { "(set)" })
            .field("max_players", &self.max_players)
            .field("port", &self.port)
            .field("upnp", &self.upnp)
            .field("announce", &self.announce)
            .field("allow", &self.allow)
            .field("ban", &self.ban)
            .field("verbose", &self.verbose)
            .field("races", &self.races)
            .finish()
    }
}

fn strip_comment(line: &str) -> &str {
    let t = line.trim_start();
    if t.starts_with(';') || t.starts_with('#') {
        return "";
    }
    let bytes = line.as_bytes();
    for (i, &c) in bytes.iter().enumerate() {
        if c == b';' && i > 0 && (bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
            return &line[..i];
        }
    }
    line
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "sim" => Some(true),
        "0" | "false" | "no" | "off" | "nao" | "não" => Some(false),
        _ => None,
    }
}

fn parse_list(v: &str) -> Vec<String> {
    v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

impl Config {
    /// Parses the text; the second value lists the problems found (shown at startup).
    pub fn parse(text: &str) -> (Config, Vec<String>) {
        let mut c = Config::default();
        let mut warnings = Vec::new();
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut section = String::new();
        for (n, raw) in text.lines().enumerate() {
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            let at = n + 1;
            if let Some(rest) = line.strip_prefix('[') {
                match rest.strip_suffix(']') {
                    Some(s) => section = s.trim().to_ascii_lowercase(),
                    None => warnings.push(format!("linha {at}: seção mal formada: {line}")),
                }
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                warnings.push(format!("linha {at}: esperava Chave=Valor: {line}"));
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim();
            let bad = |w: &mut Vec<String>, what: &str| {
                w.push(format!("linha {at}: valor inválido para {key} ({what}); usando o padrão"))
            };
            match (section.as_str(), key.as_str()) {
                ("world", "name") => c.name = value.to_string(),
                ("world", "message") => c.message = value.to_string(),
                ("world", "visibility") => match value.to_ascii_lowercase().as_str() {
                    "public" => c.visibility = Visibility::Public,
                    "unlisted" => c.visibility = Visibility::Unlisted,
                    "friends" => c.visibility = Visibility::Friends,
                    "off" => c.visibility = Visibility::Off,
                    _ => bad(&mut warnings, "public, unlisted, friends ou off"),
                },
                ("world", "password") => c.password = value.to_string(),
                ("world", "maxplayers") => match value.parse::<u8>() {
                    Ok(n) if (1..=MAX_PLAYERS_LIMIT).contains(&n) => c.max_players = n,
                    _ => bad(&mut warnings, "1 a 12"),
                },
                ("network", "port") => match value.parse::<u16>() {
                    Ok(p) if p != 0 => c.port = p,
                    _ => bad(&mut warnings, "1 a 65535"),
                },
                ("network", "upnp") => match parse_bool(value) {
                    Some(b) => c.upnp = b,
                    None => bad(&mut warnings, "0 ou 1"),
                },
                ("network", "announce") => c.announce = value.to_string(),
                ("access", "allow") => c.allow = parse_list(value),
                ("access", "ban") => c.ban = parse_list(value),
                ("log", "verbose") => match parse_bool(value) {
                    Some(b) => c.verbose = b,
                    None => bad(&mut warnings, "0 ou 1"),
                },
                ("events", "races") => match parse_bool(value) {
                    Some(b) => c.races = b,
                    None => bad(&mut warnings, "0 ou 1"),
                },
                _ => warnings.push(format!("linha {at}: chave desconhecida [{section}] {key}")),
            }
        }
        (c, warnings)
    }

    /// Reads `path`; a missing file is all defaults (Ok(None) tells the caller it was missing).
    pub fn load(path: &Path) -> io::Result<Option<(Config, Vec<String>)>> {
        match fs::read(path) {
            Ok(bytes) => Ok(Some(Config::parse(&String::from_utf8_lossy(&bytes)))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbose_from_the_file() {
        let (c, w) = Config::parse("[Log]\nVerbose=1\n");
        assert!(c.verbose && w.is_empty());
        assert!(!Config::default().verbose);
        let (c, w) = Config::parse("[Events]\nRaces=0\n");
        assert!(!c.races && w.is_empty());
        assert!(Config::default().races);
    }

    #[test]
    fn defaults_when_empty() {
        let (c, w) = Config::parse("");
        assert_eq!(c, Config::default());
        assert!(w.is_empty());
        assert_eq!(c.port, 5101);
        assert_eq!(c.max_players, 8);
    }

    #[test]
    fn full_file() {
        let text = "\u{feff}; comentário\n\
            [World]\n\
            Name=Mundo #1 do Alex\n\
            Message=Bem-vindos;sem comentário aqui\n\
            Visibility=Unlisted   ; inline\n\
            Password=segredo\n\
            MaxPlayers=12\n\
            [network]\n\
            port = 6000\n\
            UPnP=0\n\
            Announce=http://127.0.0.1:8081\n\
            [Access]\n\
            Allow= a , b,,c \n\
            Ban=\n";
        let (c, w) = Config::parse(text);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.name, "Mundo #1 do Alex");
        assert_eq!(c.message, "Bem-vindos;sem comentário aqui");
        assert_eq!(c.visibility, Visibility::Unlisted);
        assert_eq!(c.password, "segredo");
        assert_eq!(c.max_players, 12);
        assert_eq!(c.port, 6000);
        assert!(!c.upnp);
        assert_eq!(c.announce, "http://127.0.0.1:8081");
        assert_eq!(c.allow, ["a", "b", "c"]);
        assert!(c.ban.is_empty());
        assert!(!format!("{c:?}").contains("segredo"));
    }

    #[test]
    fn example_file_is_the_defaults() {
        let (c, w) = Config::parse(include_str!("../server.cfg"));
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c, Config::default());
        // and with Windows line ends
        let crlf = include_str!("../server.cfg").replace('\n', "\r\n");
        assert_eq!(Config::parse(&crlf), (Config::default(), Vec::new()));
    }

    #[test]
    fn bad_values_keep_defaults() {
        let (c, w) = Config::parse("[World]\nMaxPlayers=13\nVisibility=everyone\n[Network]\nPort=0\nUPnP=maybe\nFoo=1\nbroken\n");
        assert_eq!(c, Config::default());
        assert_eq!(w.len(), 6, "{w:?}");
    }
}
