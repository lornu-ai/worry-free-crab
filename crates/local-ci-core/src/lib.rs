use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RemoteSSHDefaults {
    #[serde(default, alias = "macos_user")]
    pub macos_user: String,
    #[serde(default, alias = "linux_spark_user")]
    pub linux_spark_user: String,
    #[serde(default, alias = "windows_user")]
    pub windows_user: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CacheConfig {
    #[serde(default)]
    pub skip_dirs: Vec<String>,
    #[serde(default)]
    pub include_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DepsConfig {
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub optional: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkspaceConfig {
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Profile {
    #[serde(default)]
    pub stages: Vec<String>,
    #[serde(default)]
    pub fail_fast: bool,
    #[serde(default)]
    pub no_cache: bool,
    #[serde(default)]
    pub json: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RemoteHost {
    pub host: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub session: String,
    #[serde(default)]
    pub remote_dir: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Stage {
    #[serde(default)]
    pub name: String,

    #[serde(alias = "cmd")]
    pub command: Option<Vec<String>>,

    #[serde(alias = "fix_cmd")]
    pub fix_command: Option<Vec<String>>,

    #[serde(default)]
    pub check: bool,

    #[serde(default)]
    pub timeout: i64, // seconds

    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(default, alias = "depends_on")]
    pub depends_on: Vec<String>,

    #[serde(default)]
    pub watch: Vec<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub ssh_defaults: RemoteSSHDefaults,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub stages: HashMap<String, Stage>,
    #[serde(default)]
    pub dependencies: DepsConfig,
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    #[serde(default)]
    pub profiles: HashMap<String, Profile>,
    #[serde(default)]
    pub hosts: HashMap<String, RemoteHost>,
}

impl RemoteSSHDefaults {
    pub fn with_defaults(&self) -> Self {
        let mut out = self.clone();
        if out.macos_user.trim().is_empty() {
            out.macos_user = "aivcs".to_string();
        }
        if out.linux_spark_user.trim().is_empty() {
            out.linux_spark_user = "aivcs2".to_string();
        }
        if out.windows_user.trim().is_empty() {
            out.windows_user = "aivcs".to_string();
        }
        out
    }
}

pub fn normalize_ssh_host(host: &str, platform: &str, defaults: &RemoteSSHDefaults) -> String {
    let host = host.trim();
    if host.is_empty() || host.contains('@') {
        return host.to_string();
    }
    let d = defaults.with_defaults();
    let user = match platform {
        "linux_spark" => &d.linux_spark_user,
        "windows" => &d.windows_user,
        _ => &d.macos_user,
    };
    format!("{}@{}", user, host)
}

impl RemoteHost {
    pub fn effective_platform(&self, preset_name: &str) -> String {
        if !self.platform.trim().is_empty() {
            return self.platform.clone();
        }
        match preset_name {
            "sparky" | "aivcs2" => "linux_spark".to_string(),
            "msi" => "windows".to_string(),
            _ => "macos".to_string(),
        }
    }
}

impl Config {
    pub fn normalize_remote_host(&self, preset_name: &str, h: &RemoteHost) -> RemoteHost {
        let mut out = h.clone();
        out.host = normalize_ssh_host(
            &h.host,
            &h.effective_platform(preset_name),
            &self.ssh_defaults,
        );
        out
    }

    pub fn get_timeout(&self, stage_name: &str) -> std::time::Duration {
        if let Some(stage) = self.stages.get(stage_name) {
            if stage.timeout > 0 {
                return std::time::Duration::from_secs(stage.timeout as u64);
            }
        }
        std::time::Duration::from_secs(30) // Default fallback
    }

    pub fn get_enabled_stages(&self) -> Vec<String> {
        // Define default order for common stages to ensure deterministic output
        let order = vec![
            "fmt", "check", "clippy", "test", "lint", "vet", "types", "build", "audit", "deny",
            "machete", "taplo",
        ];

        let mut enabled = Vec::new();
        // First add stages in predefined order if they exist and are enabled
        for name in &order {
            if let Some(stage) = self.stages.get(*name) {
                if stage.enabled {
                    enabled.push(name.to_string());
                }
            }
        }

        // Then add any remaining enabled stages not in the predefined order, sorted alphabetically
        let mut extra = Vec::new();
        for (name, stage) in &self.stages {
            if !order.contains(&name.as_str()) && stage.enabled {
                extra.push(name.clone());
            }
        }
        extra.sort();
        enabled.extend(extra);

        enabled
    }

    pub fn get_all_stages(&self) -> Vec<String> {
        let order = vec![
            "fmt", "check", "clippy", "test", "lint", "vet", "types", "build", "audit", "deny",
            "machete", "taplo",
        ];

        let mut all = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for name in &order {
            if self.stages.contains_key(*name) {
                all.push(name.to_string());
                seen.insert(name.to_string());
            }
        }

        let mut extra = Vec::new();
        for name in self.stages.keys() {
            if !seen.contains(name) {
                extra.push(name.clone());
            }
        }
        extra.sort();
        all.extend(extra);

        all
    }

    pub fn get_profile_stages(&self, profile: &Profile) -> Vec<String> {
        if profile.stages.is_empty() {
            return self.get_enabled_stages();
        }

        let order = vec![
            "fmt", "check", "clippy", "test", "lint", "vet", "types", "build", "audit", "deny",
            "machete", "taplo",
        ];

        let in_profile: std::collections::HashSet<_> = profile.stages.iter().cloned().collect();
        let mut result = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for name in &order {
            if in_profile.contains(*name) {
                result.push(name.to_string());
                seen.insert(name.to_string());
            }
        }

        for name in &profile.stages {
            if !seen.contains(name) {
                result.push(name.clone());
            }
        }

        result
    }
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: std::path::PathBuf,
    pub members: Vec<String>,
    pub excludes: Vec<String>,
    pub is_single: bool,
}

impl Workspace {
    pub fn is_excluded(&self, path: &str) -> bool {
        let path_sep = std::path::MAIN_SEPARATOR.to_string();
        for exclude in &self.excludes {
            if path == exclude {
                return true;
            }
            if path.starts_prefix_with_sep(exclude, &path_sep) {
                return true;
            }
        }
        false
    }

    pub fn get_included_members(&self) -> Vec<String> {
        let included: Vec<String> = self
            .members
            .iter()
            .filter(|m| !self.is_excluded(m))
            .cloned()
            .collect();
        if included.is_empty() {
            vec![".".to_string()]
        } else {
            included
        }
    }
}

trait StartsPrefixWithSep {
    fn starts_prefix_with_sep(&self, prefix: &str, sep: &str) -> bool;
}

impl StartsPrefixWithSep for str {
    fn starts_prefix_with_sep(&self, prefix: &str, sep: &str) -> bool {
        let full_prefix = format!("{}{}", prefix, sep);
        self.starts_with(&full_prefix)
    }
}

pub fn matches_patterns(filename: &str, patterns: &[String]) -> bool {
    for pattern in patterns {
        if pattern.starts_with("*.") {
            let ext = &pattern[1..];
            if filename.ends_with(ext) {
                return true;
            }
        } else if pattern == "*" || filename == pattern {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matches_patterns() {
        let patterns = vec![
            "*.rs".to_string(),
            "Cargo.toml".to_string(),
            "src".to_string(),
        ];
        assert!(matches_patterns("lib.rs", &patterns));
        assert!(matches_patterns("Cargo.toml", &patterns));
        assert!(!matches_patterns("lib.go", &patterns));
    }
}
