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
