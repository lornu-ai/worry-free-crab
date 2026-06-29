use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Parser, Debug)]
#[command(name = "local-ci")]
#[command(version = "0.3.0")]
#[command(about = "Universal local CI runner for any project type")]
struct Cli {
    #[arg(long, help = "Disable file hash cache")]
    no_cache: bool,

    #[arg(long, help = "Show detailed output")]
    verbose: bool,

    #[arg(long, help = "Auto-fix issues (cargo fmt)")]
    fix: bool,

    #[arg(long, help = "List available stages")]
    list: bool,

    #[arg(
        long,
        help = "List named remote host presets from .local-ci-remote.toml"
    )]
    list_remote_hosts: bool,

    #[arg(long, help = "Run all stages including disabled ones")]
    all: bool,

    #[arg(long, help = "Output in JSON format")]
    json: bool,

    #[arg(long, help = "Use a named profile from config")]
    profile: Option<String>,

    #[arg(long, help = "Show what would run without executing")]
    dry_run: bool,

    #[arg(long, help = "Number of parallel jobs (0 = auto)")]
    parallel: Option<usize>,

    #[arg(long, help = "Stop on first failure")]
    fail_fast: bool,

    // Remote stubs
    #[arg(long, help = "Run remotely on specified SSH host (e.g., user@host)")]
    remote: Option<String>,

    #[arg(
        long,
        help = "Run remotely using a named preset from .local-ci-remote.toml"
    )]
    remote_host: Option<String>,

    #[arg(
        long,
        default_value = "onion",
        help = "tmux session name for remote execution"
    )]
    session: String,

    #[arg(long, default_value_t = 30, help = "SSH operation timeout in seconds")]
    remote_timeout: usize,

    #[arg(long, help = "Remote working directory (defaults to /tmp/<basename>)")]
    remote_dir: Option<String>,
    // FFT options
    #[arg(long, help = "Path to GitHub event JSON file for FFT")]
    event: Option<String>,

    #[arg(long, help = "Repo path or name for FFT")]
    repo: Option<String>,

    #[arg(long, help = "Output file path for FFT results")]
    out: Option<String>,

    // Positional arguments
    #[arg(help = "Subcommands (init, serve, check, fft) or stage names to run")]
    args: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DryRunStage {
    name: String,
    command: String,
    would_run: bool,
    reason: String, // "cached", "hash_changed", "disabled", "no_cache_flag"
}

#[derive(Debug, Serialize, Deserialize)]
struct DryRunRemote {
    host: String,
    session: String,
    work_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    host_preset: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DryRunReport {
    workspace: String,
    source_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote: Option<DryRunRemote>,
    stages: Vec<DryRunStage>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
    id: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

fn main() {
    let mut cli = Cli::parse();

    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            local_ci_report::errorf!("Cannot get working directory: {}\n", e);
            std::process::exit(1);
        }
    };

    // 1. Handle Subcommands
    if !cli.args.is_empty() {
        if cli.args[0] == "init" {
            cmd_init(&cwd);
            return;
        } else if cli.args[0] == "serve" {
            if let Err(e) = cmd_serve(&cwd) {
                local_ci_report::errorf!("MCP server error: {}\n", e);
                std::process::exit(1);
            }
            return;
        } else if cli.args[0] == "check" {
            cmd_check(&cwd, cli.json);
            return;
        } else if cli.args[0] == "fft" {
            cmd_fft(&cwd, &cli);
            return;
        }
    }

    // 2. Verify that .wfc-ci.toml or .local-ci.toml exists
    let wfc_config_path = cwd.join(".wfc-ci.toml");
    let local_config_path = cwd.join(".local-ci.toml");
    let (config_file_path, is_deprecated) = if wfc_config_path.exists() {
        (wfc_config_path, false)
    } else {
        (local_config_path, true)
    };

    if !config_file_path.exists() {
        local_ci_report::errorf!(
            "Error: Config file not found (.wfc-ci.toml).\n\
             Please run `local-ci init` to initialize the project configuration.\n"
        );
        std::process::exit(1);
    }

    if is_deprecated {
        local_ci_report::warnf!(
            "Warning: .local-ci.toml is deprecated. Please rename it to .wfc-ci.toml.\n"
        );
    }

    let need_remote_cfg =
        cli.remote.is_some() || cli.remote_host.is_some() || cli.list_remote_hosts;

    // 3. Load config & workspace
    let mut config = match local_ci_detect::load_config(&cwd, need_remote_cfg) {
        Ok(cfg) => cfg,
        Err(e) => {
            local_ci_report::errorf!("Failed to load config: {}\n", e);
            std::process::exit(1);
        }
    };

    if cli.list_remote_hosts {
        if config.hosts.is_empty() {
            println!("No remote host presets defined. Add [hosts.<name>] to .local-ci-remote.toml");
            return;
        }
        let mut sorted_host_names: Vec<String> = config.hosts.keys().cloned().collect();
        sorted_host_names.sort();
        for name in sorted_host_names {
            if let Some(h) = config.hosts.get(&name) {
                let norm = config.normalize_remote_host(&name, h);
                let mut line = format!("{}  host={}", name, norm.host);
                if !norm.session.is_empty() {
                    line.push_str(&format!("  session={}", norm.session));
                }
                if !norm.remote_dir.is_empty() {
                    line.push_str(&format!("  remote_dir={}", norm.remote_dir));
                }
                if !norm.description.is_empty() {
                    line.push_str(&format!("  # {}", norm.description));
                }
                println!("{}", line);
            }
        }
        return;
    }

    let ws = local_ci_detect::detect_workspace(&cwd).ok();

    // 4. Handle List stage
    if cli.list {
        println!("Available stages:");
        let all_stages = config.get_all_stages();
        for name in all_stages {
            if let Some(stage) = config.stages.get(&name) {
                let status = if stage.enabled { "enabled" } else { "disabled" };
                println!("  {} ({})", name, status);
            }
        }
        return;
    }

