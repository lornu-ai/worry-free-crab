use local_ci_core::{matches_patterns, Config, Stage, Workspace};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

pub fn cache_key_for_stage(stage: &Stage, hash: &str) -> String {
    if hash.is_empty() {
        return String::new();
    }
    let cmd_str = match &stage.command {
        Some(cmd) => cmd.join(" "),
        None => String::new(),
    };
    format!("{}|{}", hash, cmd_str)
}

pub fn cache_hit(cache: &HashMap<String, String>, stage: &Stage, hash: &str) -> bool {
    if hash.is_empty() {
        return false;
    }
    match cache.get(&stage.name) {
        Some(entry) => {
            let key = cache_key_for_stage(stage, hash);
            entry == &key || entry == hash
        }
        None => false,
    }
}

pub fn load_cache(root: &Path) -> HashMap<String, String> {
    let cache_path = root.join(".local-ci-cache");
    let mut cache = HashMap::new();
    if let Ok(content) = fs::read_to_string(&cache_path) {
        for line in content.lines() {
            if line.is_empty() {
                continue;
            }
            if let Some((stage, hash)) = line.split_once(':') {
                cache.insert(stage.to_string(), hash.to_string());
            }
        }
    }
    cache
}

pub fn save_cache(cache: &HashMap<String, String>, root: &Path) -> std::io::Result<()> {
    let cache_path = root.join(".local-ci-cache");
    let mut keys: Vec<&String> = cache.keys().collect();
    keys.sort();

    let mut lines = Vec::new();
    for k in keys {
        if let Some(hash) = cache.get(k) {
            lines.push(format!("{}:{}", k, hash));
        }
    }

    fs::write(&cache_path, lines.join("\n") + "\n")
}

pub fn compute_source_hash(
    root: &Path,
    config: &Config,
    ws: Option<&Workspace>,
) -> Result<String, String> {
    let mut context = md5::Context::new();
    let skip_dirs: HashSet<String> = config.cache.skip_dirs.iter().cloned().collect();

    walk_and_hash(
        root,
        root,
        &skip_dirs,
        &config.cache.include_patterns,
        ws,
        &mut context,
    )
    .map_err(|e| format!("Failed to compute source hash: {}", e))?;

    let digest = context.compute();
    Ok(format!("{:x}", digest))
}

pub fn compute_stage_hash(
    stage: &Stage,
    root: &Path,
    config: &Config,
    ws: Option<&Workspace>,
) -> Result<String, String> {
    if stage.watch.is_empty() {
        return compute_source_hash(root, config, ws);
    }

    let mut context = md5::Context::new();
    let skip_dirs: HashSet<String> = config.cache.skip_dirs.iter().cloned().collect();

    walk_and_hash(root, root, &skip_dirs, &stage.watch, ws, &mut context)
        .map_err(|e| format!("Failed to compute stage hash: {}", e))?;

    let digest = context.compute();
    Ok(format!("{:x}", digest))
}

pub fn compute_stage_hashes(
    root: &Path,
    config: &Config,
    ws: Option<&Workspace>,
    stages: &[Stage],
) -> Result<HashMap<String, String>, String> {
    let mut result = HashMap::new();
    for stage in stages {
        let hash = compute_stage_hash(stage, root, config, ws)?;
        result.insert(stage.name.clone(), hash);
    }
    Ok(result)
}

