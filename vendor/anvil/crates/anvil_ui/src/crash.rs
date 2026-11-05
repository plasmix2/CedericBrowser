//! Crash reporting and panic handling for Anvil applications.
//!
//! Catches panics across threads, writes a detailed crash log to disk, prints a clear
//! error banner to `stderr` (no silent exits), and optionally writes a machine-readable
//! crash report if the user or app opted in.
//!
//! Installed automatically by `anvil_ui::runner::run`, or manually via [`install`].

use std::backtrace::Backtrace;
use std::fs;
use std::io;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

static INSTALLED: AtomicBool = AtomicBool::new(false);
static REPORT_OPT_IN: AtomicBool = AtomicBool::new(false);

/// Sets whether crash reports (JSON formatted diagnostics) are generated on panic.
///
/// Can also be enabled by setting the environment variable `ANVIL_CRASH_REPORT=1`.
pub fn set_reporting_opt_in(opt_in: bool) {
    REPORT_OPT_IN.store(opt_in, Ordering::SeqCst);
}

/// Returns true if the user or configuration opted into crash reporting.
pub fn is_reporting_opt_in() -> bool {
    REPORT_OPT_IN.load(Ordering::SeqCst)
        || std::env::var_os("ANVIL_CRASH_REPORT").is_some_and(|v| v == "1")
}

/// Where crash logs and reports should be stored.
///
/// Looks for `ANVIL_CRASH_DIR` first; defaults to `./crashes`.
pub fn crash_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ANVIL_CRASH_DIR") {
        PathBuf::from(dir)
    } else {
        PathBuf::from("crashes")
    }
}

