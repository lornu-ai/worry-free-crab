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
                // Send SIGKILL to the process group
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