    // Apply profile if specified
    if let Some(profile_name) = &cli.profile {
        if let Some(profile) = config.profiles.get(profile_name) {
            if profile.no_cache {
                cli.no_cache = true;
            }
            if profile.fail_fast {
                cli.fail_fast = true;
            }
            if profile.json {
                cli.json = true;
            }

            // Disable all stages first, then enable only the ones in the profile
            for stage in config.stages.values_mut() {
                stage.enabled = false;
            }
            for stage_name in &profile.stages {
                if let Some(stage) = config.stages.get_mut(stage_name) {
                    stage.enabled = true;
                }
            }
        } else {
            local_ci_report::errorf!("Profile '{}' not found in config\n", profile_name);
            std::process::exit(1);
        }
    }

    // Set output JSON mode
    let is_json = cli.json;
    if is_json {
        local_ci_report::set_json_mode(true);
    }

    // 5. Resolve Stages to run
    let mut stages_to_run = Vec::new();
    if !cli.args.is_empty() {
        for stage_name in &cli.args {
            if let Some(stage) = config.stages.get(stage_name) {
                let mut s = stage.clone();
                s.name = stage_name.clone();
                if cli.all {
                    s.enabled = true;
                }
                stages_to_run.push(s);
            } else {
                local_ci_report::errorf!("Unknown stage: {}\n", stage_name);
                std::process::exit(1);
            }
        }
    } else if cli.all {
        let all_stages = config.get_all_stages();
        for name in all_stages {
            if let Some(stage) = config.stages.get(&name) {
                let mut s = stage.clone();
                s.name = name;
                s.enabled = true;
                stages_to_run.push(s);
            }
        }
    } else if let Some(profile_name) = &cli.profile {
        if let Some(profile) = config.profiles.get(profile_name) {
            let profile_stages = config.get_profile_stages(profile);
            for name in profile_stages {
                if let Some(stage) = config.stages.get(&name) {
                    let mut s = stage.clone();
                    s.name = name;
                    stages_to_run.push(s);
                }
            }
        }
    } else {
        let enabled_stages = config.get_enabled_stages();
        for name in enabled_stages {
            if let Some(stage) = config.stages.get(&name) {
                let mut s = stage.clone();
                s.name = name;
                stages_to_run.push(s);
            }
        }
    }

    // Modify fmt command if --fix is set
    if cli.fix {
        for stage in &mut stages_to_run {
            if stage.name == "fmt" && stage.fix_command.is_some() {
                stage.command = stage.fix_command.clone();
                stage.check = false;
            }
        }
    }

    // 6. Hashing & Caching Setup
    let source_hash = match local_ci_cache::compute_source_hash(&cwd, &config, ws.as_ref()) {
        Ok(h) => h,
        Err(e) => {
            local_ci_report::warnf!("Warning: hash computation failed: {}\n", e);
            cli.no_cache = true;
            String::new()
        }
    };

    let no_cache = cli.no_cache || source_hash.is_empty();

    let mut cache = if no_cache {
        HashMap::new()
    } else {
        local_ci_cache::load_cache(&cwd)
    };

    let mut stage_hashes = HashMap::new();
    for stage in &stages_to_run {
        let hash = if !stage.watch.is_empty() {
            match local_ci_cache::compute_stage_hash(stage, &cwd, &config, ws.as_ref()) {
                Ok(h) => h,
                Err(e) => {
                    if cli.verbose {
                        local_ci_report::warnf!(
                            "Warning: stage hash computation failed for {}: {}\n",
                            stage.name,
                            e
                        );
                    }
                    source_hash.clone()
                }
            }
        } else {
            source_hash.clone()
        };
        stage_hashes.insert(stage.name.clone(), hash);
    }

    // 7. Dry-Run Handling
    if cli.dry_run {
        let report = build_dry_run_report(
            &stages_to_run,
            &cache,
            &stage_hashes,
            &source_hash,
            no_cache,
            None,
            &cwd,
        );
        if is_json {
            print_dry_run_json(&report);
        } else {
            print_dry_run_human(&report);
        }
        return;
    }

    // 8. Execute Stages
    let mut results = Vec::new();
    let start_time = std::time::Instant::now();

    let mut remote_target = cli.remote.clone();
    let mut remote_session = cli.session.clone();
    let mut remote_dir = cli.remote_dir.clone();

    if let Some(preset_name) = &cli.remote_host {
        if let Some(host_cfg) = config.hosts.get(preset_name) {
            let norm = config.normalize_remote_host(preset_name, host_cfg);
            remote_target = Some(norm.host);
            if !norm.session.is_empty() {
                remote_session = norm.session;
            }
            if !norm.remote_dir.is_empty() {
                remote_dir = Some(norm.remote_dir);
            }
        } else {
            local_ci_report::errorf!("Remote host preset '{}' not found in config\n", preset_name);
            std::process::exit(1);
        }
    }

