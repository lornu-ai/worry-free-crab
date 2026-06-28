use local_ci_core::Config;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum LintSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintWarning {
    pub rule_id: String,
    pub message: String,
    pub severity: LintSeverity,
    pub line_number: Option<usize>,
    pub remediation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LinterReport {
    pub warnings: Vec<LintWarning>,
    pub has_errors: bool,
}

pub fn lint_config_in_workspace<P: AsRef<Path>>(workspace_root: P) -> LinterReport {
    let mut report = LinterReport::default();
    let root = workspace_root.as_ref();
    let wfc_config_path = root.join(".wfc-ci.toml");
    let local_config_path = root.join(".local-ci.toml");
    let (config_path, is_deprecated) = if wfc_config_path.exists() {
        (wfc_config_path, false)
    } else {
        (local_config_path, true)
    };

    if !config_path.exists() {
        report.warnings.push(LintWarning {
            rule_id: "LC_CFG_MISSING".to_string(),
            message: "Configuration file (.wfc-ci.toml) does not exist in workspace root.".to_string(),
            severity: LintSeverity::Error,
            line_number: None,
            remediation: "Run `local-ci init` to generate a default configuration file."
                .to_string(),
        });
        report.has_errors = true;
        return report;
    }

    if is_deprecated {
        report.warnings.push(LintWarning {
            rule_id: "LC_CFG_DEPRECATED".to_string(),
            message: ".local-ci.toml is deprecated. Please rename it to .wfc-ci.toml.".to_string(),
            severity: LintSeverity::Warning,
            line_number: None,
            remediation: "Rename .local-ci.toml to .wfc-ci.toml.".to_string(),
        });
    }

    let file_name = config_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".wfc-ci.toml");

    let content = match fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            report.warnings.push(LintWarning {
                rule_id: "LC_CFG_READ_FAIL".to_string(),
                message: format!("Failed to read {}: {}", file_name, e),
                severity: LintSeverity::Error,
                line_number: None,
                remediation: "Verify file read permissions and encoding.".to_string(),
            });
            report.has_errors = true;
            return report;
        }
    };

    // 1. Lint the raw string content for simple pattern/line rules
    lint_raw_content(&content, &mut report);

    // 2. Parsed structure linting
    let config: Config = match toml::from_str(&content) {
        Ok(cfg) => cfg,
        Err(e) => {
            report.warnings.push(LintWarning {
                rule_id: "LC_CFG_PARSE_FAIL".to_string(),
                message: format!("Failed to parse {} as valid TOML: {}", file_name, e),
                severity: LintSeverity::Error,
                line_number: None,
                remediation: "Verify TOML structure matches the reference specification."
                    .to_string(),
            });
            report.has_errors = true;
            return report;
        }
    };

    lint_parsed_struct(&config, &mut report);

    report
}

fn lint_raw_content(content: &str, report: &mut LinterReport) {
    let mut has_cache_header = false;
    for (idx, line) in content.lines().enumerate() {
        let line_num = idx + 1;
        let trimmed = line.trim();

        if trimmed == "[cache]" {
            has_cache_header = true;
        }

        // Check for raw use of shell-pipe commands which might bypass caching or safety
        if trimmed.contains("curl") && trimmed.contains("|") && trimmed.contains("bash") {
            report.warnings.push(LintWarning {
                rule_id: "LC_SEC_PIPE_BASH".to_string(),
                message: "Found unsafe curl | bash pipe pattern in command string.".to_string(),
                severity: LintSeverity::Warning,
                line_number: Some(line_num),
                remediation: "Avoid downloading and executing arbitrary scripts. Bundle scripts or use a secure package manager.".to_string(),
            });
        }

        // Check for hardcoded credentials
        if (trimmed.contains("password") || trimmed.contains("secret") || trimmed.contains("token"))
            && trimmed.contains("=")
            && !trimmed.starts_with("#")
        {
            let parts: Vec<&str> = trimmed.split('=').collect();
            if parts.len() > 1 {
                let val = parts[1].trim().trim_matches('"').trim_matches('\'');
                if val.len() > 8 && !val.contains("$") && !val.contains("{") {
                    report.warnings.push(LintWarning {
                        rule_id: "LC_SEC_HARDCODED_AUTH".to_string(),
                        message: "Potential hardcoded credential or secret found in config line.".to_string(),
                        severity: LintSeverity::Error,
                        line_number: Some(line_num),
                        remediation: "Inject credentials as environment variables or load them via standard runner auth contexts instead of hardcoding.".to_string(),
                    });
                    report.has_errors = true;
                }
            }
        }
    }

    if !has_cache_header {
        report.warnings.push(LintWarning {
            rule_id: "LC_CFG_NO_CACHE_SECTION".to_string(),
            message: "No [cache] header or configuration section was defined.".to_string(),
            severity: LintSeverity::Warning,
            line_number: None,
            remediation: "Add a [cache] block with 'skip_dirs' and 'include_patterns' to enable build speeds optimization.".to_string(),
        });
    }
}

