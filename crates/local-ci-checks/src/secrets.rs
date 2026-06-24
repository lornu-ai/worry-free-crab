use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretFinding {
    pub file_path: String,
    pub line_number: usize,
    pub line_content: String,
    pub entropy: f64,
    pub secret_type: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SecretsReport {
    pub findings: Vec<SecretFinding>,
    pub files_scanned: Vec<String>,
    pub total_secrets_found: usize,
}

/// Calculate Shannon entropy of a string token to measure randomness/information content
pub fn shannon_entropy(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    let mut counts = [0usize; 256];
    let mut len = 0usize;
    for &b in s.as_bytes() {
        counts[b as usize] += 1;
        len += 1;
    }
    let mut entropy = 0.0;
    for &count in counts.iter() {
        if count > 0 {
            let p = (count as f64) / (len as f64);
            entropy -= p * p.log2();
        }
    }
    entropy
}

/// Recursively scans files in the workspace for hardcoded secrets
pub fn scan_workspace_secrets<P: AsRef<Path>>(
    workspace_root: P,
    skip_dirs: &[String],
) -> SecretsReport {
    let mut report = SecretsReport::default();
    let root = workspace_root.as_ref();
    let mut default_skips = vec![
        ".git".to_string(),
        "target".to_string(),
        "node_modules".to_string(),
        ".local-ci-cache".to_string(),
        ".claude".to_string(),
    ];
    for d in skip_dirs {
        if !default_skips.contains(d) {
            default_skips.push(d.clone());
        }
    }

    let mut files_to_scan = Vec::new();
    if collect_files(root, &default_skips, &mut files_to_scan).is_err() {
        return report;
    }

    for file_path in files_to_scan {
        let relative_path = match file_path.strip_prefix(root) {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(_) => file_path.to_string_lossy().to_string(),
        };

        report.files_scanned.push(relative_path.clone());
        if let Ok(file) = File::open(&file_path) {
            scan_file_lines(BufReader::new(file), &relative_path, &mut report);
        }
    }

    report.total_secrets_found = report.findings.len();
    report
}

fn collect_files(
    dir: &Path,
    skip_dirs: &[String],
    files: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();

            if skip_dirs.contains(&file_name.to_string()) {
                continue;
            }

            if path.is_dir() {
                collect_files(&path, skip_dirs, files)?;
            } else if path.is_file() {
                // Only scan text-like files to avoid binary scanning noise
                let ext = path
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();
                let matches_extensions = [
                    "rs", "toml", "json", "yaml", "yml", "js", "ts", "py", "go", "sh", "bash",
                    "md", "txt", "conf", "env",
                ];
                if matches_extensions.contains(&ext.as_str())
                    || file_name.starts_with(".env")
                    || ext.is_empty()
                {
                    files.push(path);
                }
            }
        }
    }
    Ok(())
}