    if let Some(remote_host) = remote_target {
        if cli.parallel.is_some() {
            local_ci_report::errorf!(
                "Cannot use --parallel and --remote together; run remote stages sequentially\n"
            );
            std::process::exit(1);
        }

        let work_dir = match remote_dir {
            Some(dir) => dir,
            None => {
                let base = cwd
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("project");
                format!("/tmp/{}", base)
            }
        };

        let re = local_ci_exec::RemoteExecutor::new(
            remote_host.clone(),
            remote_session,
            work_dir,
            std::time::Duration::from_secs(cli.remote_timeout as u64),
            cli.verbose,
        );

        local_ci_report::printf!("🔄 Synchronizing local workspace to remote...\n");
        if let Err(e) = re.sync_workspace(cwd.to_str().unwrap_or("."), &config.cache.skip_dirs) {
            local_ci_report::errorf!("Failed to sync workspace to remote: {}\n", e);
            std::process::exit(1);
        }

        local_ci_report::printf!(
            "🚀 Running local CI pipeline remotely on {}...\n\n",
            remote_host
        );

        if let Err(e) = re.ensure_remote_session() {
            local_ci_report::errorf!("Failed to initialize remote tmux session: {}\n", e);
            std::process::exit(1);
        }

        for stage in &stages_to_run {
            let stage_start = std::time::Instant::now();
            let stage_hash = stage_hashes.get(&stage.name).unwrap_or(&source_hash);

            if !no_cache && local_ci_cache::cache_hit(&cache, stage, stage_hash) {
                if cli.verbose {
                    local_ci_report::printf!("✓ {} (cached)\n", stage.name);
                }
                results.push(local_ci_exec::Result {
                    name: stage.name.clone(),
                    command: String::new(),
                    status: "pass".to_string(),
                    duration: std::time::Duration::ZERO,
                    output: String::new(),
                    cache_hit: true,
                    error: None,
                });
                continue;
            }

            local_ci_report::printf!("::group::{}\n", stage.name);

            if stage.command.is_none() || stage.command.as_ref().unwrap().is_empty() {
                local_ci_report::printf!("Error: Stage has no command defined\n");
                local_ci_report::printf!("::endgroup::\n");
                local_ci_report::errorf!("✗ {} (failed)\n", stage.name);
                results.push(local_ci_exec::Result {
                    name: stage.name.clone(),
                    command: String::new(),
                    status: "fail".to_string(),
                    duration: std::time::Duration::ZERO,
                    output: String::new(),
                    cache_hit: false,
                    error: Some("no command defined".to_string()),
                });
                if cli.fail_fast {
                    break;
                }
                continue;
            }

            if cli.verbose {
                let cmd_str = stage.command.as_ref().unwrap().join(" ");
                local_ci_report::printf!("$ {}\n", cmd_str);
            }

            let mut r = re.execute_stage(stage);
            r.duration = stage_start.elapsed();

            if r.status == "fail" {
                if !r.output.is_empty() {
                    local_ci_report::printf!("{}\n", r.output);
                } else if let Some(err) = &r.error {
                    local_ci_report::printf!("Error: {}\n", err);
                }
                local_ci_report::printf!("::endgroup::\n");
                local_ci_report::errorf!("✗ {} (failed)\n", stage.name);
                results.push(r);
                if cli.fail_fast {
                    break;
                }
            } else {
                if cli.verbose && !r.output.is_empty() {
                    local_ci_report::printf!("{}\n", r.output);
                }
                local_ci_report::printf!("::endgroup::\n");
                local_ci_report::successf!("✓ {} ({}ms)\n", stage.name, r.duration.as_millis());
                results.push(r);

                // Update cache
                cache.insert(
                    stage.name.clone(),
                    local_ci_cache::cache_key_for_stage(stage, stage_hash),
                );
            }
        }
    } else if let Some(concurrency) = cli.parallel {
        let runner = local_ci_exec::ParallelRunner {
            stages: stages_to_run,
            concurrency,
            cwd: cwd.clone(),
            no_cache,
            cache: std::sync::Arc::new(std::sync::Mutex::new(cache.clone())),
            source_hash,
            stage_hashes,
            verbose: cli.verbose,
            json: is_json,
            fail_fast: cli.fail_fast,
        };
        results = runner.run();
        cache = runner.cache.lock().unwrap().clone();
    } else {
        local_ci_report::printf!("🚀 Running local CI pipeline...\n\n");

        for stage in &stages_to_run {
            let stage_start = std::time::Instant::now();
            let stage_hash = stage_hashes.get(&stage.name).unwrap_or(&source_hash);

            if !no_cache && local_ci_cache::cache_hit(&cache, stage, stage_hash) {
                if cli.verbose {
                    local_ci_report::printf!("✓ {} (cached)\n", stage.name);
                }
                results.push(local_ci_exec::Result {
                    name: stage.name.clone(),
                    command: String::new(),
                    status: "pass".to_string(),
                    duration: std::time::Duration::ZERO,
                    output: String::new(),
                    cache_hit: true,
                    error: None,
                });
                continue;
            }

            local_ci_report::printf!("::group::{}\n", stage.name);

            let cmd_parts = match &stage.command {
                Some(parts) if !parts.is_empty() => parts,
                _ => {
                    local_ci_report::printf!("Error: Stage has no command defined\n");
                    local_ci_report::printf!("::endgroup::\n");
                    local_ci_report::errorf!("✗ {} (failed)\n", stage.name);
                    results.push(local_ci_exec::Result {
                        name: stage.name.clone(),
                        command: String::new(),
                        status: "fail".to_string(),
                        duration: stage_start.elapsed(),
                        output: String::new(),
                        cache_hit: false,
                        error: Some("no command defined".to_string()),
                    });
                    if cli.fail_fast {
                        break;
                    }
                    continue;
                }
            };

            if cli.verbose {
                let cmd_str = cmd_parts.join(" ");
                local_ci_report::printf!("$ {}\n", cmd_str);
            }

            let r = local_ci_exec::execute_single_stage(stage, &cwd, no_cache, &cache, stage_hash);
            let duration = r.duration;

            if r.status == "fail" {
                if !r.output.is_empty() {
                    local_ci_report::printf!("{}\n", r.output);
                } else if let Some(err) = &r.error {
                    local_ci_report::printf!("Error: {}\n", err);
                }
                local_ci_report::printf!("::endgroup::\n");
                local_ci_report::errorf!("✗ {} (failed)\n", stage.name);
                results.push(r);
                if cli.fail_fast {
                    break;
                }
            } else {
                if cli.verbose && !r.output.is_empty() {
                    local_ci_report::printf!("{}\n", r.output);
                }
                local_ci_report::printf!("::endgroup::\n");
                local_ci_report::successf!("✓ {} ({}ms)\n", stage.name, duration.as_millis());
                results.push(r);

                // Update cache
                cache.insert(
                    stage.name.clone(),
                    local_ci_cache::cache_key_for_stage(stage, stage_hash),
                );
            }
        }
    }

    // 9. Save Cache
    if !no_cache {
        let _ = local_ci_cache::save_cache(&cache, &cwd);
    }

    // 10. Summary
    local_ci_report::printf!("\n");
    let total_duration = start_time.elapsed();
    local_ci_report::print_report(&results, total_duration, is_json);

    // Exit code
    let has_failures = results.iter().any(|r| r.status == "fail");
    if has_failures {
        std::process::exit(1);
    }
}

// ============================================================================
// Subcommands Implementation
// ============================================================================

