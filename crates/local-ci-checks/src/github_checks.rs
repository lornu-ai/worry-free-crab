use crate::linter::{LintSeverity, LinterReport};
use crate::secrets::SecretsReport;
use crate::vulnerability::{Severity, VulnerabilityReport};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckAnnotation {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_column: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_column: Option<usize>,
    pub annotation_level: String, // "notice", "warning", "failure"
    pub message: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_details: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckOutput {
    pub title: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub annotations: Vec<CheckAnnotation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckRunPayload {
    pub name: String,
    pub head_sha: String,
    pub status: String, // "queued", "in_progress", "completed"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conclusion: Option<String>, // "success", "failure", "neutral", "cancelled", "timed_out", "action_required"
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<CheckOutput>,
}

impl CheckRunPayload {
    pub fn new(name: &str, head_sha: &str, status: &str, started_at: &str) -> Self {
        Self {
            name: name.to_string(),
            head_sha: head_sha.to_string(),
            status: status.to_string(),
            conclusion: None,
            started_at: started_at.to_string(),
            completed_at: None,
            output: None,
        }
    }

    pub fn complete(
        &mut self,
        conclusion: &str,
        completed_at: &str,
        title: &str,
        summary: &str,
        text: Option<&str>,
    ) {
        self.status = "completed".to_string();
        self.conclusion = Some(conclusion.to_string());
        self.completed_at = Some(completed_at.to_string());

        self.output = Some(CheckOutput {
            title: title.to_string(),
            summary: summary.to_string(),
            text: text.map(String::from),
            annotations: Vec::new(),
        });
    }

    /// Appends an annotation to the check run output
    pub fn add_annotation(&mut self, annotation: CheckAnnotation) {
        if let Some(ref mut out) = self.output {
            out.annotations.push(annotation);
        }
    }
}

/// Translate our VulnerabilityReport into standard GitHub CheckRun annotations
pub fn map_vulnerabilities_to_annotations(report: &VulnerabilityReport) -> Vec<CheckAnnotation> {
    let mut annotations = Vec::new();

    for vuln in &report.vulnerabilities {
        let level = match vuln.severity {
            Severity::Critical | Severity::High => "failure".to_string(),
            Severity::Medium => "warning".to_string(),
            Severity::Low => "notice".to_string(),
        };

        annotations.push(CheckAnnotation {
            path: vuln.file_path.clone(),
            start_line: vuln.line_number,
            end_line: vuln.line_number,
            start_column: None,
            end_column: None,
            annotation_level: level,
            message: format!(
                "CVE ID: {}\nDescription: {}\nRemediation: {}",
                vuln.cve_id, vuln.description, vuln.remediation
            ),
            title: format!("Vulnerability in package '{}'", vuln.package_name),
            raw_details: Some(format!("{:?}", vuln)),
        });
    }

    annotations
}

/// Translate our LinterReport into standard GitHub CheckRun annotations
pub fn map_linter_to_annotations(report: &LinterReport) -> Vec<CheckAnnotation> {
    let mut annotations = Vec::new();

    let path = if std::path::Path::new(".wfc-ci.toml").exists() {
        ".wfc-ci.toml".to_string()
    } else if std::path::Path::new(".local-ci.toml").exists() {
        ".local-ci.toml".to_string()
    } else {
        ".wfc-ci.toml".to_string()
    };

    for warn in &report.warnings {
        let level = match warn.severity {
            LintSeverity::Error => "failure".to_string(),
            LintSeverity::Warning => "warning".to_string(),
        };

        annotations.push(CheckAnnotation {
            path: path.clone(), // config path
            start_line: warn.line_number.unwrap_or(1),
            end_line: warn.line_number.unwrap_or(1),
            start_column: None,
            end_column: None,
            annotation_level: level,
            message: format!(
                "Rule: {}\nError: {}\nRemediation: {}",
                warn.rule_id, warn.message, warn.remediation
            ),
            title: format!("Linter Warning [{}]", warn.rule_id),
            raw_details: None,
        });
    }

    annotations
}

/// Translate our SecretsReport into standard GitHub CheckRun annotations
pub fn map_secrets_to_annotations(report: &SecretsReport) -> Vec<CheckAnnotation> {
    let mut annotations = Vec::new();

    for secret in &report.findings {
        annotations.push(CheckAnnotation {
            path: secret.file_path.clone(),
            start_line: secret.line_number,
            end_line: secret.line_number,
            start_column: None,
            end_column: None,
            annotation_level: "failure".to_string(), // exposed secrets are always failures
            message: format!(
                "Secret Type: {}\nPattern: {}\nDescription: {}\nNever commit credentials to origin VCS repositories.",
                secret.secret_type, secret.line_content, secret.description
            ),
            title: format!("Exposed Secret Detected [{}]", secret.secret_type),
            raw_details: None,
        });
    }

    annotations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vulnerability::Vulnerability;

    #[test]
    fn test_map_vulnerabilities_to_annotations() {
        let mut report = VulnerabilityReport::default();
        report.vulnerabilities.push(Vulnerability {
            package_name: "openssl".to_string(),
            current_version: "0.10.51".to_string(),
            severity: Severity::High,
            cve_id: "CVE-2023-5678".to_string(),
            description: "Test openssl issue".to_string(),
            remediation: "Upgrade".to_string(),
            file_path: "Cargo.lock".to_string(),
            line_number: 10,
        });

        let annotations = map_vulnerabilities_to_annotations(&report);
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].annotation_level, "failure");
        assert_eq!(annotations[0].path, "Cargo.lock");
        assert_eq!(annotations[0].start_line, 10);
    }
}
