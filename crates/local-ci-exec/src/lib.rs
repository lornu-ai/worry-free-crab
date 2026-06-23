use local_ci_core::Stage;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Result {
    pub name: String,
    pub command: String,
    pub status: String, // "pass", "fail", "skip"
    pub duration: Duration,
    pub output: String,
    pub cache_hit: bool,
    pub error: Option<String>,
}

pub fn run_with_timeout(
    mut cmd: Command,
    timeout: Duration,
    cwd: &Path,
) -> std::io::Result<(std::process::ExitStatus, String)> {
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.current_dir(cwd);

    let mut child = cmd.spawn()?;
    let child_id = child.id();

    // Read stdout & stderr concurrently to avoid deadlock
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();

    let (stdout_tx, stdout_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut out = String::new();
        use std::io::Read;
        let _ = stdout.read_to_string(&mut out);
        let _ = stdout_tx.send(out);
    });

    let (stderr_tx, stderr_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut err = String::new();
        use std::io::Read;
        let _ = stderr.read_to_string(&mut err);
        let _ = stderr_tx.send(err);
    });

    let (tx, rx) = mpsc::channel();
    let mut child_handle = child;
    thread::spawn(move || {
        let res = child_handle.wait();
        let _ = tx.send(res);
    });

    match rx.recv_timeout(timeout) {
        Ok(res) => {
            let status = res?;
            let mut out = stdout_rx.recv().unwrap_or_default();
            let err = stderr_rx.recv().unwrap_or_default();
            out.push_str(&err);
            Ok((status, out))
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // Kill the child process group or the process
            #[cfg(unix)]
            {
                // SAFETY: We are calling the `libc::kill` FFI function with a negative PID
                // `-(child_id as libc::pid_t)` to send `SIGKILL` to the entire process group.
                // This is safe because:
                // 1. The child process group was spawned by our own process and we have a valid PID.
                // 2. The PID is cast to `libc::pid_t` which is safe as process IDs fit within typical PID limits.
                // 3. Any potential error is benign, and we gracefully fall back to killing the single process directly.
                unsafe {
                    libc::kill(-(child_id as libc::pid_t), libc::SIGKILL);
                }
                // Fallback direct kill
                let _ = std::process::Command::new("kill")
                    .args(["-9", &child_id.to_string()])
                    .output();
            }
            #[cfg(windows)]
            {
                let _ = std::process::Command::new("taskkill")
                    .args(["/F", "/PID", &child_id.to_string()])
                    .output();
            }

            let mut out = stdout_rx.recv().unwrap_or_default();
            let err = stderr_rx.recv().unwrap_or_default();
            out.push_str(&err);
            out.push_str("\n[local-ci] Error: Command timed out\n");

            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Command execution timed out",
            ))
        }
        Err(e) => Err(std::io::Error::other(e.to_string())),
    }
}

pub fn execute_single_stage(
    stage: &Stage,
    cwd: &Path,
    no_cache: bool,
    cache: &HashMap<String, String>,
    stage_hash: &str,
) -> Result {
    let start = Instant::now();

    if !no_cache && local_ci_cache::cache_hit(cache, stage, stage_hash) {
        return Result {
            name: stage.name.clone(),
            command: "".to_string(),
            status: "pass".to_string(),
            duration: Duration::ZERO,
            output: "".to_string(),
            cache_hit: true,
            error: None,
        };
    }

    let cmd_parts = match &stage.command {
        Some(parts) if !parts.is_empty() => parts,
        _ => {
            return Result {
                name: stage.name.clone(),
                command: "".to_string(),
                status: "fail".to_string(),
                duration: start.elapsed(),
                output: "".to_string(),
                cache_hit: false,
                error: Some("no command defined".to_string()),
            }
        }
    };

    let mut cmd = Command::new(&cmd_parts[0]);
    if cmd_parts.len() > 1 {
        cmd.args(&cmd_parts[1..]);
    }

    let timeout_secs = if stage.timeout > 0 {
        stage.timeout as u64
    } else {
        30
    };
    let timeout = Duration::from_secs(timeout_secs);

    match run_with_timeout(cmd, timeout, cwd) {
        Ok((status, output)) => {
            if status.success() {
                Result {
                    name: stage.name.clone(),
                    command: cmd_parts.join(" "),
                    status: "pass".to_string(),
                    duration: start.elapsed(),
                    output,
                    cache_hit: false,
                    error: None,
                }
            } else {
                let code = status.code().unwrap_or(-1);
                Result {
                    name: stage.name.clone(),
                    command: cmd_parts.join(" "),
                    status: "fail".to_string(),
                    duration: start.elapsed(),
                    output,
                    cache_hit: false,
                    error: Some(format!("exit code: {}", code)),
                }
            }
        }
        Err(e) => Result {
            name: stage.name.clone(),
            command: cmd_parts.join(" "),
            status: "fail".to_string(),
            duration: start.elapsed(),
            output: e.to_string(),
            cache_hit: false,
            error: Some(e.to_string()),
        },
    }
}