fn cmd_init(root: &Path) {
    let ws = match local_ci_detect::detect_workspace(root) {
        Ok(w) => w,
        Err(e) => {
            local_ci_report::errorf!("Failed to detect workspace: {}\n", e);
            std::process::exit(1);
        }
    };

    local_ci_report::printf!("📦 Initializing local-ci for {}\n", root.display());

    if ws.is_single {
        local_ci_report::printf!("  Single crate: {}\n", ws.members[0]);
    } else {
        local_ci_report::printf!("  Workspace with {} members\n", ws.members.len());
        for member in &ws.members {
            if !ws.is_excluded(member) {
                local_ci_report::printf!("    ✓ {}\n", member);
            } else {
                local_ci_report::printf!("    ✗ {} (excluded)\n", member);
            }
        }
    }

    // Save default config TOML
    let project_type = local_ci_detect::detect_project_type(root);
    let template = local_ci_detect::get_config_template_for_type(project_type, root);
    let config_path = root.join(".wfc-ci.toml");
    if let Err(e) = std::fs::write(&config_path, template) {
        local_ci_report::errorf!("Failed to create .wfc-ci.toml: {}\n", e);
        std::process::exit(1);
    }
    local_ci_report::successf!("✅ Created .wfc-ci.toml\n");

    // Update .gitignore
    if let Err(e) = update_gitignore(root) {
        local_ci_report::warnf!("Could not update .gitignore: {}\n", e);
    } else {
        local_ci_report::successf!("✅ Updated .gitignore\n");
    }

    // Try to create pre-commit hook if .git exists
    let git_dir = root.join(".git");
    if git_dir.exists() {
        if let Err(e) = create_pre_commit_hook(root, project_type) {
            local_ci_report::warnf!("Could not create pre-commit hook: {}\n", e);
        } else {
            local_ci_report::successf!("✅ Created pre-commit hook\n");
        }
    }

    local_ci_report::printf!("\n💡 Next steps:\n");
    local_ci_report::printf!("  1. Run 'local-ci' to test the setup\n");
    local_ci_report::printf!("  2. Customize .wfc-ci.toml as needed\n");
    if project_type == local_ci_detect::ProjectType::Rust {
        local_ci_report::printf!("  3. Consider installing cargo tools:\n");
        local_ci_report::printf!("     - cargo install cargo-deny\n");
        local_ci_report::printf!("     - cargo install cargo-audit\n");
        local_ci_report::printf!("     - cargo install cargo-machete\n");
    }
}

fn update_gitignore(root: &Path) -> std::io::Result<()> {
    let gitignore_path = root.join(".gitignore");
    let mut content = if gitignore_path.exists() {
        std::fs::read_to_string(&gitignore_path).unwrap_or_default()
    } else {
        String::new()
    };

    if content.contains(".local-ci-cache") {
        return Ok(());
    }

    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(".local-ci-cache\n");

    std::fs::write(&gitignore_path, content)?;
    Ok(())
}

fn get_pre_commit_hook_template(project_type: local_ci_detect::ProjectType) -> String {
    let stages_cmd = match project_type {
        local_ci_detect::ProjectType::Rust => "fmt clippy",
        local_ci_detect::ProjectType::Python => "format lint",
        local_ci_detect::ProjectType::Go => "fmt vet",
        local_ci_detect::ProjectType::TypeScript => "install typecheck lint",
        local_ci_detect::ProjectType::Java => "build",
        local_ci_detect::ProjectType::Swift => "fmt",
        _ => "fmt",
    };

    format!(
        r#"#!/bin/bash
# local-ci pre-commit hook
# Auto-generated by local-ci init
# Runs local-ci before allowing commits

set -e

# Run local-ci with fast checks
if ! local-ci {}; then
  echo ""
  echo "❌ Pre-commit checks failed. Fix the issues above and try again."
  echo ""
  echo "💡 Tip: Run 'local-ci --fix' to auto-fix formatting issues."
  exit 1
fi

echo "✅ Pre-commit checks passed"
"#,
        stages_cmd
    )
}

fn create_pre_commit_hook(
    root: &Path,
    project_type: local_ci_detect::ProjectType,
) -> std::io::Result<()> {
    let mut git_dir = root.join(".git");
    if git_dir.is_file() {
        if let Ok(content) = std::fs::read_to_string(&git_dir) {
            if let Some(line) = content.lines().next() {
                if let Some(path_str) = line.strip_prefix("gitdir:") {
                    let path = Path::new(path_str.trim());
                    if path.is_absolute() {
                        git_dir = path.to_path_buf();
                    } else {
                        git_dir = root.join(path);
                    }
                }
            }
        }
    }
    let hooks_dir = git_dir.join("hooks");
    std::fs::create_dir_all(&hooks_dir)?;

    let pre_commit_path = hooks_dir.join("pre-commit");

    let existing_content = if pre_commit_path.exists() {
        std::fs::read_to_string(&pre_commit_path).unwrap_or_default()
    } else {
        String::new()
    };

    if existing_content.contains("local-ci") {
        return Ok(());
    }

    let hook_content = get_pre_commit_hook_template(project_type);
    let final_content = if !existing_content.is_empty() {
        if !existing_content.starts_with("#!") {
            format!("{}\n{}", existing_content, hook_content)
        } else {
            // Append to existing file nicely
            format!("{}\n{}", existing_content, hook_content)
        }
    } else {
        hook_content
    };

    std::fs::write(&pre_commit_path, final_content)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&pre_commit_path, std::fs::Permissions::from_mode(0o755))?;
    }

    Ok(())
}

