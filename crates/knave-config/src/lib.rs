//! Canonical Knave configuration loading, validation, and atomic writing.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use toml_edit::{DocumentMut, Item, Table, value};

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum SessionBackend {
    #[default]
    Auto,
    Tty,
    Winit,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionConfig {
    #[serde(default)]
    pub backend: SessionBackend,
    #[serde(default = "default_compositor_binary")]
    pub compositor_binary: String,
    #[serde(default = "default_shell_binary")]
    pub shell_binary: String,
    #[serde(default = "default_true")]
    pub restart_on_failure: bool,
}

fn default_compositor_binary() -> String {
    "villain".into()
}

fn default_shell_binary() -> String {
    "knave-shell".into()
}

fn default_true() -> bool {
    true
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            backend: SessionBackend::default(),
            compositor_binary: default_compositor_binary(),
            shell_binary: default_shell_binary(),
            restart_on_failure: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShellConfig {
    #[serde(default = "default_true")]
    pub start_bar: bool,
    #[serde(default = "default_true")]
    pub start_overview_service: bool,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            start_bar: true,
            start_overview_service: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Config {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub session: SessionConfig,
    #[serde(default)]
    pub shell: ShellConfig,
}

fn default_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            session: SessionConfig::default(),
            shell: ShellConfig::default(),
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(ConfigError::SchemaVersion {
                found: self.schema_version,
                expected: CURRENT_SCHEMA_VERSION,
            });
        }
        if self.session.compositor_binary.trim().is_empty() {
            return Err(ConfigError::Invalid(
                "session.compositor_binary is empty".into(),
            ));
        }
        if self.session.shell_binary.trim().is_empty() {
            return Err(ConfigError::Invalid("session.shell_binary is empty".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not determine Knave configuration path: {0}")]
    Path(String),
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not parse {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("could not parse editable document {path}: {source}")]
    Document {
        path: PathBuf,
        source: toml_edit::TomlError,
    },
    #[error("invalid Knave configuration: {0}")]
    Invalid(String),
    #[error("unsupported schema version {found}; expected {expected}")]
    SchemaVersion { found: u32, expected: u32 },
    #[error("could not write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

pub struct ConfigDocument {
    path: PathBuf,
    document: DocumentMut,
    config: Config,
}

impl ConfigDocument {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, ConfigError> {
        let path = path.into();
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(source) => {
                return Err(ConfigError::Read { path, source });
            }
        };

        let document = if source.trim().is_empty() {
            DocumentMut::new()
        } else {
            source
                .parse::<DocumentMut>()
                .map_err(|source| ConfigError::Document {
                    path: path.clone(),
                    source,
                })?
        };
        let config = if source.trim().is_empty() {
            Config::default()
        } else {
            toml::from_str::<Config>(&source).map_err(|source| ConfigError::Parse {
                path: path.clone(),
                source,
            })?
        };
        config.validate()?;

        Ok(Self {
            path,
            document,
            config,
        })
    }

    pub fn at_default_path() -> Result<Self, ConfigError> {
        Self::load(config_path()?)
    }

    pub fn default_path() -> Result<PathBuf, ConfigError> {
        config_path()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn source(&self) -> String {
        self.document.to_string()
    }

    pub fn write(&mut self, config: Config) -> Result<(), ConfigError> {
        config.validate()?;
        apply_config(&mut self.document, &config);
        let source = self.document.to_string();
        atomic_write(&self.path, source.as_bytes())?;
        self.config = config;
        Ok(())
    }

    pub fn write_default_at(path: impl Into<PathBuf>) -> Result<Self, ConfigError> {
        let path = path.into();
        if path.exists() {
            return Err(ConfigError::Invalid(format!(
                "configuration already exists at {}",
                path.display()
            )));
        }
        let mut document = Self::load(path.clone())?;
        document.write(Config::default())?;
        Ok(document)
    }
}

fn apply_config(document: &mut DocumentMut, config: &Config) {
    document["schema_version"] = value(i64::from(config.schema_version));
    set_table_value(
        document,
        "session",
        "backend",
        value(format!("{:?}", config.session.backend).to_lowercase()),
    );
    set_table_value(
        document,
        "session",
        "compositor_binary",
        value(config.session.compositor_binary.clone()),
    );
    set_table_value(
        document,
        "session",
        "shell_binary",
        value(config.session.shell_binary.clone()),
    );
    set_table_value(
        document,
        "session",
        "restart_on_failure",
        value(config.session.restart_on_failure),
    );
    set_table_value(
        document,
        "shell",
        "start_bar",
        value(config.shell.start_bar),
    );
    set_table_value(
        document,
        "shell",
        "start_overview_service",
        value(config.shell.start_overview_service),
    );
}

fn set_table_value(document: &mut DocumentMut, section: &str, key: &str, item: Item) {
    if !document.get(section).is_some_and(Item::is_table) {
        document[section] = Item::Table(Table::new());
    }
    document[section]
        .as_table_mut()
        .expect("section was initialized as a table")[key] = item;
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source: std::io::Error::other(source),
        })?
        .as_nanos();
    let temporary = path.with_extension(format!("tmp.{}.{}", std::process::id(), stamp));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|source| ConfigError::Write {
                path: temporary.clone(),
                source,
            })?;
        file.write_all(contents)
            .map_err(|source| ConfigError::Write {
                path: temporary.clone(),
                source,
            })?;
        file.sync_all().map_err(|source| ConfigError::Write {
            path: temporary.clone(),
            source,
        })?;
        fs::rename(&temporary, path).map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn config_path() -> Result<PathBuf, ConfigError> {
    if let Some(path) = std::env::var_os("KNAVE_CONFIG") {
        return Ok(path.into());
    }
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(path).join("knave/config.toml"));
    }
    let home =
        std::env::var_os("HOME").ok_or_else(|| ConfigError::Path("HOME is not set".into()))?;
    Ok(PathBuf::from(home).join(".config/knave/config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn unknown_toml_is_preserved_when_known_values_change() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(
            &path,
            "schema_version = 1\ncustom_value = \"keep\"\n[session]\nbackend = \"auto\"\n",
        )
        .unwrap();

        let mut document = ConfigDocument::load(&path).unwrap();
        let mut config = document.config().clone();
        config.session.restart_on_failure = false;
        document.write(config).unwrap();

        let output = fs::read_to_string(path).unwrap();
        assert!(output.contains("custom_value = \"keep\""));
        assert!(output.contains("restart_on_failure = false"));
    }
}