struct SimpleSemaphore {
    count: Mutex<usize>,
    cvar: std::sync::Condvar,
}

impl SimpleSemaphore {
    fn new(limit: usize) -> Self {
        Self {
            count: Mutex::new(limit),
            cvar: std::sync::Condvar::new(),
        }
    }

    fn acquire(&self) {
        let mut count = self.count.lock().unwrap();
        while *count == 0 {
            count = self.cvar.wait(count).unwrap();
        }
        *count -= 1;
    }

    fn release(&self) {
        let mut count = self.count.lock().unwrap();
        *count += 1;
        self.cvar.notify_one();
    }
}

pub struct ParallelRunner {
    pub stages: Vec<Stage>,
    pub concurrency: usize,
    pub cwd: PathBuf,
    pub no_cache: bool,
    pub cache: Arc<Mutex<HashMap<String, String>>>,
    pub source_hash: String,
    pub stage_hashes: HashMap<String, String>,
    pub verbose: bool,
    pub json: bool,
    pub fail_fast: bool,
}

impl ParallelRunner {
    pub fn run(&self) -> Vec<Result> {
        let concurrency = if self.concurrency == 0 {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
        } else {
            self.concurrency
        };

        let sem = Arc::new(SimpleSemaphore::new(concurrency));
        let completed = Arc::new(Mutex::new(HashMap::<String, bool>::new()));
        let failed = Arc::new(AtomicBool::new(false));

        let (result_tx, result_rx) = mpsc::channel();
        let mut handles = Vec::new();

        let stage_deps: HashMap<String, Vec<String>> = self
            .stages
            .iter()
            .map(|s| (s.name.clone(), s.depends_on.clone()))
            .collect();

        let stage_index: HashMap<String, usize> = self
            .stages
            .iter()
            .enumerate()
            .map(|(i, s)| (s.name.clone(), i))
            .collect();

        for stage in &self.stages {
            let s = stage.clone();
            let sem = Arc::clone(&sem);
            let completed = Arc::clone(&completed);
            let failed = Arc::clone(&failed);
            let result_tx = result_tx.clone();

            let stage_deps = stage_deps.clone();
            let stage_index = stage_index.clone();
            let cwd = self.cwd.clone();
            let no_cache = self.no_cache;
            let cache = Arc::clone(&self.cache);
            let fail_fast = self.fail_fast;
            let stages_list = self.stages.clone();

            // Resolve stage hash
            let stage_hash = self
                .stage_hashes
                .get(&s.name)
                .cloned()
                .unwrap_or_else(|| self.source_hash.clone());

            let handle = thread::spawn(move || {
                // Wait loop for dependencies
                loop {
                    let mut ready = true;
                    {
                        let comp_map = completed.lock().unwrap();
                        if let Some(deps) = stage_deps.get(&s.name) {
                            for dep in deps {
                                if !comp_map.get(dep).copied().unwrap_or(false) {
                                    ready = false;
                                    break;
                                }
                            }
                        }

                        if ready && fail_fast {
                            let my_idx = stage_index[&s.name];
                            for earlier in &stages_list {
                                if stage_index[&earlier.name] >= my_idx {
                                    break;
                                }
                                if !comp_map.get(&earlier.name).copied().unwrap_or(false) {
                                    ready = false;
                                    break;
                                }
                            }
                        }
                    }

                    if ready {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }

                let skip = || {
                    let _ = result_tx.send(Result {
                        name: s.name.clone(),
                        command: "".to_string(),
                        status: "skip".to_string(),
                        duration: Duration::ZERO,
                        output: "".to_string(),
                        cache_hit: false,
                        error: None,
                    });
                    let mut comp_map = completed.lock().unwrap();
                    comp_map.insert(s.name.clone(), true);
                };

                if fail_fast && failed.load(Ordering::SeqCst) {
                    skip();
                    return;
                }

                // Acquire semaphore permit
                sem.acquire();

                if fail_fast && failed.load(Ordering::SeqCst) {
                    sem.release();
                    skip();
                    return;
                }

                let local_cache = {
                    let map = cache.lock().unwrap();
                    map.clone()
                };

                let result = execute_single_stage(&s, &cwd, no_cache, &local_cache, &stage_hash);

                if result.status != "pass" {
                    failed.store(true, Ordering::SeqCst);
                } else if !result.cache_hit {
                    let mut map = cache.lock().unwrap();
                    map.insert(
                        s.name.clone(),
                        local_ci_cache::cache_key_for_stage(&s, &stage_hash),
                    );
                }

                let _ = result_tx.send(result);

                {
                    let mut comp_map = completed.lock().unwrap();
                    comp_map.insert(s.name.clone(), true);
                }

                // Release semaphore permit
                sem.release();
            });
            handles.push(handle);
        }

        // Wait for all threads to finish
        for h in handles {
            let _ = h.join();
        }

        // Drain results
        drop(result_tx); // close sender
        let mut results = Vec::new();
        while let Ok(r) = result_rx.recv() {
            results.push(r);
        }

        // Return results ordered by stage configuration to match Go's deterministic output
        let mut result_map = HashMap::new();
        for r in results {
            result_map.insert(r.name.clone(), r);
        }

        let mut sorted_results = Vec::new();
        for s in &self.stages {
            if let Some(r) = result_map.remove(&s.name) {
                sorted_results.push(r);
            }
        }

        sorted_results
    }
}

#[derive(Debug, Clone)]
pub struct RemoteExecutor {
    pub host: String,
    pub session: String,
    pub work_dir: String,
    pub timeout: Duration,
    pub verbose: bool,
}

fn escape_shell_arg(arg: &str) -> String {
    if !arg.chars().any(|c| " \t\n'\"\\$`".contains(c)) {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', "'\\''"))
}

fn escape_for_tmux(cmd: &str) -> String {
    cmd.replace('\'', "'\\''")
}

fn join_shell_command(parts: &[String]) -> String {
    if parts.is_empty() {
        return String::new();
    }
    let quoted: Vec<String> = parts.iter().map(|p| escape_shell_arg(p)).collect();
    quoted.join(" ")
}

fn build_remote_stage_command(work_dir: &str, cmd: &[String], sentinel_file: &str) -> String {
    format!(
        "cd {} && {}; echo $? > {}",
        escape_shell_arg(work_dir),
        join_shell_command(cmd),
        sentinel_file
    )
}

fn benign_ssh_failure(cmd: &str, output: &str) -> bool {
    let trimmed = cmd.trim();
    if !trimmed.starts_with("cat ") {
        return false;
    }
    output.contains("No such file") || output.contains("cannot open") || output.trim().is_empty()
}

impl RemoteExecutor {
    pub fn new(
        host: String,
        session: String,
        work_dir: String,
        timeout: Duration,
        verbose: bool,
    ) -> Self {
        let session = if session.is_empty() {
            "onion".to_string()
        } else {
            session
        };
        let timeout = if timeout.is_zero() {
            Duration::from_secs(30)
        } else {
            timeout
        };
        Self {
            host,
            session,
            work_dir,
            timeout,
            verbose,
        }
    }

    pub fn test_ssh_connection(&self) -> std::io::Result<()> {
        let _ = self.ssh_exec_with_output("echo 'SSH connection OK'", self.timeout)?;
        Ok(())
    }

    pub fn ensure_remote_session(&self) -> std::io::Result<()> {
        let cmd = format!(
            "tmux new-session -d -s {} -c {} 'sleep 999999' 2>/dev/null || true",
            escape_shell_arg(&self.session),
            escape_shell_arg(&self.work_dir)
        );
        let _ = self.ssh_exec_with_output(&cmd, self.timeout)?;
        Ok(())
    }

    pub fn kill_remote_session(&self) -> std::io::Result<()> {
        let cmd = format!(
            "tmux kill-session -t {} 2>/dev/null || true",
            escape_shell_arg(&self.session)
        );
        let _ = self.ssh_exec_with_output(&cmd, self.timeout)?;
        Ok(())
    }

    pub fn sync_workspace(&self, local_dir: &str, skip_dirs: &[String]) -> std::io::Result<()> {
        // Ensure remote work directory exists
        let mkdir_cmd = format!("mkdir -p {}", escape_shell_arg(&self.work_dir));
        let _ = self.ssh_exec_with_output(&mkdir_cmd, self.timeout)?;

        let mut rsync_args = vec!["-az".to_string(), "--delete".to_string()];
        rsync_args.push("--exclude".to_string());
        rsync_args.push(".git".to_string());
        rsync_args.push("--exclude".to_string());
        rsync_args.push(".local-ci-cache".to_string());

        for dir in skip_dirs {
            if !dir.is_empty() && dir != ".git" {
                rsync_args.push("--exclude".to_string());
                rsync_args.push(dir.clone());
            }
        }

        let mut src = local_dir.to_string();
        if !src.ends_with('/') {
            src.push('/');
        }
        let dest = format!("{}:{}", self.host, self.work_dir);
        rsync_args.push(src);
        rsync_args.push(dest);

        if self.verbose {
            println!(
                "Syncing workspace to remote: rsync {}",
                rsync_args.join(" ")
            );
        }

        let mut cmd = Command::new("rsync");
        cmd.args(&rsync_args);

        // Run rsync with a timeout of 2 minutes
        let (status, output) = run_with_timeout(cmd, Duration::from_secs(120), Path::new("."))?;
        if !status.success() {
            return Err(std::io::Error::other(format!(
                "rsync failed: {} (output: {})",
                status, output
            )));
        }

        Ok(())
    }

    pub fn execute_stage(&self, stage: &Stage) -> Result {
        let start = Instant::now();
        let cmd_parts = match &stage.command {
            Some(parts) if !parts.is_empty() => parts,
            _ => {
                return Result {
                    name: stage.name.clone(),
                    command: String::new(),
                    status: "fail".to_string(),
                    duration: start.elapsed(),
                    output: String::new(),
                    cache_hit: false,
                    error: Some("no command defined".to_string()),
                }
            }
        };

        // Use high-resolution timing for unique sentinel filename
        let sentinel_file = format!(
            "/tmp/kc_exit_{}_{}",
            stage.name,
            Instant::now().duration_since(start).as_nanos()
        );
        let remote_cmd = build_remote_stage_command(&self.work_dir, cmd_parts, &sentinel_file);

        let stage_timeout_secs = if stage.timeout > 0 {
            stage.timeout as u64
        } else {
            600
        };
        let stage_timeout = Duration::from_secs(stage_timeout_secs);

        if let Err(e) = self.send_to_session(&remote_cmd, stage_timeout) {
            return Result {
                name: stage.name.clone(),
                command: cmd_parts.join(" "),
                status: "fail".to_string(),
                duration: start.elapsed(),
                output: format!("Remote execution failed: {}", e),
                cache_hit: false,
                error: Some(e.to_string()),
            };
        }

        let exit_code = match self.poll_exit_code(&sentinel_file, stage_timeout) {
            Ok(code) => code,
            Err(e) => {
                return Result {
                    name: stage.name.clone(),
                    command: cmd_parts.join(" "),
                    status: "fail".to_string(),
                    duration: start.elapsed(),
                    output: format!("Failed to get exit code: {}", e),
                    cache_hit: false,
                    error: Some(e.to_string()),
                };
            }
        };

        let output = match self.capture_session_output(stage_timeout) {
            Ok(out) => out,
            Err(e) => {
                return Result {
                    name: stage.name.clone(),
                    command: cmd_parts.join(" "),
                    status: "fail".to_string(),
                    duration: start.elapsed(),
                    output: format!("Failed to capture output: {}", e),
                    cache_hit: false,
                    error: Some(e.to_string()),
                };
            }
        };

        let _ = self.cleanup_sentinel(&sentinel_file);

        let status = if exit_code == 0 {
            "pass".to_string()
        } else {
            "fail".to_string()
        };
        let error = if exit_code == 0 {
            None
        } else {
            Some(format!("exit code {}", exit_code))
        };

        Result {
            name: stage.name.clone(),
            command: cmd_parts.join(" "),
            status,
            duration: start.elapsed(),
            output,
            cache_hit: false,
            error,
        }
    }

    fn send_to_session(&self, cmd: &str, timeout: Duration) -> std::io::Result<()> {
        let init_cmd = format!(
            "tmux new-session -d -s {} -c {} 'sleep 999999' 2>/dev/null; true",
            escape_shell_arg(&self.session),
            escape_shell_arg(&self.work_dir)
        );

        if let Err(e) = self.ssh_exec_with_output(&init_cmd, timeout) {
            if self.verbose {
                eprintln!(
                    "Warning: could not initialize session: {} (proceeding anyway)",
                    e
                );
            }
        }

        let send_cmd = format!(
            "tmux send-keys -t {} '{}' Enter",
            escape_shell_arg(&self.session),
            escape_for_tmux(cmd)
        );

        self.ssh_exec_with_output(&send_cmd, timeout)?;
        Ok(())
    }

    fn capture_session_output(&self, timeout: Duration) -> std::io::Result<String> {
        let capture_cmd = format!(
            "tmux capture-pane -t {} -p",
            escape_shell_arg(&self.session)
        );
        self.ssh_exec_with_output(&capture_cmd, timeout)
    }

    fn poll_exit_code(&self, sentinel_file: &str, timeout: Duration) -> std::io::Result<i32> {
        let start = Instant::now();
        let cat_cmd = format!("cat {} 2>/dev/null", sentinel_file);

        while start.elapsed() < timeout {
            if let Ok(output) = self.ssh_exec_with_output(&cat_cmd, Duration::from_secs(5)) {
                let trimmed = output.trim();
                if !trimmed.is_empty() {
                    if let Ok(code) = trimmed.parse::<i32>() {
                        return Ok(code);
                    }
                }
            }
            thread::sleep(Duration::from_millis(100));
        }

        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("timeout waiting for exit code from {}", sentinel_file),
        ))
    }