fn cmd_serve(cwd: &Path) -> Result<(), String> {
    let wfc_config_path = cwd.join(".wfc-ci.toml");
    let local_config_path = cwd.join(".local-ci.toml");
    let config_file_path = if wfc_config_path.exists() {
        wfc_config_path
    } else {
        local_config_path
    };

    if !config_file_path.exists() {
        return Err("Config file not found (.wfc-ci.toml). Please run `local-ci init` to initialize the project configuration.".to_string());
    }
    let config = local_ci_detect::load_config(cwd, false)
        .map_err(|e| format!("Failed to load config: {}", e))?;
    let ws = local_ci_detect::detect_workspace(cwd).ok();
    let source_hash =
        local_ci_cache::compute_source_hash(cwd, &config, ws.as_ref()).unwrap_or_default();

    let stdin = std::io::stdin();
    for line in stdin.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }

        let req: Result<JsonRpcRequest, serde_json::Error> = serde_json::from_str(&line);
        match req {
            Ok(r) => {
                if r.jsonrpc != "2.0" {
                    send_rpc_error(
                        r.id,
                        -32600,
                        "Invalid Request: invalid jsonrpc version",
                        None,
                    );
                    continue;
                }

                match r.method.as_str() {
                    "initialize" => {
                        let result = serde_json::json!({
                            "protocolVersion": "2024-11-05",
                            "capabilities": {
                                "tools": {}
                            },
                            "serverInfo": {
                                "name": "local-ci",
                                "version": "0.3.0"
                            }
                        });
                        send_rpc_response(r.id, Some(result));
                    }
                    "notifications/initialized" => {
                        // Protocol notification, no response
                    }
                    "tools/list" => {
                        let result = serde_json::json!({
                            "tools": [
                                {
                                    "name": "run_stage",
                                    "description": "Run a single CI stage by name and return the result",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "name": {
                                                "type": "string",
                                                "description": "Stage name to run (e.g. fmt, clippy, test)"
                                            }
                                        },
                                        "required": ["name"]
                                    }
                                },
                                {
                                    "name": "run_all",
                                    "description": "Run all enabled CI stages and return results",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {}
                                    }
                                },
                                {
                                    "name": "get_stages",
                                    "description": "List all stages with their enabled status and cache state",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {}
                                    }
                                },
                                {
                                    "name": "get_stale_stages",
                                    "description": "List stages that need to run (cache miss or no cache)",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {}
                                    }
                                },
                                {
                                    "name": "invalidate",
                                    "description": "Clear the cache for a specific stage, forcing it to re-run next time",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "name": {
                                                "type": "string",
                                                "description": "Stage name to invalidate"
                                            }
                                        },
                                        "required": ["name"]
                                    }
                                },
                                {
                                    "name": "get_source_hash",
                                    "description": "Compute and return the current source hash used for cache keys",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {}
                                    }
                                },
                                {
                                    "name": "get_workspace",
                                    "description": "Return the detected workspace structure (members, excludes, project type)",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {}
                                    }
                                }
                            ]
                        });
                        send_rpc_response(r.id, Some(result));
                    }
                    "tools/call" => {
                        let name = r
                            .params
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        let arguments =
                            r.params.get("arguments").cloned().unwrap_or_else(|| {
                                serde_json::Value::Object(serde_json::Map::new())
                            });

                        let result = match name {
                            "run_stage" => {
                                let stage_name = arguments.get("name").and_then(|v| v.as_str());
                                match stage_name {
                                    None => {
                                        make_mcp_error_result("missing required parameter: name")
                                    }
                                    Some(s_name) => {
                                        match config.stages.get(s_name) {
                                            None => make_mcp_error_result(&format!(
                                                "unknown stage: {}",
                                                s_name
                                            )),
                                            Some(stage) => {
                                                if !stage.enabled {
                                                    make_mcp_error_result(&format!("stage \"{}\" is disabled; enable it in .local-ci.toml to run", s_name))
                                                } else {
                                                    let mut s = stage.clone();
                                                    s.name = s_name.to_string();
                                                    let res = execute_mcp_stage(
                                                        &s,
                                                        cwd,
                                                        &config,
                                                        ws.as_ref(),
                                                    );
                                                    let res_json = local_ci_report::ResultJSON::from_exec_result(&res);
                                                    let text = serde_json::to_string(&res_json)
                                                        .unwrap_or_default();
                                                    make_mcp_success_result(&text)
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            "run_all" => {
                                let enabled_names = config.get_enabled_stages();
                                let mut results = Vec::new();
                                for name in enabled_names {
                                    if let Some(stage) = config.stages.get(&name) {
                                        let mut s = stage.clone();
                                        s.name = name;
                                        let res = execute_mcp_stage(&s, cwd, &config, ws.as_ref());
                                        results.push(
                                            local_ci_report::ResultJSON::from_exec_result(&res),
                                        );
                                    }
                                }
                                let text = serde_json::to_string(&results).unwrap_or_default();
                                make_mcp_success_result(&text)
                            }
                            "get_stages" => {
                                let cache = local_ci_cache::load_cache(cwd);
                                #[derive(Serialize)]
                                struct StageInfo {
                                    name: String,
                                    enabled: bool,
                                    cache_hit: bool,
                                    command: String,
                                }
                                let mut stages_info = Vec::new();
                                for (name, stage) in &config.stages {
                                    let mut s = stage.clone();
                                    s.name = name.clone();
                                    let hash = if !s.watch.is_empty() {
                                        local_ci_cache::compute_stage_hash(
                                            &s,
                                            cwd,
                                            &config,
                                            ws.as_ref(),
                                        )
                                        .unwrap_or_else(|_| source_hash.clone())
                                    } else {
                                        source_hash.clone()
                                    };
                                    let hit = local_ci_cache::cache_hit(&cache, &s, &hash);
                                    let cmd_str = match &s.command {
                                        Some(cmd) => cmd.join(" "),
                                        None => String::new(),
                                    };
                                    stages_info.push(StageInfo {
                                        name: name.clone(),
                                        enabled: s.enabled,
                                        cache_hit: hit,
                                        command: cmd_str,
                                    });
                                }
                                stages_info.sort_by(|a, b| a.name.cmp(&b.name));
                                let text = serde_json::to_string(&stages_info).unwrap_or_default();
                                make_mcp_success_result(&text)
                            }
                            "get_stale_stages" => {
                                let cache = local_ci_cache::load_cache(cwd);
                                #[derive(Serialize)]
                                struct StaleStage {
                                    name: String,
                                    reason: String,
                                }
                                let mut stale = Vec::new();
                                for (name, stage) in &config.stages {
                                    if !stage.enabled {
                                        continue;
                                    }
                                    let mut s = stage.clone();
                                    s.name = name.clone();
                                    let hash = if !s.watch.is_empty() {
                                        local_ci_cache::compute_stage_hash(
                                            &s,
                                            cwd,
                                            &config,
                                            ws.as_ref(),
                                        )
                                        .unwrap_or_else(|_| source_hash.clone())
                                    } else {
                                        source_hash.clone()
                                    };
                                    if !local_ci_cache::cache_hit(&cache, &s, &hash) {
                                        let reason = if cache.contains_key(name) {
                                            "source changed"
                                        } else {
                                            "never run"
                                        };
                                        stale.push(StaleStage {
                                            name: name.clone(),
                                            reason: reason.to_string(),
                                        });
                                    }
                                }
                                stale.sort_by(|a, b| a.name.cmp(&b.name));
                                let text = serde_json::to_string(&stale).unwrap_or_default();
                                make_mcp_success_result(&text)
                            }
                            "invalidate" => {
                                let stage_name = arguments.get("name").and_then(|v| v.as_str());
                                match stage_name {
                                    None => {
                                        make_mcp_error_result("missing required parameter: name")
                                    }
                                    Some(s_name) => {
                                        if !config.stages.contains_key(s_name) {
                                            make_mcp_error_result(&format!(
                                                "unknown stage: {}",
                                                s_name
                                            ))
                                        } else {
                                            #[derive(Serialize)]
                                            struct InvalidateResp {
                                                stage: String,
                                                status: String,
                                            }
                                            let mut cache = local_ci_cache::load_cache(cwd);
                                            let status = if cache.contains_key(s_name) {
                                                cache.remove(s_name);
                                                let _ = local_ci_cache::save_cache(&cache, cwd);
                                                "invalidated"
                                            } else {
                                                "no_cache_entry"
                                            };
                                            let resp = InvalidateResp {
                                                stage: s_name.to_string(),
                                                status: status.to_string(),
                                            };
                                            let text =
                                                serde_json::to_string(&resp).unwrap_or_default();
                                            make_mcp_success_result(&text)
                                        }
                                    }
                                }
                            }
                            "get_source_hash" => {
                                #[derive(Serialize)]
                                struct HashResp {
                                    hash: String,
                                }
                                let resp = HashResp {
                                    hash: source_hash.clone(),
                                };
                                let text = serde_json::to_string(&resp).unwrap_or_default();
                                make_mcp_success_result(&text)
                            }
                            "get_workspace" => {
                                let project_type = local_ci_detect::detect_project_type(cwd);
                                #[derive(Serialize)]
                                struct WsInfo {
                                    root: String,
                                    project_type: String,
                                    members: Vec<String>,
                                    is_single: bool,
                                }
                                let info = WsInfo {
                                    root: cwd.to_string_lossy().to_string(),
                                    project_type: project_type.to_string(),
                                    members: ws
                                        .as_ref()
                                        .map(|w| w.get_included_members())
                                        .unwrap_or_else(|| vec![".".to_string()]),
                                    is_single: ws.as_ref().map(|w| w.is_single).unwrap_or(true),
                                };
                                let text = serde_json::to_string(&info).unwrap_or_default();
                                make_mcp_success_result(&text)
                            }
                            _ => {
                                send_rpc_error(
                                    r.id,
                                    -32601,
                                    &format!("Method not found: {}", name),
                                    None,
                                );
                                continue;
                            }
                        };
                        send_rpc_response(r.id, Some(result));
                    }
                    _ => {
                        send_rpc_error(
                            r.id,
                            -32601,
                            &format!("Method not found: {}", r.method),
                            None,
                        );
                    }
                }
            }
            Err(_) => {
                send_rpc_error(None, -32700, "Parse error: invalid json received", None);
            }
        }
    }

    Ok(())
}

fn execute_mcp_stage(
    stage: &local_ci_core::Stage,
    cwd: &Path,
    config: &local_ci_core::Config,
    ws: Option<&local_ci_core::Workspace>,
) -> local_ci_exec::Result {
    let hash = if !stage.watch.is_empty() {
        local_ci_cache::compute_stage_hash(stage, cwd, config, ws).unwrap_or_default()
    } else {
        local_ci_cache::compute_source_hash(cwd, config, ws).unwrap_or_default()
    };

    let cache = local_ci_cache::load_cache(cwd);
    let r = local_ci_exec::execute_single_stage(stage, cwd, false, &cache, &hash);

    if r.status == "pass" && !r.cache_hit {
        let mut updated_cache = cache.clone();
        updated_cache.insert(
            stage.name.clone(),
            local_ci_cache::cache_key_for_stage(stage, &hash),
        );
        let _ = local_ci_cache::save_cache(&updated_cache, cwd);
    }

    r
}

fn send_rpc_response(id: Option<serde_json::Value>, result: Option<serde_json::Value>) {
    let response = JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        result,
        error: None,
        id,
    };
    if let Ok(serialized) = serde_json::to_string(&response) {
        println!("{}", serialized);
    }
}

