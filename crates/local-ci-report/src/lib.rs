use local_ci_exec::Result as ExecResult;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

pub static PRINT_JSON_MODE: AtomicBool = AtomicBool::new(false);

pub fn set_json_mode(json: bool) {
    PRINT_JSON_MODE.store(json, Ordering::SeqCst);
}

pub fn is_json_mode() -> bool {
    PRINT_JSON_MODE.load(Ordering::SeqCst)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultJSON {
    pub name: String,
    pub command: String,
    pub status: String,
    pub duration_ms: u64,
    pub cache_hit: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineReportJSON {
    pub schema_version: String,
    pub results: Vec<ResultJSON>,
    pub passed: usize,
    pub failed: usize,
    pub duration_ms: u64,
}

fn default_schema_version() -> String {
    "local-ci.result.v1".to_string()
}

impl ResultJSON {
    pub fn from_exec_result(r: &ExecResult) -> Self {
        Self {
            name: r.name.clone(),
            command: r.command.clone(),
            status: r.status.clone(),
            duration_ms: r.duration.as_millis() as u64,
            cache_hit: r.cache_hit,
            output: if r.output.trim().is_empty() {
                None
            } else {
                Some(r.output.trim().to_string())
            },
            error: r.error.clone(),
        }
    }
}

pub fn printf_impl(args: std::fmt::Arguments) {
    if is_json_mode() {
        let mut stderr = std::io::stderr().lock();
        let _ = stderr.write_all(format!("{}", args).as_bytes());
        let _ = stderr.flush();
    } else {
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(format!("{}", args).as_bytes());
        let _ = stdout.flush();
    }
}

pub fn successf_impl(args: std::fmt::Arguments) {
    let colored = format!("\x1b[32m{}\x1b[0m", args);
    if is_json_mode() {
        let mut stderr = std::io::stderr().lock();
        let _ = stderr.write_all(colored.as_bytes());
        let _ = stderr.flush();
    } else {
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(colored.as_bytes());
        let _ = stdout.flush();
    }
}

pub fn errorf_impl(args: std::fmt::Arguments) {
    let colored = format!("\x1b[31m{}\x1b[0m", args);
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(colored.as_bytes());
    let _ = stderr.flush();
}

pub fn warnf_impl(args: std::fmt::Arguments) {
    let colored = format!("\x1b[33m{}\x1b[0m", args);
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(colored.as_bytes());
    let _ = stderr.flush();
}

#[macro_export]
macro_rules! printf {
    ($($arg:tt)*) => {
        $crate::printf_impl(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! successf {
    ($($arg:tt)*) => {
        $crate::successf_impl(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! errorf {
    ($($arg:tt)*) => {
        $crate::errorf_impl(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! warnf {
    ($($arg:tt)*) => {
        $crate::warnf_impl(format_args!($($arg)*))
    };
}

pub fn print_report(results: &[ExecResult], total_duration: std::time::Duration, json: bool) {
    let mut pass_count = 0;
    let mut cached_count = 0;
    let mut executed_count = 0;
    let mut fail_count = 0;

    for r in results {
        if r.status == "pass" {
            pass_count += 1;
            if r.cache_hit {
                cached_count += 1;
            } else {
                executed_count += 1;
            }
        } else if r.status == "fail" {
            fail_count += 1;
        } else {
            // "skip" or other status
        }
    }

    if json {
        set_json_mode(true);
        let json_results: Vec<ResultJSON> =
            results.iter().map(ResultJSON::from_exec_result).collect();
        let report = PipelineReportJSON {
            schema_version: default_schema_version(),
            results: json_results,
            passed: pass_count,
            failed: fail_count,
            duration_ms: total_duration.as_millis() as u64,
        };
        // Print detailed human summary to stderr in JSON mode
        if fail_count == 0 {
            successf!(
                "✅ All {} stage(s) passed in {}ms\n",
                results.len(),
                total_duration.as_millis()
            );
        } else {
            errorf!("❌ {}/{} stages failed\n", fail_count, results.len());
        }
        printf!("\n📊 Summary:\n");
        printf!("  Total stages: {}\n", results.len());
        printf!("  Passed: {}\n", pass_count);
        if fail_count > 0 {
            printf!("  Failed: {}\n", fail_count);
        }
        if cached_count > 0 {
            let pct = (cached_count as f64) * 100.0 / (results.len() as f64);
            printf!("  Cached: {} ({:.0}%)\n", cached_count, pct);
        }
        if executed_count > 0 {
            printf!("  Executed: {}\n", executed_count);
        }
        printf!("  Total time: {}ms\n", total_duration.as_millis());

        // Print JSON report to stdout
        if let Ok(serialized) = serde_json::to_string_pretty(&report) {
            println!("{}", serialized);
        }
    } else {
        set_json_mode(false);
        if fail_count == 0 {
            successf!(
                "✅ All {} stage(s) passed in {}ms\n",
                results.len(),
                total_duration.as_millis()
            );
        } else {
            errorf!("❌ {}/{} stages failed\n", fail_count, results.len());
        }

        printf!("\n📊 Summary:\n");
        printf!("  Total stages: {}\n", results.len());
        printf!("  Passed: {}\n", pass_count);
        if fail_count > 0 {
            printf!("  Failed: {}\n", fail_count);
        }
        if cached_count > 0 {
            let pct = (cached_count as f64) * 100.0 / (results.len() as f64);
            printf!("  Cached: {} ({:.0}%)\n", cached_count, pct);
        }
        if executed_count > 0 {
            printf!("  Executed: {}\n", executed_count);
        }
        printf!("  Total time: {}ms\n", total_duration.as_millis());
    }
}
