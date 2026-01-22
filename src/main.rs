use anyhow::Result;
use clap::{ArgGroup, Parser};
use regex::Regex;
use std::{
    collections::HashMap,
    fs::{self, File, create_dir_all},
    io::{BufRead, BufReader},
    path::PathBuf,
    process::Command,
    thread,
    time::Duration,
};

use crate::{backend::make_lister, misc::run_command, misc::send_stop_warning};
pub mod backend;
pub mod misc;

/// Monitor processes/windows belonging to a given user, accumulate run‑time,
/// warn before a configurable limit and eventually terminate the process.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
#[command(group(
    ArgGroup::new("pattern")
        .required(true)
        .args(&["cmd_pattern", "title_pattern"])
        .multiple(true)
))]
struct Args {
    /// Username that owns the graphical session (mandatory)
    #[arg(long, short = 'u')]
    user: String,

    /// Hard time‑limit in seconds (default 7200 ≈ 2 h)
    #[arg(long, default_value_t = 7200)]
    limit: u64,

    /// Seconds before the limit when a warning is shown (default 900 ≈ 15 min)
    #[arg(long, default_value_t = 900)]
    warn_before: u64,

    /// Interval between scans, in seconds
    #[arg(long, default_value_t = 10)]
    interval: u64,

    /// Path to the persistent apps file (default $HOME/.local/share/parental-watchdog)
    #[arg(long, short = 'f', default_value = "")]
    apps_file: String,

    /// Regex that must match the command name
    #[arg(long, value_name = "REGEX")]
    cmd_pattern: String,

    /// Regex that must match the window title
    #[arg(long, value_name = "REGEX")]
    title_pattern: String,

    /// Which backend to use: "kdotool" or "niri"
    #[arg(short, long, default_value = "kdotool")]
    backend: backend::Backend,
}

// ---------------------------------------------------------------------------
// Load persisted `<key> <seconds>` pairs from the apps file.
fn load_apps(path: &PathBuf) -> Result<HashMap<String, u64>> {
    let mut map = HashMap::new();

    // Touch‑like behaviour – create an empty file if missing.
    if !path.exists() {
        File::create(path)?;
        return Ok(map);
    }

    let f = File::open(path)?;
    for line in BufReader::new(f).lines() {
        let l = line?;
        let mut parts = l.splitn(2, ' ');
        if let (Some(key), Some(val_str)) = (parts.next(), parts.next()) {
            if let Ok(val) = val_str.parse::<u64>() {
                map.insert(key.to_string(), val);
            }
        }
    }
    Ok(map)
}

// ---------------------------------------------------------------------------
// Write the hashmap back to disk (overwrites the file).
fn save_apps(path: &PathBuf, apps: &HashMap<String, u64>) -> Result<()> {
    let mut out = String::new();
    for (k, v) in apps {
        out.push_str(&format!("{k} {v}\n"));
    }
    fs::write(path, out)?;
    Ok(())
}

fn sum_seconds(apps: &HashMap<String, u64>) -> u64 {
    // Build the suffix we need to match – e.g. "2026-01-18"
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    apps.iter()
        .filter(|(k, _)| {
            // Fast‑path: the key must start with the literal prefix
            if !k.starts_with("seconds:") {
                return false;
            }

            let mut parts = k.split(':');
            match (parts.next(), parts.next(), parts.next(), parts.next()) {
                (Some("seconds"), Some(_), Some(_), Some(date_part)) => date_part == today,
                _ => false,
            }
        })
        // Sum the values that passed the filter
        .map(|(_, v)| *v)
        .sum()
}

// ---------------------------------------------------------------------------
// Core logic: given a PID, fetch its elapsed time, command name and full command line,
// then decide whether to store / warn / kill.
fn add_to_apps(
    user: &str,
    apps: &mut HashMap<String, u64>,
    apps_path: &PathBuf,
    pid: u32,
    cmd_rx: &Regex,
    title_rx: &Regex,
    title: &str,
    limit: u64,
    warn_before: u64,
    warned: &mut String,
) -> Result<bool> {
    // Retrieve process info via `ps`.
    let ps_out = run_command(
        "ps",
        &[
            "--no-headers",
            "-p",
            &pid.to_string(),
            "-o",
            "etimes,comm,command",
        ],
    )?;

    // Example: "1234 bash /bin/bash -c …"
    let mut parts = ps_out.split_whitespace();
    let secs_str = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing etimes from ps output"))?;
    let comm = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing comm from ps output"))?;
    let command: String = parts.collect::<Vec<_>>().join(" ");
    // The rest of the command line is ignored for our matching needs.
    let seconds: u64 = secs_str.parse()?;

    let match_cmd = match cmd_rx.is_match(&command) {
        true => {
            println!("Matched by cmd: {command}");
            true
        }
        false => false,
    };

    let match_title = match title_rx.is_match(title) {
        true => {
            println!("Matched by title: {title}");
            true
        }
        false => false,
    };

    if !match_cmd && !match_title {
        return Ok(false);
    }

    // Build a deterministic key: "<comm>:<pid>:<YYYY‑MM‑DD>"
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let key = format!("seconds:{comm}:{pid}:{today}");

    let entry = match apps.get_mut(&key) {
        None => {
            // No existing entry – just store the incoming seconds.
            apps.insert(key.clone(), seconds);
            seconds
        }
        Some(old_seconds) => {
            // There is already a value. Compute the delta and add it.
            let delta = seconds.saturating_sub(*old_seconds);
            *old_seconds = old_seconds.saturating_add(delta);
            *old_seconds
        }
    };

    let total = sum_seconds(apps);
    println!("App: {key} => {entry} ({total}/{limit})");
    let _ = save_apps(apps_path, apps);

    // Warning / killing logic.
    if total > (limit - warn_before) && total < limit && *warned != today {
        send_stop_warning(user, limit - total)?;
        *warned = today;
    } else if total >= limit {
        println!("Killing {pid}, after {total}s reached: cmd='{comm}', title='{title}'");
        // Fire SIGTERM; ignore errors (process may already be gone).
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status();
    }

    Ok(true)
}

// ---------------------------------------------------------------------------
// Program entry point.
fn main() -> Result<()> {
    let args = Args::parse();
    let lister = make_lister(args.backend);

    let apps_path = if args.apps_file.len() != 0 {
        PathBuf::from(args.apps_file)
    } else {
        let mut home = dirs::state_dir().unwrap();
        create_dir_all(&home)?;
        home.push("parental-watchdog");
        home
    };

    // Load existing data.
    let mut apps = load_apps(&apps_path)?;

    let cmd_regex = match Regex::new(&args.cmd_pattern) {
        Ok(rx) => rx,
        Err(err) => panic!("Problem with cmd_regex: {err:?}"),
    };
    let title_regex = match Regex::new(&args.title_pattern) {
        Ok(rx) => rx,
        Err(err) => panic!("Problem with title_regex: {err:?}"),
    };

    // -----------------------------------------------------------------------
    // Main monitoring loop.
    let mut warned = String::from(""); // remember whether we already sent the warning
    loop {
        match lister.list_windows(&args.user) {
            Ok(windows) => {
                for win in windows {
                    if add_to_apps(
                        &args.user,
                        &mut apps,
                        &apps_path,
                        win.pid,
                        &cmd_regex,
                        &title_regex,
                        &win.title,
                        args.limit,
                        args.warn_before,
                        &mut warned,
                    )? {
                        break; // save only first window
                    }
                }
            }
            Err(e) => eprintln!("Error retrieving windows: {}", e),
        }

        // Wait before the next scan.
        thread::sleep(Duration::from_secs(args.interval));
    }
}
