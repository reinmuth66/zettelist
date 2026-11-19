//! 設定ファイル (`~/.config/zettelist/config.toml`) の読み込み。
//! ファイルがない、または項目が書かれていないときは既定値を使う。

use serde::Deserialize;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub omnigraph: OmnigraphConfig,
    pub backup: BackupConfig,
    pub link: LinkConfig,
    pub ui: UiConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct OmnigraphConfig {
    pub server: String,
    pub graph: String,
}

impl Default for OmnigraphConfig {
    fn default() -> Self {
        Self {
            server: "http://127.0.0.1:8080".to_string(),
            graph: "zettelkasten".to_string(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct BackupConfig {
    pub repo: Option<PathBuf>,
    pub push: bool,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct LinkConfig {
    pub candidates: usize,
    pub min_similarity: f32,
    pub inbox_similarity: f32,
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            candidates: 5,
            min_similarity: 0.0,
            inbox_similarity: 0.0,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub locator: bool,
}

#[derive(Debug)]
pub enum ConfigError {
    Parse(toml::de::Error),
    Io(std::io::Error),
    NoHome,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Parse(e) => write!(f, "設定ファイルを読めません: {e}"),
            ConfigError::Io(e) => write!(f, "設定ファイルを開けません: {e}"),
            ConfigError::NoHome => write!(f, "ホームディレクトリが分かりません (HOME が未設定)"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    pub fn from_toml_str(s: &str, home: &Path) -> Result<Config, ConfigError> {
        let mut config: Config = toml::from_str(s).map_err(ConfigError::Parse)?;
        if let Some(repo) = &config.backup.repo {
            config.backup.repo = Some(expand_tilde(repo, home));
        }
        Ok(config)
    }

    /// `~/.config/zettelist/config.toml` を読む。ファイルがなければ既定値。
    #[allow(dead_code)] // 計画 2 で使う
    pub fn load() -> Result<Config, ConfigError> {
        let home = std::env::var_os("HOME").map(PathBuf::from).ok_or(ConfigError::NoHome)?;
        let path = home.join(".config/zettelist/config.toml");
        match std::fs::read_to_string(&path) {
            Ok(s) => Config::from_toml_str(&s, &home),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(ConfigError::Io(e)),
        }
    }
}

fn expand_tilde(path: &Path, home: &Path) -> PathBuf {
    match path.strip_prefix("~") {
        Ok(rest) => home.join(rest),
        Err(_) => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn empty_file_gives_defaults() {
        let c = Config::from_toml_str("", Path::new("/home/u")).unwrap();
        assert_eq!(c.omnigraph.graph, "zettelkasten");
        assert_eq!(c.omnigraph.server, "http://127.0.0.1:8080");
        assert!(!c.backup.push);
        assert_eq!(c.link.candidates, 5);
        assert!(!c.ui.locator);
    }

    #[test]
    fn partial_section_keeps_other_defaults() {
        let c = Config::from_toml_str("[link]\ncandidates = 3\n", Path::new("/home/u")).unwrap();
        assert_eq!(c.link.candidates, 3);
        assert_eq!(c.link.min_similarity, 0.0);
        assert_eq!(c.omnigraph.graph, "zettelkasten");
    }

    #[test]
    fn tilde_in_repo_expands_to_home() {
        let c = Config::from_toml_str("[backup]\nrepo = \"~/data\"\n", Path::new("/home/u")).unwrap();
        assert_eq!(c.backup.repo, Some(PathBuf::from("/home/u/data")));
    }

    #[test]
    fn unknown_sections_are_ignored() {
        assert!(Config::from_toml_str("[theme]\nx = \"red\"\n[keys]\ny = \"z\"\n", Path::new("/h")).is_ok());
    }

    #[test]
    fn broken_toml_reports_line() {
        let e = Config::from_toml_str("[link\n", Path::new("/h")).unwrap_err();
        assert!(e.to_string().contains("1"), "error: {e}");
    }
}