fn scan_file_lines<R: BufRead>(reader: R, relative_path: &str, report: &mut SecretsReport) {
    for (idx, line_res) in reader.lines().enumerate() {
        let line_num = idx + 1;
        let line = match line_res {
            Ok(l) => l,
            Err(_) => continue,
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // 1. Signature checks (known prefix rules)
        if trimmed.contains("-----BEGIN") && trimmed.contains("PRIVATE KEY-----") {
            report.findings.push(SecretFinding {
                file_path: relative_path.to_string(),
                line_number: line_num,
                line_content: "[REDACTED PRIVATE KEY HEADER]".to_string(),
                entropy: 0.0,
                secret_type: "Private Key".to_string(),
                description: "Asymmetric cryptographic private key block exposed.".to_string(),
            });
            continue;
        }

        struct SigRule {
            prefixes: &'static [&'static str],
            secret_type: &'static str,
            description: &'static str,
            length: usize,
        }

        let rules = [
            SigRule {
                prefixes: &["xoxb-", "xoxp-"],
                secret_type: "Slack Token",
                description: "Slack Bot or User API token found.",
                length: 30,
            },
            SigRule {
                prefixes: &["ghp_", "github_pat_"],
                secret_type: "GitHub PAT",
                description: "GitHub Personal Access Token found.",
                length: 40,
            },
            SigRule {
                prefixes: &["sk_live_", "rk_live_"],
                secret_type: "Stripe API Key",
                description: "Stripe Live API secret or restricted key found.",
                length: 24,
            },
            SigRule {
                prefixes: &["AIzaSy"],
                secret_type: "Google API Key",
                description: "Google Cloud Platform API key found.",
                length: 39,
            },
        ];

        let mut signature_matched = false;
        for rule in &rules {
            for prefix in rule.prefixes {
                if trimmed.contains(prefix) {
                    report.findings.push(SecretFinding {
                        file_path: relative_path.to_string(),
                        line_number: line_num,
                        line_content: redact_secret_string(trimmed, prefix, rule.length),
                        entropy: 0.0,
                        secret_type: rule.secret_type.to_string(),
                        description: rule.description.to_string(),
                    });
                    signature_matched = true;
                    break;
                }
            }
            if signature_matched {
                break;
            }
        }
        if signature_matched {
            continue;
        }

        // Heuristic checks for AWS access key (simple match since we don't have regex)
        if let Some(pos) = trimmed.find("AKIA") {
            if trimmed.len() >= pos + 20 {
                let chunk = &trimmed[pos..pos + 20];
                if chunk
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                {
                    report.findings.push(SecretFinding {
                        file_path: relative_path.to_string(),
                        line_number: line_num,
                        line_content: format!("AWS Key: {}...", &chunk[0..8]),
                        entropy: 0.0,
                        secret_type: "AWS Access Key ID".to_string(),
                        description: "AWS Cloud IAM Credential identifier found.".to_string(),
                    });
                    continue;
                }
            }
        }

        // 2. Entropy check on key value assignments
        // E.g. secret = "d87a3f3bce92a348" or passwd: "..."
        let lower = trimmed.to_lowercase();
        let is_assignment = lower.contains("api_key")
            || lower.contains("secret")
            || lower.contains("passwd")
            || lower.contains("password")
            || lower.contains("token")
            || lower.contains("credential")
            || lower.contains("private_key")
            || lower.contains("api_token");

        if is_assignment {
            if let Some(pos) = trimmed.find('=') {
                let value_part = &trimmed[pos + 1..].trim();
                let clean_val = value_part
                    .trim_matches('"')
                    .trim_matches('\'')
                    .trim_matches(';');
                if clean_val.len() >= 16 && !clean_val.contains("$") && !clean_val.contains("{") {
                    let entropy = shannon_entropy(clean_val);
                    if entropy >= 4.3 {
                        // High entropy string assignment
                        report.findings.push(SecretFinding {
                            file_path: relative_path.to_string(),
                            line_number: line_num,
                            line_content: redact_value(trimmed, clean_val),
                            entropy,
                            secret_type: "High Entropy Secret".to_string(),
                            description: format!("Heuristic credentials check detected high-entropy value (S={:.2}).", entropy),
                        });
                    }
                }
            }
        }
    }
}

fn redact_secret_string(line: &str, pattern: &str, length: usize) -> String {
    if let Some(pos) = line.find(pattern) {
        let end_idx = std::cmp::min(line.len(), pos + length);
        let secret = &line[pos..end_idx];
        let prefix_len = std::cmp::min(secret.len(), 8);
        let replacement = if secret.len() <= 8 {
            if secret.is_empty() {
                "****".to_string()
            } else {
                format!("{}****", &secret[0..1])
            }
        } else {
            format!("{}...", &secret[0..prefix_len])
        };
        line.replace(secret, &replacement)
    } else {
        "[REDACTED]".to_string()
    }
}

fn redact_value(line: &str, value: &str) -> String {
    let replacement = if value.len() <= 6 {
        if value.is_empty() {
            "****".to_string()
        } else {
            format!("{}****", &value[0..1])
        }
    } else {
        format!("{}...", &value[0..6])
    };
    line.replace(value, &replacement)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_shannon_entropy() {
        let low_entropy = "aaaaabbbbbccccc";
        let high_entropy = "4g9Hs2K8lQzP9xW2";
        assert!(shannon_entropy(high_entropy) > shannon_entropy(low_entropy));
    }

    #[test]
    fn test_scan_file_lines_secrets() {
        let data = "
const API_KEY = \"a9Fj28Hsl9WxzP9K12mK89\"; // test secret
let password = \"not_random\";
let token = \"xoxb-1234567890-abcdefgh\";
";
        let mut report = SecretsReport::default();
        scan_file_lines(Cursor::new(data), "index.js", &mut report);
        assert_eq!(report.findings.len(), 2);
        assert_eq!(report.findings[0].secret_type, "High Entropy Secret");
        assert_eq!(report.findings[1].secret_type, "Slack Token");
    }

    #[test]
    fn test_redact_short_secrets() {
        assert_eq!(
            redact_value("secret = \"abc\"", "abc"),
            "secret = \"a****\""
        );
        assert_eq!(
            redact_value("secret = \"abcdefg\"", "abcdefg"),
            "secret = \"abcdef...\""
        );
        assert_eq!(
            redact_secret_string("token = \"xoxb-1\"", "xoxb-", 10),
            "token = \"x****"
        );
    }
}