    fn cleanup_sentinel(&self, sentinel_file: &str) -> std::io::Result<()> {
        let cleanup_cmd = format!("rm -f {}", sentinel_file);
        let _ = self.ssh_exec_with_output(&cleanup_cmd, self.timeout)?;
        Ok(())
    }

    fn ssh_exec_with_output(&self, cmd_str: &str, timeout: Duration) -> std::io::Result<String> {
        let timeout_sec = timeout.as_secs();
        let timeout_sec = if timeout_sec < 1 { 10 } else { timeout_sec };

        let mut command = Command::new("ssh");
        command.args([
            "-o",
            &format!("ConnectTimeout={}", timeout_sec),
            &self.host,
            cmd_str,
        ]);

        let (status, output) = run_with_timeout(command, timeout, Path::new("."))?;

        if !status.success() {
            if benign_ssh_failure(cmd_str, &output) {
                return Ok(String::new());
            }
            return Err(std::io::Error::other(format!(
                "SSH command failed with status {} (output: {})",
                status, output
            )));
        }

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use local_ci_core::Stage;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[test]
    fn test_escape_shell_arg() {
        assert_eq!(escape_shell_arg("hello"), "hello");
        assert_eq!(escape_shell_arg("hello world"), "'hello world'");
        assert_eq!(escape_shell_arg("don't"), "'don'\\''t'");
    }

    #[test]
    fn test_escape_for_tmux() {
        assert_eq!(escape_for_tmux("echo 'hello'"), "echo '\\''hello'\\''");
    }

    #[test]
    fn test_join_shell_command() {
        assert_eq!(join_shell_command(&[]), "");
        assert_eq!(
            join_shell_command(&["echo".to_string(), "hello world".to_string()]),
            "echo 'hello world'"
        );
    }

    #[test]
    fn test_build_remote_stage_command() {
        let cmd = vec!["cargo".to_string(), "test".to_string()];
        let remote_cmd = build_remote_stage_command("/tmp/work", &cmd, "/tmp/sentinel");
        assert_eq!(
            remote_cmd,
            "cd /tmp/work && cargo test; echo $? > /tmp/sentinel"
        );
    }

    #[test]
    fn test_run_with_timeout_success() {
        let mut cmd = Command::new("echo");
        cmd.arg("hello-test");
        let (status, output) =
            run_with_timeout(cmd, Duration::from_secs(5), Path::new(".")).unwrap();
        assert!(status.success());
        assert!(output.contains("hello-test"));
    }

    #[test]
    fn test_run_with_timeout_failure() {
        let cmd = Command::new("false");
        let (status, _output) =
            run_with_timeout(cmd, Duration::from_secs(5), Path::new(".")).unwrap();
        assert!(!status.success());
    }

    #[test]
    fn test_run_with_timeout_timeout() {
        let mut cmd = Command::new("sleep");
        cmd.arg("10");
        let res = run_with_timeout(cmd, Duration::from_millis(100), Path::new("."));
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
    }

    #[test]
    fn test_execute_single_stage_success() {
        let stage = Stage {
            name: "test_stage".to_string(),
            command: Some(vec!["echo".to_string(), "test-ok".to_string()]),
            ..Default::default()
        };

        let cache = HashMap::new();
        let res = execute_single_stage(&stage, Path::new("."), false, &cache, "some_hash");
        assert_eq!(res.status, "pass");
        assert!(!res.cache_hit);
        assert!(res.output.contains("test-ok"));
    }

    #[test]
    fn test_execute_single_stage_cache_hit() {
        let stage = Stage {
            name: "test_stage".to_string(),
            command: Some(vec!["echo".to_string(), "test-ok".to_string()]),
            ..Default::default()
        };

        let mut cache = HashMap::new();
        cache.insert(
            "test_stage".to_string(),
            "some_hash|echo test-ok".to_string(),
        );

        let res = execute_single_stage(&stage, Path::new("."), false, &cache, "some_hash");
        assert_eq!(res.status, "pass");
        assert!(res.cache_hit);
        assert_eq!(res.output, "");
    }

    #[test]
    fn test_execute_single_stage_missing_command() {
        let stage = Stage {
            name: "test_stage".to_string(),
            command: None,
            ..Default::default()
        };

        let cache = HashMap::new();
        let res = execute_single_stage(&stage, Path::new("."), false, &cache, "some_hash");
        assert_eq!(res.status, "fail");
        assert_eq!(res.error.unwrap(), "no command defined");
    }

    #[test]
    fn test_parallel_runner_basic() {
        let stage_a = Stage {
            name: "A".to_string(),
            command: Some(vec!["echo".to_string(), "A_done".to_string()]),
            ..Default::default()
        };

        let stage_b = Stage {
            name: "B".to_string(),
            command: Some(vec!["echo".to_string(), "B_done".to_string()]),
            ..Default::default()
        };

        let runner = ParallelRunner {
            stages: vec![stage_a, stage_b],
            concurrency: 2,
            cwd: PathBuf::from("."),
            no_cache: true,
            cache: Arc::new(Mutex::new(HashMap::new())),
            source_hash: "hash_xyz".to_string(),
            stage_hashes: HashMap::new(),
            verbose: false,
            json: false,
            fail_fast: false,
        };

        let results = runner.run();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].name, "A");
        assert_eq!(results[0].status, "pass");
        assert_eq!(results[1].name, "B");
        assert_eq!(results[1].status, "pass");
    }

    #[test]
    fn test_parallel_runner_dependencies() {
        let stage_a = Stage {
            name: "A".to_string(),
            command: Some(vec!["echo".to_string(), "A_done".to_string()]),
            ..Default::default()
        };

        let stage_b = Stage {
            name: "B".to_string(),
            command: Some(vec!["echo".to_string(), "B_done".to_string()]),
            depends_on: vec!["A".to_string()],
            ..Default::default()
        };

        let runner = ParallelRunner {
            stages: vec![stage_a, stage_b],
            concurrency: 2,
            cwd: PathBuf::from("."),
            no_cache: true,
            cache: Arc::new(Mutex::new(HashMap::new())),
            source_hash: "hash_xyz".to_string(),
            stage_hashes: HashMap::new(),
            verbose: false,
            json: false,
            fail_fast: false,
        };

        let results = runner.run();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].name, "A");
        assert_eq!(results[0].status, "pass");
        assert_eq!(results[1].name, "B");
        assert_eq!(results[1].status, "pass");
    }

    #[test]
    fn test_parallel_runner_fail_fast() {
        let stage_a = Stage {
            name: "A".to_string(),
            command: Some(vec!["false".to_string()]),
            ..Default::default()
        };

        let stage_b = Stage {
            name: "B".to_string(),
            command: Some(vec!["echo".to_string(), "B_done".to_string()]),
            ..Default::default()
        };

        let runner = ParallelRunner {
            stages: vec![stage_a, stage_b],
            concurrency: 1, // force sequential to guarantee fail_fast behavior triggers before B runs
            cwd: PathBuf::from("."),
            no_cache: true,
            cache: Arc::new(Mutex::new(HashMap::new())),
            source_hash: "hash_xyz".to_string(),
            stage_hashes: HashMap::new(),
            verbose: false,
            json: false,
            fail_fast: true,
        };

        let results = runner.run();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].name, "A");
        assert_eq!(results[0].status, "fail");
        assert_eq!(results[1].name, "B");
        assert_eq!(results[1].status, "skip");
    }
}
