use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::rule::Severity;

/// Configuration loaded from `elm-assist.toml`.
#[derive(Debug, Default, Deserialize)]
pub struct Config {
    /// Source directory override (default: "src").
    pub src: Option<String>,
    /// Rule configuration.
    #[serde(default)]
    pub rules: RulesConfig,
}

/// Rule-level configuration.
#[derive(Debug, Default, Deserialize)]
pub struct RulesConfig {
    /// Rules to disable entirely.
    #[serde(default)]
    pub disable: Vec<String>,
    /// Per-rule severity overrides.
    #[serde(default)]
    pub severity: HashMap<String, SeverityValue>,
}

/// A severity value in the config file. `Off` disables the rule.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SeverityValue {
    Error,
    Warning,
    Off,
}

/// Errors that can occur when loading a config file.
#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(toml::de::Error),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "could not read config file: {e}"),
            ConfigError::Parse(e) => write!(f, "could not parse config file: {e}"),
        }
    }
}

impl Config {
    /// Load a config from a specific file path.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path).map_err(ConfigError::Io)?;
        toml::from_str(&contents).map_err(ConfigError::Parse)
    }

    /// Walk up from the current directory looking for `elm-assist.toml`.
    pub fn discover() -> Option<(PathBuf, Self)> {
        let mut dir = std::env::current_dir().ok()?;
        loop {
            let candidate = dir.join("elm-assist.toml");
            if candidate.exists() {
                let config = Self::load(&candidate).ok()?;
                return Some((candidate, config));
            }
            if !dir.pop() {
                return None;
            }
        }
    }

    /// Check if a rule is disabled (by the `disable` list or `severity = "off"`).
    pub fn is_rule_disabled(&self, name: &str) -> bool {
        if self.rules.disable.iter().any(|n| n == name) {
            return true;
        }
        matches!(
            self.rules.severity.get(name),
            Some(SeverityValue::Off)
        )
    }

    /// Get the configured severity for a rule, if any.
    pub fn severity_for(&self, name: &str) -> Option<Severity> {
        match self.rules.severity.get(name)? {
            SeverityValue::Error => Some(Severity::Error),
            SeverityValue::Warning => Some(Severity::Warning),
            SeverityValue::Off => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let config: Config = toml::from_str("").unwrap();
        assert!(config.src.is_none());
        assert!(config.rules.disable.is_empty());
        assert!(config.rules.severity.is_empty());
    }

    #[test]
    fn parse_full_config() {
        let config: Config = toml::from_str(
            r#"
src = "lib"

[rules]
disable = ["NoTodoComment", "NoMissingTypeAnnotation"]

[rules.severity]
NoDebug = "error"
NoUnusedImports = "warning"
NoAlwaysIdentity = "off"
"#,
        )
        .unwrap();

        assert_eq!(config.src.as_deref(), Some("lib"));
        assert_eq!(config.rules.disable.len(), 2);
        assert!(config.is_rule_disabled("NoTodoComment"));
        assert!(config.is_rule_disabled("NoMissingTypeAnnotation"));
        assert!(!config.is_rule_disabled("NoDebug"));
        // "off" in severity also disables.
        assert!(config.is_rule_disabled("NoAlwaysIdentity"));

        assert_eq!(config.severity_for("NoDebug"), Some(Severity::Error));
        assert_eq!(config.severity_for("NoUnusedImports"), Some(Severity::Warning));
        assert_eq!(config.severity_for("NoAlwaysIdentity"), None);
        assert_eq!(config.severity_for("UnknownRule"), None);
    }

    #[test]
    fn default_config_disables_nothing() {
        let config = Config::default();
        assert!(!config.is_rule_disabled("NoDebug"));
        assert_eq!(config.severity_for("NoDebug"), None);
    }
}