fn send_rpc_error(
    id: Option<serde_json::Value>,
    code: i32,
    message: &str,
    data: Option<serde_json::Value>,
) {
    let response = JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.to_string(),
            data,
        }),
        id,
    };
    if let Ok(serialized) = serde_json::to_string(&response) {
        println!("{}", serialized);
    }
}

fn make_mcp_success_result(text: &str) -> serde_json::Value {
    serde_json::json!({
        "content": [
            {
                "type": "text",
                "text": text
            }
        ],
        "isError": false
    })
}

fn make_mcp_error_result(error_msg: &str) -> serde_json::Value {
    serde_json::json!({
        "content": [
            {
                "type": "text",
                "text": error_msg
            }
        ],
        "isError": true
    })
}

// ============================================================================
// Dry Run Helper
// ============================================================================

fn build_dry_run_report(
    stages: &[local_ci_core::Stage],
    cache: &HashMap<String, String>,
    stage_hashes: &HashMap<String, String>,
    source_hash: &str,
    no_cache: bool,
    remote: Option<DryRunRemote>,
    cwd: &Path,
) -> DryRunReport {
    let mut dry_run_stages = Vec::new();
    for stage in stages {
        let cmd_str = match &stage.command {
            Some(cmd) => cmd.join(" "),
            None => String::new(),
        };

        let hash = stage_hashes
            .get(&stage.name)
            .map(|s| s.as_str())
            .unwrap_or(source_hash);

        let (would_run, reason) = if !stage.enabled {
            (false, "disabled")
        } else if no_cache {
            (true, "no_cache_flag")
        } else if local_ci_cache::cache_hit(cache, stage, hash) {
            (false, "cached")
        } else {
            (true, "hash_changed")
        };

        dry_run_stages.push(DryRunStage {
            name: stage.name.clone(),
            command: cmd_str,
            would_run,
            reason: reason.to_string(),
        });
    }

    DryRunReport {
        workspace: cwd.to_string_lossy().to_string(),
        source_hash: source_hash.to_string(),
        remote,
        stages: dry_run_stages,
    }
}

