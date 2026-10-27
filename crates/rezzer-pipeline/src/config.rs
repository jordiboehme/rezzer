// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The user's config file: `$XDG_CONFIG_HOME/rezzer/config.toml`, else
//! `~/.config/rezzer/config.toml` (on macOS too, as the spec says). The home is `HOME`, or
//! `USERPROFILE` when `HOME` is empty or unset, so on Windows the file is
//! `%USERPROFILE%\.config\rezzer\config.toml`. A home that is not an absolute path gives no
//! config file, so none is read from the working directory. A missing file means the
//! defaults; an unknown key is an error that names the file and the key.
//!
//! ```toml
//! cache = true
//! cache_dir = "/absolute/path"
//! cache_max_bytes = 536870912
//!
//! [limits]
//! max_file_bytes = 2147483648
//! max_inline_bytes = 33554432
//! max_time = 120
//! max_elements = 2000000
//! max_text_bytes = 268435456
//! page_tokens = 20000
//! part_size = 10000
//!
//! [enrich]
//! disabled = ["language"]
//!
//! [read]
//! table_style = "records"
//! ```

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use rezzer_enrich::EnrichOptions;
use rezzer_limits::Limits;
use rezzer_render::TableStyle;
use serde::Deserialize;

/// Settings for every run of the pipeline.
#[derive(Debug, Clone)]
pub struct Config {
    pub limits: Limits,
    /// Results are cached unless this is false.
    pub cache: bool,
    /// Where cache entries live; `None` is the user cache dir of [`cache_dir_from`]
    /// (`$XDG_CACHE_HOME/rezzer` when that is absolute, else `~/.cache/rezzer`, on Linux;
    /// `~/Library/Caches/rezzer` on macOS;
    /// `%LOCALAPPDATA%\rezzer\cache` on Windows), and no cache when there is none.
    pub cache_dir: Option<PathBuf>,
    /// Size cap of the cache dir. After a write over the cap the oldest entries are removed
    /// until the cache is under 90 % of it.
    pub cache_max_bytes: u64,
    pub enrich: EnrichOptions,
    pub read: ReadDefaults,
}

/// Defaults for how a read is written, from the `[read]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadDefaults {
    /// How sheet tables are written when a request does not say.
    pub table_style: TableStyle,
}

/// Default [`Config::cache_max_bytes`]: 512 MiB.
pub const DEFAULT_CACHE_MAX_BYTES: u64 = 512 * 1024 * 1024;

impl Default for Config {
    fn default() -> Self {
        Config {
            limits: Limits::default(),
            cache: true,
            cache_dir: None,
            cache_max_bytes: DEFAULT_CACHE_MAX_BYTES,
            enrich: EnrichOptions::default(),
            read: ReadDefaults::default(),
        }
    }
}

