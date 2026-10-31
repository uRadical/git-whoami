//! Read-only view of the git-persona config (`~/.config/git-persona/config.json`).
//!
//! The structs mirror git-persona's own config format; git-whoami never writes it.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Persona {
    pub name: String,
    pub user_name: String,
    pub email: String,
    #[serde(default)]
    pub ssh_key: Option<String>,
    #[serde(default)]
    pub gpg_key: Option<String>,
    #[serde(default = "default_color")]
    pub color: String,
}

fn default_color() -> String {
    "white".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct Location {
    pub path: PathBuf,
    pub persona_name: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub personas: HashMap<String, Persona>,
    #[serde(default)]
    pub locations: Vec<Location>,
}

impl Config {
    /// Load config from disk, or return empty config if git-persona isn't set up
    pub fn load() -> Result<Self> {
        let path = config_path()?;

        if !path.exists() {
            return Ok(Config::default());
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config from {}", path.display()))?;

        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))
    }

    /// The persona git-persona has applied to `path` (most specific location wins)
    pub fn persona_for_path(&self, path: &Path) -> Option<(&Location, &Persona)> {
        let abs_path = path.canonicalize().ok()?;

        self.locations
            .iter()
            .filter_map(|loc| {
                let canonical = loc.path.canonicalize().ok()?;
                abs_path
                    .starts_with(&canonical)
                    .then(|| (canonical.components().count(), loc))
            })
            .max_by_key(|(depth, _)| *depth)
            .and_then(|(_, loc)| Some((loc, self.personas.get(&loc.persona_name)?)))
    }

    /// Personas whose identity matches the given name and email
    pub fn personas_matching(&self, name: &str, email: &str) -> Vec<&Persona> {
        let mut matches: Vec<&Persona> = self
            .personas
            .values()
            .filter(|p| p.user_name == name && p.email.eq_ignore_ascii_case(email))
            .collect();
        matches.sort_by(|a, b| a.name.cmp(&b.name));
        matches
    }
}

/// Get the git-persona config file path
pub fn config_path() -> Result<PathBuf> {
    let config_dir = if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else {
        dirs::home_dir()
            .context("Could not determine home directory")?
            .join(".config")
    };

    Ok(config_dir.join("git-persona").join("config.json"))
}

/// Replace the home directory prefix with ~ for display
pub fn display_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(stripped) = path.strip_prefix(&home)
    {
        return format!("~/{}", stripped.display());
    }
    path.display().to_string()
}

/// Expand a leading ~/ to the home directory
pub fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest).display().to_string();
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn persona(name: &str, email: &str) -> Persona {
        Persona {
            name: name.to_string(),
            user_name: "Alan".to_string(),
            email: email.to_string(),
            ssh_key: None,
            gpg_key: None,
            color: "blue".to_string(),
        }
    }

    #[test]
    fn parses_git_persona_config() {
        let json = r#"{
            "personas": {
                "work": {"name": "work", "user_name": "Alan", "email": "alan@work.com",
                         "ssh_key": "~/.ssh/work", "color": "blue"}
            },
            "locations": [{"path": "/tmp/work", "persona_name": "work"}]
        }"#;
        let config: Config = serde_json::from_str(json).unwrap();
        let work = &config.personas["work"];
        assert_eq!(work.ssh_key.as_deref(), Some("~/.ssh/work"));
        assert_eq!(work.gpg_key, None);
        assert_eq!(config.locations.len(), 1);
    }

    #[test]
    fn most_specific_location_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let outer = tmp.path().join("code");
        let inner = outer.join("client");
        let deep = inner.join("repo/src");
        fs::create_dir_all(&deep).unwrap();

        let mut config = Config::default();
        config
            .personas
            .insert("work".into(), persona("work", "a@work.com"));
        config
            .personas
            .insert("client".into(), persona("client", "a@client.com"));
        config.locations.push(Location {
            path: outer.clone(),
            persona_name: "work".into(),
        });
        config.locations.push(Location {
            path: inner,
            persona_name: "client".into(),
        });

        let (_, p) = config.persona_for_path(&deep).unwrap();
        assert_eq!(p.name, "client");
        let (_, p) = config.persona_for_path(&outer).unwrap();
        assert_eq!(p.name, "work");
        assert!(config.persona_for_path(tmp.path()).is_none());
    }

    #[test]
    fn matching_ignores_email_case() {
        let mut config = Config::default();
        config
            .personas
            .insert("work".into(), persona("work", "Alan@Work.com"));
        assert_eq!(config.personas_matching("Alan", "alan@work.com").len(), 1);
        assert!(config.personas_matching("Bob", "alan@work.com").is_empty());
    }
}