fn print_dry_run_human(report: &DryRunReport) {
    println!("📋 Dry-run report for: {}", report.workspace);
    println!("   Source hash: {}", report.source_hash);
    if let Some(r) = &report.remote {
        let mut line = format!(
            "   Remote: {} (session={}, work_dir={})",
            r.host, r.session, r.work_dir
        );
        if let Some(p) = &r.host_preset {
            line.push_str(&format!(" [preset={}]", p));
        }
        println!("{}", line);
    }
    println!();
    println!("Stages:");
    for stage in &report.stages {
        let status = if stage.would_run { "✓" } else { "✗" };
        println!("  {} {}", status, stage.name);
        println!("      Command: {}", stage.command);
        println!("      Reason: {}", stage.reason);
    }
    let would_run_count = report.stages.iter().filter(|s| s.would_run).count();
    println!(
        "\n📊 Summary: {}/{} stages would run",
        would_run_count,
        report.stages.len()
    );
}

fn print_dry_run_json(report: &DryRunReport) {
    if let Ok(serialized) = serde_json::to_string_pretty(report) {
        println!("{}", serialized);
    }
}

fn get_skip_dirs(repo_dir: &Path) -> Vec<String> {
    if let Ok(config) = local_ci_detect::load_config(repo_dir, false) {
        config.cache.skip_dirs.clone()
    } else {
        vec![
            "target".to_string(),
            ".git".to_string(),
            "node_modules".to_string(),
        ]
    }
}

fn cmd_check(cwd: &Path, json: bool) {
    let linter_report = local_ci_checks::lint_config_in_workspace(cwd);

    let skip_dirs = get_skip_dirs(cwd);

    let secrets_report = local_ci_checks::scan_workspace_secrets(cwd, &skip_dirs);
    let vulnerability_report = local_ci_checks::scan_workspace_vulnerabilities(cwd);

    if json {
        let output = serde_json::json!({
            "config_lint": linter_report,
            "secrets": secrets_report,
            "vulnerabilities": vulnerability_report,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&output).unwrap_or_default()
        );
    } else {
        println!("🔍 Running local-ci-checks...\n");

        println!("📋 1. Config Linter Report:");
        if linter_report.warnings.is_empty() {
            println!("  ✅ No config lint issues found.");
        } else {
            for warning in &linter_report.warnings {
                let sev = match warning.severity {
                    local_ci_checks::LintSeverity::Error => "❌ Error",
                    local_ci_checks::LintSeverity::Warning => "⚠️  Warning",
                };
                let line_str = match warning.line_number {
                    Some(l) => format!(" (line {})", l),
                    None => String::new(),
                };
                println!(
                    "  {} [{}]{}: {}",
                    sev, warning.rule_id, line_str, warning.message
                );
                println!("     Remediation: {}", warning.remediation);
            }
        }
        println!();

        println!("🔑 2. Secrets Scanner Report:");
        if secrets_report.findings.is_empty() {
            println!("  ✅ No secrets found in workspace.");
        } else {
            for finding in &secrets_report.findings {
                println!(
                    "  ⚠️  Found potential {} in {} (line {})",
                    finding.secret_type, finding.file_path, finding.line_number
                );
                println!(
                    "     Entropy: {:.2}, Context: {}",
                    finding.entropy,
                    finding.line_content.trim()
                );
            }
        }
        println!();

        println!("🛡️  3. Vulnerability Report:");
        if vulnerability_report.vulnerabilities.is_empty() {
            println!("  ✅ No known vulnerable packages found in workspace.");
        } else {
            for vuln in &vulnerability_report.vulnerabilities {
                let sev = match vuln.severity {
                    local_ci_checks::Severity::Low => "Low",
                    local_ci_checks::Severity::Medium => "Medium",
                    local_ci_checks::Severity::High => "High",
                    local_ci_checks::Severity::Critical => "Critical",
                };
                println!(
                    "  ❌ [{}] Found vulnerable package: {} v{} (Severity: {}) in {}",
                    vuln.cve_id, vuln.package_name, vuln.current_version, sev, vuln.file_path
                );
                println!("     Description: {}", vuln.description);
                println!("     Remediation: {}", vuln.remediation);
            }
        }
    }

    let has_failures = linter_report.has_errors
        || !secrets_report.findings.is_empty()
        || !vulnerability_report.vulnerabilities.is_empty();
    if has_failures {
        std::process::exit(1);
    }
}

fn extract_sha_from_event(event_path: &str) -> Result<String, String> {
    let content = std::fs::read_to_string(event_path)
        .map_err(|e| format!("Failed to read event file '{}': {}", event_path, e))?;
    let val: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("Failed to parse event JSON: {}", e))?;

    if let Some(sha) = val
        .pointer("/pull_request/head/sha")
        .and_then(|v| v.as_str())
    {
        return Ok(sha.to_string());
    }
    if let Some(sha) = val.get("after").and_then(|v| v.as_str()) {
        return Ok(sha.to_string());
    }
    if let Some(sha) = val.pointer("/head_commit/id").and_then(|v| v.as_str()) {
        return Ok(sha.to_string());
    }
    Err("Could not find commit SHA in event payload (checked /pull_request/head/sha, /after, and /head_commit/id)".to_string())
}

fn get_git_sha(repo_path: &Path) -> String {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_path)
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            if let Ok(sha) = String::from_utf8(out.stdout) {
                return sha.trim().to_string();
            }
        }
    }
    "0000000000000000000000000000000000000000".to_string()
}