/// A config file that could not be used. The message is one line.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read config file {path}: {message}")]
    Io { path: String, message: String },
    #[error("config file {path}: {message}")]
    Invalid { path: String, message: String },
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ConfigFile {
    limits: Limits,
    cache: Option<bool>,
    cache_dir: Option<PathBuf>,
    cache_max_bytes: Option<u64>,
    enrich: EnrichFile,
    read: ReadFile,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ReadFile {
    /// Any type, so that a wrong one gives a message that names the key.
    table_style: Option<toml::Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct EnrichFile {
    disabled: Vec<String>,
}

impl Config {
    /// Loads the user's config file, or the defaults when there is none (or no home).
    pub fn load() -> Result<Config, ConfigError> {
        let xdg = std::env::var_os("XDG_CONFIG_HOME");
        let home = std::env::var_os("HOME");
        let user_profile = std::env::var_os("USERPROFILE");
        match config_path_from(xdg.as_deref(), home.as_deref(), user_profile.as_deref()) {
            Some(path) => Config::load_from(&path),
            None => Ok(Config::default()),
        }
    }

    /// Loads the config file at `path`. A missing file gives the defaults.
    pub fn load_from(path: &Path) -> Result<Config, ConfigError> {
        let shown = path.display().to_string();
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => {
                return Err(ConfigError::Io {
                    path: shown,
                    message: e.to_string(),
                });
            }
        };
        let invalid = |message: String| ConfigError::Invalid {
            path: shown.clone(),
            message,
        };
        let file: ConfigFile = toml::from_str(&text).map_err(|e| {
            let message = one_line(e.message());
            match e.span() {
                Some(span) => invalid(format!("line {}: {message}", line_of(&text, span.start))),
                None => invalid(message),
            }
        })?;
        if let Some(dir) = &file.cache_dir
            && !dir.is_absolute()
        {
            return Err(invalid(format!(
                "cache_dir must be an absolute path, got {}",
                dir.display()
            )));
        }
        rezzer_limits::check_part_size(file.limits.part_size)
            .map_err(|e| invalid(format!("limits.{e}")))?;
        rezzer_limits::check_inline_bytes(file.limits.max_inline_bytes)
            .map_err(|e| invalid(format!("limits.{e}")))?;
        let known: Vec<&str> = rezzer_enrich::passes().iter().map(|p| p.name()).collect();
        if let Some(bad) = file
            .enrich
            .disabled
            .iter()
            .find(|d| !known.contains(&d.as_str()))
        {
            return Err(invalid(format!(
                "enrich.disabled: unknown pass {bad:?}, expected one of {}",
                known.join(", ")
            )));
        }
        let table_style = match &file.read.table_style {
            None => TableStyle::default(),
            Some(toml::Value::String(text)) => text
                .parse::<TableStyle>()
                .map_err(|e| invalid(format!("read.table_style: {e}")))?,
            Some(other) => {
                return Err(invalid(format!(
                    "read.table_style: expected table or records, not {other}"
                )));
            }
        };
        Ok(Config {
            limits: file.limits,
            cache: file.cache.unwrap_or(true),
            cache_dir: file.cache_dir,
            cache_max_bytes: file.cache_max_bytes.unwrap_or(DEFAULT_CACHE_MAX_BYTES),
            enrich: EnrichOptions {
                disabled: file.enrich.disabled,
            },
            read: ReadDefaults { table_style },
        })
    }
}

/// The config file path from the values of `XDG_CONFIG_HOME`, `HOME` and `USERPROFILE`. An
/// empty or relative `XDG_CONFIG_HOME` is ignored, as the XDG base directory spec says. The
/// home is `HOME`, or `USERPROFILE` (which Windows sets) when `HOME`
/// is empty or unset. Without an absolute home there is no path, so a config file is never
/// read from the working directory. Absoluteness follows the rules of the system this runs on.
pub fn config_path_from(
    xdg_config_home: Option<&OsStr>,
    home: Option<&OsStr>,
    user_profile: Option<&OsStr>,
) -> Option<PathBuf> {
    let base = match xdg_config_home.filter(|v| host_absolute(v)) {
        Some(dir) => PathBuf::from(dir),
        None => home_dir(CacheOs::current(), home, user_profile)?.join(".config"),
    };
    Some(base.join("rezzer").join("config.toml"))
}

/// The system whose rules [`cache_dir_from`] follows; the config path uses the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheOs {
    /// Linux and every other Unix but macOS.
    Unix,
    MacOs,
    Windows,
}

impl CacheOs {
    /// The system this program was built for.
    pub(crate) fn current() -> CacheOs {
        if cfg!(windows) {
            CacheOs::Windows
        } else if cfg!(any(target_os = "macos", target_os = "ios")) {
            CacheOs::MacOs
        } else {
            CacheOs::Unix
        }
    }
}