fn walk_and_hash(
    dir: &Path,
    root: &Path,
    skip_dirs: &HashSet<String>,
    include_patterns: &[String],
    ws: Option<&Workspace>,
    context: &mut md5::Context,
) -> std::io::Result<()> {
    if let Ok(entries) = fs::read_dir(dir) {
        // Sort entries alphabetically to guarantee cross-filesystem/cross-OS hashing determinism
        let mut sorted_entries = Vec::new();
        for entry in entries.flatten() {
            sorted_entries.push(entry);
        }
        sorted_entries.sort_by_key(|e| e.file_name());

        for entry in sorted_entries {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().into_owned();

            if entry.file_type()?.is_dir() {
                if skip_dirs.contains(&file_name) {
                    continue;
                }
                if let Some(w) = ws {
                    if !w.is_single {
                        if let Ok(rel) = path.strip_prefix(root) {
                            let rel_str = rel.to_string_lossy().into_owned();
                            if w.is_excluded(&rel_str) {
                                continue;
                            }
                        }
                    }
                }
                walk_and_hash(&path, root, skip_dirs, include_patterns, ws, context)?;
            } else {
                if let Some(w) = ws {
                    if !w.is_single {
                        if let Ok(rel) = path.strip_prefix(root) {
                            let rel_str = rel.to_string_lossy().into_owned();
                            if w.is_excluded(&rel_str) {
                                continue;
                            }
                        }
                    }
                }

                if matches_patterns(&file_name, include_patterns) {
                    if let Ok(data) = fs::read(&path) {
                        context.consume(&data);
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use local_ci_core::{CacheConfig, Config, Stage};
    use std::fs::File;
    use std::io::Write;

    fn make_temp_dir() -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("local_ci_cache_test_{}", nanos));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn test_cache_key_for_stage() {
        let mut stage = Stage {
            name: "test_stage".to_string(),
            command: Some(vec!["cargo".to_string(), "test".to_string()]),
            ..Default::default()
        };

        assert_eq!(cache_key_for_stage(&stage, ""), "");
        assert_eq!(cache_key_for_stage(&stage, "abc"), "abc|cargo test");

        stage.command = None;
        assert_eq!(cache_key_for_stage(&stage, "abc"), "abc|");
    }

    #[test]
    fn test_cache_hit() {
        let stage = Stage {
            name: "test_stage".to_string(),
            command: Some(vec!["cargo".to_string(), "test".to_string()]),
            ..Default::default()
        };

        let mut cache = HashMap::new();
        // Entry with format hash|cmd
        cache.insert("test_stage".to_string(), "abc|cargo test".to_string());

        assert!(cache_hit(&cache, &stage, "abc"));
        assert!(!cache_hit(&cache, &stage, "def"));
        assert!(!cache_hit(&cache, &stage, ""));

        // Entry with format hash only
        let mut cache_simple = HashMap::new();
        cache_simple.insert("test_stage".to_string(), "abc".to_string());
        assert!(cache_hit(&cache_simple, &stage, "abc"));

        // No entry
        assert!(!cache_hit(&HashMap::new(), &stage, "abc"));
    }

    #[test]
    fn test_save_and_load_cache() {
        let path = make_temp_dir();
        let mut cache = HashMap::new();
        cache.insert("stage_a".to_string(), "hash_a".to_string());
        cache.insert("stage_b".to_string(), "hash_b".to_string());

        save_cache(&cache, &path).unwrap();

        // Verify file was written
        let cache_file = path.join(".local-ci-cache");
        assert!(cache_file.exists());

        // Load back
        let loaded = load_cache(&path);
        assert_eq!(loaded.get("stage_a").unwrap(), "hash_a");
        assert_eq!(loaded.get("stage_b").unwrap(), "hash_b");
        assert_eq!(loaded.len(), 2);

        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn test_compute_hashes() {
        let path = make_temp_dir();

        // Write some source files
        let mut f1 = File::create(path.join("main.rs")).unwrap();
        writeln!(f1, "fn main() {{}}").unwrap();

        let mut f2 = File::create(path.join("Cargo.toml")).unwrap();
        writeln!(f2, "[package]").unwrap();

        // Write a directory to skip
        let target_dir = path.join("target");
        std::fs::create_dir_all(&target_dir).unwrap();
        let mut f3 = File::create(target_dir.join("build_output.log")).unwrap();
        writeln!(f3, "build logs...").unwrap();

        let config = Config {
            cache: CacheConfig {
                skip_dirs: vec!["target".to_string()],
                include_patterns: vec!["*.rs".to_string(), "*.toml".to_string()],
            },
            ..Default::default()
        };

        // Compute initial source hash
        let hash1 = compute_source_hash(&path, &config, None).unwrap();
        assert!(!hash1.is_empty());

        // Modify target log file (which is skipped) and assert hash is identical
        writeln!(f3, "more logs").unwrap();
        let hash2 = compute_source_hash(&path, &config, None).unwrap();
        assert_eq!(hash1, hash2);

        // Modify a watched file and assert hash changed
        let mut f1_mod = File::create(path.join("main.rs")).unwrap();
        writeln!(f1_mod, "fn main() {{ println!(\"hello\"); }}").unwrap();
        let hash3 = compute_source_hash(&path, &config, None).unwrap();
        assert_ne!(hash1, hash3);

        // Test stage hash with a specific watch pattern
        let stage = Stage {
            name: "clippy".to_string(),
            watch: vec!["*.toml".to_string()],
            ..Default::default()
        };

        let hash_toml_only = compute_stage_hash(&stage, &path, &config, None).unwrap();

        // Modifying main.rs (not in watch) shouldn't affect the stage hash
        let mut f1_mod2 = File::create(path.join("main.rs")).unwrap();
        writeln!(f1_mod2, "fn main() {{ println!(\"world\"); }}").unwrap();
        let hash_toml_only_2 = compute_stage_hash(&stage, &path, &config, None).unwrap();
        assert_eq!(hash_toml_only, hash_toml_only_2);

        // Modifying Cargo.toml (in watch) should affect the stage hash
        let mut f2_mod = File::create(path.join("Cargo.toml")).unwrap();
        writeln!(f2_mod, "[package]\nname = \"test\"").unwrap();
        let hash_toml_only_3 = compute_stage_hash(&stage, &path, &config, None).unwrap();
        assert_ne!(hash_toml_only, hash_toml_only_3);

        let _ = std::fs::remove_dir_all(&path);
    }
}
