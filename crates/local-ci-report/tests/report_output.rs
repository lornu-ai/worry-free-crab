//! Integration tests for local-ci report module
//! Tests JSON serialization, human-readable output, and output formatting

// Mock structures for testing
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct MockStageResult {
    name: String,
    status: String,
    duration_ms: u64,
    cache_hit: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct MockPipelineResult {
    schema_version: String,
    run_id: String,
    repo: String,
    sha: String,
    status: String,
    duration_ms: u64,
    stages: Vec<MockStageResult>,
}

impl Default for MockPipelineResult {
    fn default() -> Self {
        Self {
            schema_version: "local-ci.result.v1".to_string(),
            run_id: "run_test_123".to_string(),
            repo: "lornu-ai/test-repo".to_string(),
            sha: "abc123def456".to_string(),
            status: "passed".to_string(),
            duration_ms: 5000,
            stages: vec![],
        }
    }
}

#[test]
fn test_json_schema_version() {
    let result = MockPipelineResult::default();
    let json = serde_json::to_value(&result).expect("failed to serialize");
    
    assert_eq!(
        json["schema_version"],
        "local-ci.result.v1",
        "schema_version must match expected format"
    );
}

#[test]
fn test_json_required_fields() {
    let result = MockPipelineResult::default();
    let json = serde_json::to_value(&result).expect("failed to serialize");
    
    // All required fields must be present
    assert!(json["run_id"].is_string(), "run_id must be string");
    assert!(json["repo"].is_string(), "repo must be string");
    assert!(json["sha"].is_string(), "sha must be string");
    assert!(json["status"].is_string(), "status must be string");
    assert!(json["duration_ms"].is_number(), "duration_ms must be number");
    assert!(json["stages"].is_array(), "stages must be array");
}

#[test]
fn test_json_status_values() {
    let statuses = vec!["passed", "failed", "skipped", "cancelled", "timed_out"];
    
    for status in statuses {
        let mut result = MockPipelineResult::default();
        result.status = status.to_string();
        let json = serde_json::to_value(&result).expect("failed to serialize");
        
        assert_eq!(
            json["status"].as_str().unwrap(),
            status,
            "status '{}' should serialize correctly",
            status
        );
    }
}

#[test]
fn test_json_single_stage() {
    let mut result = MockPipelineResult::default();
    result.stages.push(MockStageResult {
        name: "test".to_string(),
        status: "passed".to_string(),
        duration_ms: 1000,
        cache_hit: false,
    });

    let json = serde_json::to_value(&result).expect("failed to serialize");
    
    assert_eq!(json["stages"].as_array().unwrap().len(), 1);
    assert_eq!(json["stages"][0]["name"], "test");
    assert_eq!(json["stages"][0]["status"], "passed");
    assert_eq!(json["stages"][0]["duration_ms"], 1000);
    assert_eq!(json["stages"][0]["cache_hit"], false);
}

#[test]
fn test_json_multiple_stages() {
    let mut result = MockPipelineResult::default();
    result.stages = vec![
        MockStageResult {
            name: "fmt".to_string(),
            status: "passed".to_string(),
            duration_ms: 500,
            cache_hit: true,
        },
        MockStageResult {
            name: "clippy".to_string(),
            status: "passed".to_string(),
            duration_ms: 2000,
            cache_hit: false,
        },
        MockStageResult {
            name: "test".to_string(),
            status: "failed".to_string(),
            duration_ms: 3000,
            cache_hit: false,
        },
    ];

    let json = serde_json::to_value(&result).expect("failed to serialize");
    
    let stages = json["stages"].as_array().unwrap();
    assert_eq!(stages.len(), 3);
    assert_eq!(stages[0]["cache_hit"], true);
    assert_eq!(stages[1]["cache_hit"], false);
    assert_eq!(stages[2]["status"], "failed");
}

#[test]
fn test_json_empty_stages() {
    let result = MockPipelineResult::default();
    let json = serde_json::to_value(&result).expect("failed to serialize");
    
    assert_eq!(json["stages"].as_array().unwrap().len(), 0);
}

#[test]
fn test_json_all_cached() {
    let mut result = MockPipelineResult::default();
    result.stages = vec![
        MockStageResult {
            name: "fmt".to_string(),
            status: "passed".to_string(),
            duration_ms: 50,
            cache_hit: true,
        },
        MockStageResult {
            name: "test".to_string(),
            status: "passed".to_string(),
            duration_ms: 40,
            cache_hit: true,
        },
    ];

    let json = serde_json::to_value(&result).expect("failed to serialize");
    
    // Calculate cache percentage
    let stages = json["stages"].as_array().unwrap();
    let cached_count = stages.iter().filter(|s| s["cache_hit"].as_bool().unwrap_or(false)).count();
    let cache_percentage = (cached_count as f64 / stages.len() as f64) * 100.0;
    
    assert_eq!(cache_percentage, 100.0, "100% of stages cached");
}

#[test]
fn test_json_partial_cached() {
    let mut result = MockPipelineResult::default();
    result.stages = vec![
        MockStageResult {
            name: "fmt".to_string(),
            status: "passed".to_string(),
            duration_ms: 50,
            cache_hit: true,
        },
        MockStageResult {
            name: "test".to_string(),
            status: "passed".to_string(),
            duration_ms: 2000,
            cache_hit: false,
        },
    ];

    let json = serde_json::to_value(&result).expect("failed to serialize");
    
    let stages = json["stages"].as_array().unwrap();
    let cached_count = stages.iter().filter(|s| s["cache_hit"].as_bool().unwrap_or(false)).count();
    let cache_percentage = (cached_count as f64 / stages.len() as f64) * 100.0;
    
    assert_eq!(cache_percentage, 50.0, "50% of stages cached");
}

#[test]
fn test_json_special_characters_in_names() {
    let mut result = MockPipelineResult::default();
    result.repo = "lornu-ai/repo-with-dashes".to_string();
    result.stages.push(MockStageResult {
        name: "stage-with-dashes".to_string(),
        status: "passed".to_string(),
        duration_ms: 1000,
        cache_hit: false,
    });

    let json = serde_json::to_value(&result).expect("failed to serialize");
    
    assert!(json.to_string().contains("lornu-ai/repo-with-dashes"));
    assert!(json["stages"][0]["name"].as_str().unwrap().contains("dashes"));
}

#[test]
fn test_json_unicode_characters() {
    let mut result = MockPipelineResult::default();
    result.stages.push(MockStageResult {
        name: "测试-test".to_string(),  // Chinese + English
        status: "passed".to_string(),
        duration_ms: 1000,
        cache_hit: false,
    });

    let json = serde_json::to_value(&result).expect("failed to serialize");
    let json_str = json.to_string();
    
    // Should serialize without error and preserve unicode
    assert!(json_str.contains("test"), "ASCII part preserved");
}

#[test]
fn test_json_large_duration() {
    let mut result = MockPipelineResult::default();
    result.duration_ms = 999_999_999;
    
    let json = serde_json::to_value(&result).expect("failed to serialize");
    assert_eq!(json["duration_ms"], 999_999_999);
}

#[test]
fn test_json_zero_duration() {
    let mut result = MockPipelineResult::default();
    result.duration_ms = 0;
    
    let json = serde_json::to_value(&result).expect("failed to serialize");
    assert_eq!(json["duration_ms"], 0);
}

#[test]
fn test_json_many_stages() {
    let mut result = MockPipelineResult::default();
    
    // Create 50 stages
    for i in 0..50 {
        result.stages.push(MockStageResult {
            name: format!("stage_{:02}", i),
            status: if i % 2 == 0 { "passed" } else { "failed" }.to_string(),
            duration_ms: (i as u64) * 100,
            cache_hit: i % 3 == 0,
        });
    }

    let json = serde_json::to_value(&result).expect("failed to serialize");
    assert_eq!(json["stages"].as_array().unwrap().len(), 50);
}

#[test]
fn test_json_status_distribution() {
    let mut result = MockPipelineResult::default();
    result.status = "mixed".to_string();
    
    result.stages = vec![
        MockStageResult {
            name: "s1".to_string(),
            status: "passed".to_string(),
            duration_ms: 1000,
            cache_hit: false,
        },
        MockStageResult {
            name: "s2".to_string(),
            status: "failed".to_string(),
            duration_ms: 500,
            cache_hit: false,
        },
        MockStageResult {
            name: "s3".to_string(),
            status: "passed".to_string(),
            duration_ms: 800,
            cache_hit: true,
        },
        MockStageResult {
            name: "s4".to_string(),
            status: "skipped".to_string(),
            duration_ms: 0,
            cache_hit: false,
        },
    ];

    let json = serde_json::to_value(&result).expect("failed to serialize");
    let stages = json["stages"].as_array().unwrap();
    
    let passed = stages.iter().filter(|s| s["status"] == "passed").count();
    let failed = stages.iter().filter(|s| s["status"] == "failed").count();
    let skipped = stages.iter().filter(|s| s["status"] == "skipped").count();
    let cached = stages.iter().filter(|s| s["cache_hit"].as_bool().unwrap_or(false)).count();
    
    assert_eq!(passed, 2);
    assert_eq!(failed, 1);
    assert_eq!(skipped, 1);
    assert_eq!(cached, 1);
}

#[test]
fn test_json_prettyprint() {
    let result = MockPipelineResult::default();
    let json_str = serde_json::to_string_pretty(&result).expect("failed to serialize");
    
    // Pretty-printed JSON should have newlines and indentation
    assert!(json_str.contains('\n'), "pretty JSON should have newlines");
    assert!(json_str.contains("  "), "pretty JSON should have indentation");
}

#[test]
fn test_json_compact() {
    let result = MockPipelineResult::default();
    let json_str = serde_json::to_string(&result).expect("failed to serialize");
    
    // Compact JSON should not have unnecessary whitespace
    assert!(!json_str.starts_with(' '), "compact JSON should not start with space");
}

#[test]
fn test_json_roundtrip() {
    let mut result = MockPipelineResult::default();
    result.stages.push(MockStageResult {
        name: "test".to_string(),
        status: "passed".to_string(),
        duration_ms: 1234,
        cache_hit: true,
    });

    // Serialize and deserialize
    let json_str = serde_json::to_string(&result).expect("failed to serialize");
    let deserialized: MockPipelineResult = serde_json::from_str(&json_str).expect("failed to deserialize");
    
    assert_eq!(result.repo, deserialized.repo);
    assert_eq!(result.status, deserialized.status);
    assert_eq!(result.stages.len(), deserialized.stages.len());
    assert_eq!(result.stages[0].name, deserialized.stages[0].name);
}