/// Formats a SystemTime into (ISO-8601 UTC string, filename-safe string).
fn format_utc_timestamp(time: SystemTime) -> (String, String) {
    let duration = time.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    let total_secs = duration.as_secs();
    let millis = duration.subsec_millis();

    let secs_per_day = 86400;
    let mut days = (total_secs / secs_per_day) as i64;
    let day_secs = (total_secs % secs_per_day) as u32;

    let hour = day_secs / 3600;
    let minute = (day_secs % 3600) / 60;
    let second = day_secs % 60;

    let mut year = 1970;
    loop {
        let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
        let days_in_year = if is_leap { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    let month_days = [
        31,
        if is_leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1;
    for &d in &month_days {
        if days < d {
            break;
        }
        days -= d;
        month += 1;
    }
    let day = days + 1;

    let iso = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year, month, day, hour, minute, second, millis
    );
    let file_safe = format!(
        "{:04}{:02}{:02}_{:02}{:02}{:02}_{:03}",
        year, month, day, hour, minute, second, millis
    );
    (iso, file_safe)
}

/// Escapes a string for valid JSON embedding.
fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                use std::fmt::Write;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// Information gathered from a panic.
#[derive(Debug, Clone)]
pub struct CrashRecord {
    pub timestamp_iso: String,
    pub timestamp_file: String,
    pub anvil_version: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
    pub pid: u32,
    pub thread_name: String,
    pub location: String,
    pub message: String,
    pub backtrace: String,
}

impl CrashRecord {
    /// Extracts crash details from a [`PanicHookInfo`].
    pub fn from_panic(info: &PanicHookInfo<'_>) -> Self {
        let (iso, file_safe) = format_utc_timestamp(SystemTime::now());

        let message = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "non-string panic payload".to_string()
        };

        let location = if let Some(loc) = info.location() {
            format!("{}:{}:{}", loc.file(), loc.line(), loc.column())
        } else {
            "unknown location".to_string()
        };

        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("unnamed").to_string();

        let bt = Backtrace::force_capture();
        let backtrace = format!("{bt}");

        Self {
            timestamp_iso: iso,
            timestamp_file: file_safe,
            anvil_version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            pid: std::process::id(),
            thread_name,
            location,
            message,
            backtrace,
        }
    }

    /// Formats the human-readable text log.
    pub fn format_text_log(&self) -> String {
        format!(
            "================================================================================\n\
             ANVIL ENGINE CRASH LOG\n\
             ================================================================================\n\
             Timestamp:     {}\n\
             Anvil Version: {}\n\
             Platform:      {} ({})\n\
             Process ID:    {}\n\
             Thread:        {}\n\
             \n\
             PANIC:\n\
             Location:      {}\n\
             Message:       {}\n\
             \n\
             BACKTRACE:\n\
             {}\n\
             ================================================================================\n",
            self.timestamp_iso,
            self.anvil_version,
            self.os,
            self.arch,
            self.pid,
            self.thread_name,
            self.location,
            self.message,
            self.backtrace,
        )
    }

    /// Formats the machine-readable JSON crash report.
    pub fn format_json_report(&self) -> String {
        format!(
            "{{\n\
             \x20 \"schema\": \"anvil_crash_v1\",\n\
             \x20 \"anvil_version\": \"{}\",\n\
             \x20 \"timestamp\": \"{}\",\n\
             \x20 \"os\": \"{}\",\n\
             \x20 \"arch\": \"{}\",\n\
             \x20 \"pid\": {},\n\
             \x20 \"thread\": \"{}\",\n\
             \x20 \"location\": \"{}\",\n\
             \x20 \"message\": \"{}\",\n\
             \x20 \"backtrace\": \"{}\"\n\
             }}\n",
            escape_json(self.anvil_version),
            escape_json(&self.timestamp_iso),
            escape_json(self.os),
            escape_json(self.arch),
            self.pid,
            escape_json(&self.thread_name),
            escape_json(&self.location),
            escape_json(&self.message),
            escape_json(&self.backtrace),
        )
    }

    /// Writes the log file and, if opted in, the JSON crash report into `dir`.
    pub fn write_to(&self, dir: &Path, write_report: bool) -> io::Result<(PathBuf, Option<PathBuf>)> {
        if !dir.exists() {
            fs::create_dir_all(dir)?;
        }

        let base_name = format!("crash_{}_{}", self.timestamp_file, self.pid);
        let log_path = dir.join(format!("{base_name}.log"));
        fs::write(&log_path, self.format_text_log())?;

        let report_path = if write_report {
            let r_path = dir.join(format!("{base_name}.report.json"));
            fs::write(&r_path, self.format_json_report())?;
            Some(r_path)
        } else {
            None
        };

        Ok((log_path, report_path))
    }
}

/// Installs the panic hook.
///
/// When a panic happens:
/// 1. A crash log file is written to `ANVIL_CRASH_DIR` (or `./crashes`).
/// 2. If opted in (or `ANVIL_CRASH_REPORT=1`), a JSON crash report is also written.
/// 3. A banner is printed to `stderr` with the panic details and path to the crash file.
/// 4. The previous panic hook is called so existing handlers or standard diagnostics remain.
pub fn install() {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }

    let prev_hook = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        let record = CrashRecord::from_panic(info);
        let out_dir = crash_dir();
        let opt_in = is_reporting_opt_in();

        let write_result = record.write_to(&out_dir, opt_in);

        // Ensure there are no silent exits: output to stderr with file locations.
        eprintln!("\n================================================================================");
        eprintln!("ANVIL CRASH DETECTED");
        eprintln!("Thread '{}' panicked at '{}'", record.thread_name, record.message);
        eprintln!("Location: {}", record.location);

        match write_result {
            Ok((log_path, report_path)) => {
                eprintln!("Crash log saved: {}", log_path.display());
                if let Some(r_path) = report_path {
                    eprintln!("Crash report saved: {}", r_path.display());
                }
            }
            Err(e) => {
                eprintln!("Failed to write crash log to {}: {e}", out_dir.display());
            }
        }
        eprintln!("================================================================================\n");

        prev_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_timestamp_known_epoch() {
        let (iso, file_safe) = format_utc_timestamp(SystemTime::UNIX_EPOCH);
        assert_eq!(iso, "1970-01-01T00:00:00.000Z");
        assert_eq!(file_safe, "19700101_000000_000");
    }

    #[test]
    fn json_escape_special_chars() {
        let raw = "hello \"world\"\nnext\tline\\end";
        let escaped = escape_json(raw);
        assert_eq!(escaped, "hello \\\"world\\\"\\nnext\\tline\\\\end");
    }

    #[test]
    fn write_crash_log_and_opt_in_report() {
        let temp_dir = std::env::temp_dir().join(format!("anvil_test_crash_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);

        let record = CrashRecord {
            timestamp_iso: "2026-10-27T12:00:00.000Z".into(),
            timestamp_file: "20261027_120000_000".into(),
            anvil_version: "0.3.0",
            os: "test_os",
            arch: "test_arch",
            pid: 99999,
            thread_name: "test-thread".into(),
            location: "src/test.rs:10:5".into(),
            message: "simulated crash error".into(),
            backtrace: "test backtrace frame 1\nframe 2".into(),
        };

        // Write without report
        let (log_path, report_path) = record.write_to(&temp_dir, false).expect("write failed");
        assert!(log_path.exists());
        assert!(report_path.is_none());

        let content = fs::read_to_string(&log_path).expect("read failed");
        assert!(content.contains("ANVIL ENGINE CRASH LOG"));
        assert!(content.contains("simulated crash error"));
        assert!(content.contains("src/test.rs:10:5"));
        assert!(content.contains("test-thread"));

        // Write with report
        let (log_path2, report_path2) = record.write_to(&temp_dir, true).expect("write failed");
        assert!(log_path2.exists());
        assert!(report_path2.is_some());
        let r_path = report_path2.unwrap();
        assert!(r_path.exists());

        let r_content = fs::read_to_string(&r_path).expect("read failed");
        assert!(r_content.contains("\"schema\": \"anvil_crash_v1\""));
        assert!(r_content.contains("\"message\": \"simulated crash error\""));
        assert!(r_content.contains("\"location\": \"src/test.rs:10:5\""));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn opt_in_flag_toggle() {
        set_reporting_opt_in(false);
        assert!(!REPORT_OPT_IN.load(Ordering::SeqCst));
        set_reporting_opt_in(true);
        assert!(REPORT_OPT_IN.load(Ordering::SeqCst));
        set_reporting_opt_in(false);
    }
}