fn cmd_fft(cwd: &Path, cli: &Cli) {
    if cli.args.len() < 2 || cli.args[1] != "run" {
        local_ci_report::errorf!(
            "Usage: local-ci fft run --event <event_json> --repo <repo_path> --out <output_json>\n"
        );
        std::process::exit(1);
    }

    let repo_dir = match &cli.repo {
        Some(r) => Path::new(r).to_path_buf(),
        None => cwd.to_path_buf(),
    };

    if !repo_dir.exists() {
        local_ci_report::errorf!(
            "Error: Repo directory '{}' does not exist.\n",
            repo_dir.display()
        );
        std::process::exit(1);
    }

    let head_sha = match &cli.event {
        Some(event_path) => match extract_sha_from_event(event_path) {
            Ok(sha) => sha,
            Err(e) => {
                local_ci_report::errorf!("Error: {}\n", e);
                std::process::exit(1);
            }
        },
        None => get_git_sha(&repo_dir),
    };

    let linter_report = local_ci_checks::lint_config_in_workspace(&repo_dir);

    let skip_dirs = get_skip_dirs(&repo_dir);

    let secrets_report = local_ci_checks::scan_workspace_secrets(&repo_dir, &skip_dirs);
    let vulnerability_report = local_ci_checks::scan_workspace_vulnerabilities(&repo_dir);

    let now = chrono::Utc::now();
    let started_at = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let mut payload =
        local_ci_checks::CheckRunPayload::new("FFT Checks", &head_sha, "completed", &started_at);

    let mut annotations = Vec::new();
    annotations.extend(local_ci_checks::map_vulnerabilities_to_annotations(
        &vulnerability_report,
    ));
    annotations.extend(local_ci_checks::map_linter_to_annotations(&linter_report));
    annotations.extend(local_ci_checks::map_secrets_to_annotations(&secrets_report));

    let has_failures = linter_report.has_errors
        || !secrets_report.findings.is_empty()
        || !vulnerability_report.vulnerabilities.is_empty();

    let conclusion = if has_failures { "failure" } else { "success" };
    let summary = format!(
        "FFT Run completed with conclusion: {}\n- {} vulnerabilities\n- {} config lint warnings\n- {} secrets detected",
        conclusion,
        vulnerability_report.vulnerabilities.len(),
        linter_report.warnings.len(),
        secrets_report.findings.len()
    );

    let completed_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    payload.complete(
        conclusion,
        &completed_at,
        "Fast-Free-Testing (FFT) Security & Lint Check",
        &summary,
        None,
    );

    if let Some(ref mut out) = payload.output {
        out.annotations = annotations;
    }

    let workspace_name = repo_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project");

    let mut s3_plan = local_ci_checks::S3UploadPlan::new(
        workspace_name,
        &head_sha,
        "lornu-fft-artifacts",
        "us-east-1",
    );

    let json_str = if let Some(out_path) = &cli.out {
        let placeholder_hash = "0000000000000000000000000000000000000000000000000000000000000000";
        s3_plan.add_item(out_path, "application/json", placeholder_hash, 0);

        let placeholder_output = serde_json::json!({
            "github_checks": payload,
            "s3_upload_plan": s3_plan,
        });

        let placeholder_json_str =
            serde_json::to_string_pretty(&placeholder_output).unwrap_or_default();

        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(placeholder_json_str.as_bytes());
        let computed_hash = format!("{:x}", hasher.finalize());
        let computed_size = placeholder_json_str.len();

        let mut actual_s3_plan = local_ci_checks::S3UploadPlan::new(
            workspace_name,
            &head_sha,
            "lornu-fft-artifacts",
            "us-east-1",
        );
        actual_s3_plan.add_item(
            out_path,
            "application/json",
            &computed_hash,
            computed_size as u64,
        );

        let final_output = serde_json::json!({
            "github_checks": payload,
            "s3_upload_plan": actual_s3_plan,
        });

        serde_json::to_string_pretty(&final_output).unwrap_or_default()
    } else {
        let final_output = serde_json::json!({
            "github_checks": payload,
            "s3_upload_plan": s3_plan,
        });
        serde_json::to_string_pretty(&final_output).unwrap_or_default()
    };

    if let Some(out_path) = &cli.out {
        if let Err(e) = std::fs::write(out_path, &json_str) {
            local_ci_report::errorf!("Failed to write FFT results to '{}': {}\n", out_path, e);
            std::process::exit(1);
        }
        if cli.verbose {
            println!("{}", json_str);
        } else {
            local_ci_report::successf!("✅ FFT Run complete. Results saved to '{}'.\n", out_path);
        }
    } else {
        println!("{}", json_str);
    }

    if has_failures {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_sha_from_event() {
        let temp_file = "test_event_temp.json";

        // 1. Test missing file
        let res = extract_sha_from_event("non_existent_file_xyz.json");
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Failed to read event file"));

        // 2. Test invalid JSON
        std::fs::write(temp_file, "invalid-json-content").unwrap();
        let res = extract_sha_from_event(temp_file);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Failed to parse event JSON"));

        // 3. Test pull_request head sha
        let pr_json = r#"{
            "pull_request": {
                "head": {
                    "sha": "abcdef1234567890abcdef1234567890abcdef12"
                }
            }
        }"#;
        std::fs::write(temp_file, pr_json).unwrap();
        let res = extract_sha_from_event(temp_file);
        assert_eq!(res.unwrap(), "abcdef1234567890abcdef1234567890abcdef12");

        // 4. Test after field
        let after_json = r#"{
            "after": "fedcba0987654321fedcba0987654321fedcba09"
        }"#;
        std::fs::write(temp_file, after_json).unwrap();
        let res = extract_sha_from_event(temp_file);
        assert_eq!(res.unwrap(), "fedcba0987654321fedcba0987654321fedcba09");

        // 5. Test head_commit id
        let head_commit_json = r#"{
            "head_commit": {
                "id": "1111222233334444555566667777888899990000"
            }
        }"#;
        std::fs::write(temp_file, head_commit_json).unwrap();
        let res = extract_sha_from_event(temp_file);
        assert_eq!(res.unwrap(), "1111222233334444555566667777888899990000");

        // 6. Test missing SHA fields
        let empty_json = r#"{"foo": "bar"}"#;
        std::fs::write(temp_file, empty_json).unwrap();
        let res = extract_sha_from_event(temp_file);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Could not find commit SHA"));

        // Clean up
        let _ = std::fs::remove_file(temp_file);
    }
}