fn lint_parsed_struct(config: &Config, report: &mut LinterReport) {
    // Check Cache skip_dirs contains common defaults
    let skip_dirs = &config.cache.skip_dirs;
    let expected_skips = &[".git", "target", "node_modules"];
    for expected in expected_skips {
        if !skip_dirs.contains(&expected.to_string()) {
            report.warnings.push(LintWarning {
                rule_id: "LC_CFG_MISSING_CACHE_SKIP".to_string(),
                message: format!("Directory '{}' is not skipped under [cache.skip_dirs].", expected),
                severity: LintSeverity::Warning,
                line_number: None,
                remediation: format!("Add '{}' to [cache.skip_dirs] to avoid scanning extremely large or irrelevant directory contents.", expected),
            });
        }
    }

    // Check Stages sanity
    for (name, stage) in &config.stages {
        // Stage names validation
        if name.len() < 2 {
            report.warnings.push(LintWarning {
                rule_id: "LC_STAGE_NAME_SHORT".to_string(),
                message: format!("Stage name '{}' is too short.", name),
                severity: LintSeverity::Warning,
                line_number: None,
                remediation: "Choose descriptive stage names of at least 2 characters.".to_string(),
            });
        }

        // Commands validation
        if stage.enabled {
            match &stage.command {
                None => {
                    report.warnings.push(LintWarning {
                        rule_id: "LC_STAGE_MISSING_CMD".to_string(),
                        message: format!("Enabled stage '{}' does not define a 'command'.", name),
                        severity: LintSeverity::Error,
                        line_number: None,
                        remediation: "Define an executable command array or disable the stage."
                            .to_string(),
                    });
                    report.has_errors = true;
                }
                Some(cmd) if cmd.is_empty() => {
                    report.warnings.push(LintWarning {
                        rule_id: "LC_STAGE_EMPTY_CMD".to_string(),
                        message: format!("Enabled stage '{}' has an empty 'command' list.", name),
                        severity: LintSeverity::Error,
                        line_number: None,
                        remediation: "Add executable program arguments to the stage command list."
                            .to_string(),
                    });
                    report.has_errors = true;
                }
                _ => {}
            }
        }

        // Timeouts check
        if stage.timeout <= 0 {
            report.warnings.push(LintWarning {
                rule_id: "LC_STAGE_TIMEOUT_ZERO".to_string(),
                message: format!("Stage '{}' timeout is less than or equal to 0.", name),
                severity: LintSeverity::Warning,
                line_number: None,
                remediation: "Configure a positive timeout integer (in seconds) to prevent infinite process locks.".to_string(),
            });
        } else if stage.timeout > 3600 {
            report.warnings.push(LintWarning {
                rule_id: "LC_STAGE_TIMEOUT_EXCESSIVE".to_string(),
                message: format!(
                    "Stage '{}' timeout is extremely long ({} seconds / over 1 hour).",
                    name, stage.timeout
                ),
                severity: LintSeverity::Warning,
                line_number: None,
                remediation:
                    "Keep local timeouts short (under 600s) to keep iteration cycles snappy."
                        .to_string(),
            });
        }

        // Validate dependencies exist in config
        for dep in &stage.depends_on {
            if !config.stages.contains_key(dep) {
                report.warnings.push(LintWarning {
                    rule_id: "LC_STAGE_UNKNOWN_DEP".to_string(),
                    message: format!("Stage '{}' depends on unknown stage '{}'.", name, dep),
                    severity: LintSeverity::Error,
                    line_number: None,
                    remediation: format!("Create a stage configuration for '{}' or remove it from stage '{}' depends_on lists.", dep, name),
                });
                report.has_errors = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use local_ci_core::{CacheConfig, Stage};
    use std::collections::HashMap;

    #[test]
    fn test_lint_parsed_struct_errors() {
        let mut stages = HashMap::new();
        stages.insert(
            "t".to_string(),
            Stage {
                name: "t".to_string(),
                command: Some(vec![]),
                fix_command: None,
                check: false,
                timeout: -5,
                enabled: true,
                depends_on: vec!["missing_stage".to_string()],
                watch: vec![],
            },
        );

        let config = Config {
            ssh_defaults: Default::default(),
            cache: CacheConfig {
                skip_dirs: vec![],
                include_patterns: vec![],
            },
            stages,
            dependencies: Default::default(),
            workspace: Default::default(),
            profiles: Default::default(),
            hosts: Default::default(),
        };

        let mut report = LinterReport::default();
        lint_parsed_struct(&config, &mut report);

        assert!(report.has_errors);
        let rule_ids: Vec<String> = report.warnings.iter().map(|w| w.rule_id.clone()).collect();
        assert!(rule_ids.contains(&"LC_CFG_MISSING_CACHE_SKIP".to_string()));
        assert!(rule_ids.contains(&"LC_STAGE_NAME_SHORT".to_string()));
        assert!(rule_ids.contains(&"LC_STAGE_EMPTY_CMD".to_string()));
        assert!(rule_ids.contains(&"LC_STAGE_TIMEOUT_ZERO".to_string()));
        assert!(rule_ids.contains(&"LC_STAGE_UNKNOWN_DEP".to_string()));
    }
}
