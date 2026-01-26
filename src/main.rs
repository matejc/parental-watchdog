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

    /// Path to the persistent apps file (default $HOME/.local/state/parental-watchdog)
    #[arg(long, short = 'f', default_value = "")]
    apps_file: String,

    /// Regex that must match the command name
    #[arg(long, value_name = "REGEX")]
    cmd_pattern: Option<String>,

    /// Regex that must match the window title
    #[arg(long, value_name = "REGEX")]
    title_pattern: Option<String>,

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

fn parse_key(key: &str) -> Option<(String, u64, String)> {
    // Returns (app_name, start_epoch, date_str) if the key matches our pattern
    let mut parts = key.split(':');

    // Expected layout: seconds : <app> : <pid> : <start_epoch> : <date>
    match (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) {
        (Some("app"), Some(app), Some(_pid), Some(start_str), Some(date)) => {
            if let Ok(start) = start_str.parse::<u64>() {
                Some((app.to_string(), start, date.to_string()))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn merge_intervals(mut intervals: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    if intervals.is_empty() {
        return intervals;
    }

    intervals.sort_unstable_by_key(|&(s, _)| s);

    let mut merged = Vec::with_capacity(intervals.len());
    let mut cur = intervals[0];

    for &(s, e) in intervals.iter().skip(1) {
        if s > cur.1 {
            // No overlap – push the finished interval
            merged.push(cur);
            cur = (s, e);
        } else {
            // Overlap – extend the current interval
            if e > cur.1 {
                cur.1 = e;
            }
        }
    }
    merged.push(cur);
    merged
}

fn sum_seconds_for_today(apps: &HashMap<String, u64>) -> u64 {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    let mut intervals: Vec<(u64, u64)> = Vec::new();

    for (key, &etime) in apps.iter() {
        if !key.starts_with("app:") {
            continue;
        }

        // Parse the key – we need app name, start epoch, and the date part
        if let Some((_app, start_epoch, date_part)) = parse_key(key) {
            if date_part == today {
                // Build the interval: [start, start + etime)
                let end = start_epoch.saturating_add(etime);
                intervals.push((start_epoch, end));
            }
        }
    }

    let merged = merge_intervals(intervals);

    merged.iter().map(|&(s, e)| e - s).sum()
}

fn matches_rx(str: &str, regex_opt: &Option<Regex>) -> bool {
    match regex_opt {
        Some(re) => re.is_match(str),
        None => false,
    }
}

fn add_to_apps(
    user: &str,
    apps: &mut HashMap<String, u64>,
    apps_path: &PathBuf,
    pid: u32,
    cmd_rx: &Option<Regex>,
    title_rx: &Option<Regex>,
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

    let match_cmd = if matches_rx(&command, cmd_rx) {
        println!("Matched by cmd: {command}");
        true
    } else {
        false
    };

    let match_title = if matches_rx(title, title_rx) {
        println!("Matched by title: {title}");
        true
    } else {
        false
    };

    if !match_cmd && !match_title {
        return Ok(false);
    }

    // Build a deterministic key: "<comm>:<pid>:<YYYY‑MM‑DD>"
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let start_at = chrono::Local::now()
        .timestamp()
        .saturating_sub_unsigned(seconds);

    let key = format!("app:{comm}:{pid}:{start_at}:{today}");

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

    let total = sum_seconds_for_today(apps);
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

    let cmd_regex: Option<Regex> = args.cmd_pattern.as_ref().map(|pat| {
        Regex::new(pat).unwrap_or_else(|err| {
            panic!("Problem compiling cmd pattern `{}`: {err:?}", pat);
        })
    });
    let title_regex: Option<Regex> = args.title_pattern.as_ref().map(|pat| {
        Regex::new(pat).unwrap_or_else(|err| {
            panic!("Problem compiling title pattern `{}`: {err:?}", pat);
        })
    });

    let mut warned = String::from(""); // remember whether we already sent the warning
    loop {
        match lister.list_windows(&args.user) {
            Ok(windows) => {
                for win in windows {
                    add_to_apps(
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
                    )?;
                }
            }
            Err(e) => eprintln!("Error retrieving windows: {}", e),
        }

        // Wait before the next scan.
        thread::sleep(Duration::from_secs(args.interval));
    }
}
