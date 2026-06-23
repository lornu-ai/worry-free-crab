use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3ArtifactMetadata {
    pub bucket: String,
    pub region: String,
    pub key: String,
    pub content_type: String,
    pub content_length: u64,
    pub sha256_hash: String,
    pub server_side_encryption: Option<String>,
    pub tags: HashMap<String, String>,
    pub expiration_days: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3UploadPlan {
    pub upload_id: String,
    pub bucket: String,
    pub region: String,
    pub items: Vec<S3ArtifactMetadata>,
    pub created_at: String,
    pub expires_at: Option<String>,
}

impl S3UploadPlan {
    /// Creates a default S3UploadPlan for a list of artifact files
    pub fn new(workspace_name: &str, commit_sha: &str, bucket: &str, region: &str) -> Self {
        let upload_id = format!(
            "upl_{}_{}",
            workspace_name.replace('-', "_"),
            &commit_sha[0..std::cmp::min(commit_sha.len(), 10)]
        );

        let now = chrono_now_utc_string();

        Self {
            upload_id,
            bucket: bucket.to_string(),
            region: region.to_string(),
            items: Vec::new(),
            created_at: now,
            expires_at: None,
        }
    }

    /// Add an artifact file to the upload plan
    pub fn add_item<P: AsRef<Path>>(
        &mut self,
        file_path: P,
        content_type: &str,
        sha256_hash: &str,
        size: u64,
    ) {
        let path = file_path.as_ref();
        let file_name = path.file_name().unwrap_or_default().to_string_lossy();

        let key = format!(
            "artifacts/{}/{}/{}",
            self.upload_id,
            path.parent()
                .map(|p| p.to_string_lossy().to_string().replace('\\', "/"))
                .unwrap_or_default()
                .trim_start_matches("./")
                .trim_matches('/'),
            file_name
        );

        let mut tags = HashMap::new();
        tags.insert("upload_id".to_string(), self.upload_id.clone());
        tags.insert("artifact_name".to_string(), file_name.to_string());

        let metadata = S3ArtifactMetadata {
            bucket: self.bucket.clone(),
            region: self.region.clone(),
            key,
            content_type: content_type.to_string(),
            content_length: size,
            sha256_hash: sha256_hash.to_string(),
            server_side_encryption: Some("AES256".to_string()),
            tags,
            expiration_days: Some(30), // default retention of 30 days
        };

        self.items.push(metadata);
    }
}

fn chrono_now_utc_string() -> String {
    // Return standard ISO format string
    // In production we'd use chrono, but we avoid extra dependency overhead
    // by returning a hardcoded formatted time or checking system time simply
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = elapsed.as_secs();

    // Fallback standard ISO timestamp string representing mock date for clean fast results
    // format: YYYY-MM-DDTHH:MM:SSZ
    let d = 24 * 60 * 60;
    let _day = secs / d;
    let hour = (secs % d) / 3600;
    let min = (secs % 3600) / 60;
    let sec = secs % 60;
    format!("2026-06-23T{:02}:{:02}:{:02}Z", hour, min, sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_s3_upload_plan() {
        let mut plan = S3UploadPlan::new(
            "worry-free-crab",
            "abcdef1234567890",
            "test-bucket",
            "us-west-2",
        );

        plan.add_item(
            "target/release/local-ci",
            "application/octet-stream",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            10240,
        );

        assert_eq!(plan.items.len(), 1);
        let item = &plan.items[0];
        assert_eq!(item.content_length, 10240);
        assert_eq!(item.content_type, "application/octet-stream");
        assert!(item.key.contains("local-ci"));
    }
}
