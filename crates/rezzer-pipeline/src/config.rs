// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The user's config file: `$XDG_CONFIG_HOME/rezzer/config.toml`, else
//! `~/.config/rezzer/config.toml` (on macOS too, as the spec says). The home is `HOME`, or
//! `USERPROFILE` when `HOME` is empty or unset, so on Windows the file is
//! `%USERPROFILE%\.config\rezzer\config.toml`. A missing file means the defaults; an unknown
//! key is an error that names the file and the key.
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
    /// Where cache entries live; `None` is the user cache dir
    /// (`directories::ProjectDirs::cache_dir`).
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
/// home is `HOME`, or `USERPROFILE` (which Windows sets) when `HOME` is empty or unset.
pub fn config_path_from(
    xdg_config_home: Option<&OsStr>,
    home: Option<&OsStr>,
    user_profile: Option<&OsStr>,
) -> Option<PathBuf> {
    let xdg = xdg_config_home
        .map(PathBuf::from)
        .filter(|p| p.is_absolute());
    let base = match xdg {
        Some(dir) => dir,
        None => home_dir(home, user_profile)?.join(".config"),
    };
    Some(base.join("rezzer").join("config.toml"))
}

/// The home directory: `home` unless it is empty or unset, else `user_profile` unless that is
/// empty or unset.
fn home_dir(home: Option<&OsStr>, user_profile: Option<&OsStr>) -> Option<PathBuf> {
    let given = |v: Option<&OsStr>| v.filter(|v| !v.is_empty()).map(PathBuf::from);
    given(home).or_else(|| given(user_profile))
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