/// The user cache dir of Rezzer from the values of `XDG_CACHE_HOME`, `HOME` and
/// `LOCALAPPDATA`, the same folder the `directories` crate (6.0.0) gave:
///
/// - Unix: `$XDG_CACHE_HOME/rezzer` when that is absolute, else `$HOME/.cache/rezzer`
/// - macOS: `$HOME/Library/Caches/rezzer` (`XDG_CACHE_HOME` is not used)
/// - Windows: `%LOCALAPPDATA%\rezzer\cache` (`HOME` is not used)
///
/// On Unix and macOS `HOME` must be set and absolute, even when `XDG_CACHE_HOME` is. A value
/// that is not absolute by the rules of `os` (an empty one included) gives `None`, so the
/// cache is off and never lands in the working directory. Absoluteness follows `os`, not the
/// system this runs on.
pub fn cache_dir_from(
    os: CacheOs,
    xdg_cache_home: Option<&OsStr>,
    home: Option<&OsStr>,
    local_app_data: Option<&OsStr>,
) -> Option<PathBuf> {
    match os {
        CacheOs::Windows => {
            let base = local_app_data.filter(|v| windows_absolute(v))?;
            Some(PathBuf::from(base).join("rezzer").join("cache"))
        }
        CacheOs::MacOs => {
            let home = home_dir(os, home, None)?;
            Some(home.join("Library/Caches").join("rezzer"))
        }
        CacheOs::Unix => {
            let home = home_dir(os, home, None)?;
            let base = match xdg_cache_home.filter(|v| unix_absolute(v)) {
                Some(xdg) => PathBuf::from(xdg),
                None => home.join(".cache"),
            };
            Some(base.join("rezzer"))
        }
    }
}

/// The user cache dir on this system, see [`cache_dir_from`].
pub(crate) fn default_cache_dir() -> Option<PathBuf> {
    let xdg = std::env::var_os("XDG_CACHE_HOME");
    let home = std::env::var_os("HOME");
    let local_app_data = std::env::var_os("LOCALAPPDATA");
    cache_dir_from(
        CacheOs::current(),
        xdg.as_deref(),
        home.as_deref(),
        local_app_data.as_deref(),
    )
}

/// Whether `path` is absolute on the system this runs on: the Windows rules on Windows, the
/// Unix rules everywhere else.
fn host_absolute(path: &OsStr) -> bool {
    absolute_on(CacheOs::current(), path)
}

/// Whether `path` is absolute by the rules of `os`.
fn absolute_on(os: CacheOs, path: &OsStr) -> bool {
    match os {
        CacheOs::Windows => windows_absolute(path),
        CacheOs::Unix | CacheOs::MacOs => unix_absolute(path),
    }
}

/// The home folder, the one place that finds it: `home` unless it is empty or unset, else
/// `user_profile` unless that is empty or unset. The value found must be absolute by the
/// rules of `os`, else there is no home (a relative home would put files in the working
/// directory). The cache passes no `user_profile`: on Unix and macOS it uses `HOME` only, as
/// the `directories` crate did, and on Windows it uses `LOCALAPPDATA`.
fn home_dir(os: CacheOs, home: Option<&OsStr>, user_profile: Option<&OsStr>) -> Option<PathBuf> {
    let dir = home
        .filter(|v| !v.is_empty())
        .or_else(|| user_profile.filter(|v| !v.is_empty()))?;
    absolute_on(os, dir).then(|| PathBuf::from(dir))
}

/// A Unix path is absolute when it starts with `/`.
fn unix_absolute(path: &OsStr) -> bool {
    path.as_encoded_bytes().first() == Some(&b'/')
}

/// A Windows path is absolute with a drive and a separator (`C:\`, `C:/`) or as a UNC or
/// device path (`\\server\share`, `\\?\C:\`). `C:x` and `\x` are relative to a current
/// folder.
fn windows_absolute(path: &OsStr) -> bool {
    let separator = |b: u8| b == b'\\' || b == b'/';
    match path.as_encoded_bytes() {
        [drive, b':', sep, ..] => drive.is_ascii_alphabetic() && separator(*sep),
        [b'\\', b'\\', ..] => true,
        _ => false,
    }
}

fn line_of(text: &str, offset: usize) -> usize {
    let end = offset.min(text.len());
    text.as_bytes()[..end]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

/// Collapses every run of whitespace and control characters, line breaks included, to one
/// space.
pub(crate) fn one_line(s: &str) -> String {
    rezzer_model::text::one_line(s)
}
