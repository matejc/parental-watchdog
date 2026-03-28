use anyhow::Result;
use chrono::NaiveTime;
use clap::{Parser, Subcommand};
use regex::Regex;
use std::{
    collections::HashMap,
    fs::{self, create_dir_all, File},
    io::{BufRead, BufReader},
    path::PathBuf,
    process::Command,
    thread,
    time::Duration,
};

use crate::{
    backend::make_lister,
    config::load_config,
    misc::{fmt_time, run_command, send_stop_warning},
};
pub mod backend;
pub mod config;
pub mod misc;

/// Monitor processes/windows belonging to a given user, accumulate run‑time,
/// warn before a configurable limit and eventually terminate the process.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run the parental watchdog monitor
    Run(RunArgs),
    /// Show time used for today
    TimeUsed(TimeUsedArgs),
    /// Show time left for today
    TimeRemaining(TimeRemainingArgs),
    /// Show effective configuration for today
    ShowConfig(ConfigArgs),
}

#[derive(Parser, Debug)]
struct RunArgs {
    /// Path to the YAML configuration file
    #[arg(long, short = 'c')]
    config: String,

    /// Path to the persistent apps file
    #[arg(long, short = 'a', default_value = "")]
    apps_path: String,
}

#[derive(Parser, Debug)]
struct TimeUsedArgs {
    /// Path to the persistent apps file
    #[arg(long, short = 'a', default_value = "")]
    apps_path: String,
}

#[derive(Parser, Debug)]
struct TimeRemainingArgs {
    /// Path to the YAML configuration file
    #[arg(long, short = 'c')]
    config: String,

    /// Path to the persistent apps file
    #[arg(long, short = 'a', default_value = "")]
    apps_path: String,
}

#[derive(Parser, Debug)]
struct ConfigArgs {
    /// Path to the YAML configuration file
    #[arg(long, short = 'c')]
    config: String,
}

// ---------------------------------------------------------------------------
// Load persisted `<key> <seconds>` pairs from the apps file.
fn load_apps(path: &PathBuf) -> Result<HashMap<String, i64>> {
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
            if let Ok(val) = val_str.parse::<i64>() {
                map.insert(key.to_string(), val);
            }
        }
    }
    Ok(map)
}

// ---------------------------------------------------------------------------
// Write the hashmap back to disk (overwrites the file).
fn save_apps(path: &PathBuf, apps: &HashMap<String, i64>) -> Result<()> {
    let mut out = String::new();
    for (k, v) in apps {
        out.push_str(&format!("{k} {v}\n"));
    }
    fs::write(path, out)?;
    Ok(())
}

fn parse_key(key: &str) -> Option<(String, i64, String)> {
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
            if let Ok(start) = start_str.parse::<i64>() {
                Some((app.to_string(), start, date.to_string()))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn merge_intervals(mut intervals: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
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

fn sum_seconds_for_today(apps: &HashMap<String, i64>) -> i64 {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    let mut intervals: Vec<(i64, i64)> = Vec::new();

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
    apps: &mut HashMap<String, i64>,
    apps_path: &PathBuf,
    pid: u32,
    cmd_rx: &Option<Regex>,
    title_rx: &Option<Regex>,
    title: &str,
    limit: i64,
    warn_before: i64,
    warned: &mut String,
    time_begin: NaiveTime,
    time_end: NaiveTime,
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
    let seconds: i64 = secs_str.parse()?;

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

    let today_date = chrono::Local::now().date_naive();
    let today = today_date.format("%Y-%m-%d").to_string();
    let now_epoch = chrono::Local::now().timestamp();
    let start_at = now_epoch.saturating_sub(seconds);
    let today_begin_epoch = today_date
        .and_time(time_begin)
        .and_local_timezone(chrono::Local)
        .single()
        .unwrap()
        .timestamp();
    let today_end_epoch = today_date
        .and_time(time_end)
        .and_local_timezone(chrono::Local)
        .single()
        .unwrap()
        .timestamp();

    if today_begin_epoch > now_epoch {
        println!(
            "Killing {pid}, before the begin time ({}): cmd='{comm}', title='{title}'",
            fmt_time(today_begin_epoch - now_epoch)
        );
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status();
        return Ok(true);
    } else if now_epoch > today_end_epoch {
        println!(
            "Killing {pid}, after the end time ({}): cmd='{comm}', title='{title}'",
            fmt_time(now_epoch - today_end_epoch)
        );
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status();
        return Ok(true);
    }

    // Build a deterministic key: "app:<comm>:<pid>:<epoch>:<YYYY‑MM‑DD>"
    let key = format!("app:{comm}:{pid}:{start_at}:{today}");

    let seconds_per_key = match apps.get_mut(&key) {
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
    let _ = save_apps(apps_path, apps);

    let remaining = if (today_end_epoch - now_epoch) < (limit - total) {
        today_end_epoch - now_epoch
    } else {
        limit - total
    };

    println!(
        "App[{key} = {}]: Used {} out of {}, remaining {}",
        fmt_time(seconds_per_key),
        fmt_time(total),
        fmt_time(limit),
        fmt_time(remaining)
    );

    // Warning / killing logic.
    if remaining < warn_before && *warned != today {
        send_stop_warning(user, remaining)?;
        *warned = today;
    } else if remaining < 0 {
        println!(
            "Killing {pid}, after {} reached: cmd='{comm}', title='{title}'",
            fmt_time(total)
        );
        // Fire SIGTERM; ignore errors (process may already be gone).
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status();
    }

    Ok(true)
}

fn resolve_apps_path(apps_path: &str) -> Result<PathBuf> {
    if !apps_path.is_empty() {
        Ok(PathBuf::from(apps_path))
    } else {
        let mut home_state = dirs::state_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine state directory"))?;
        create_dir_all(&home_state)?;
        home_state.push("parental-watchdog");
        Ok(home_state)
    }
}

fn resolve_config_path(config_path: &str) -> Result<PathBuf> {
    if !config_path.is_empty() {
        Ok(PathBuf::from(config_path))
    } else {
        let mut home_config = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;
        home_config.push("parental-watchdog");
        home_config.push("config.yaml");
        Ok(home_config)
    }
}

fn run_monitor(args: RunArgs) -> Result<()> {
    let config_path = resolve_config_path(&args.config)?;
    let apps_path = resolve_apps_path(&args.apps_path)?;

    // Load existing data.
    let mut apps = load_apps(&apps_path)?;

    let mut warned = String::from(""); // remember whether we already sent the warning
    loop {
        let config = load_config(&config_path)?;
        let lister = make_lister(config.backend.clone());

        let cmd_regex: Option<Regex> = config.cmd_pattern.as_ref().map(|pat| {
            Regex::new(pat).unwrap_or_else(|err| {
                panic!("Problem compiling cmd pattern `{}`: {err:?}", pat);
            })
        });
        let title_regex: Option<Regex> = config.title_pattern.as_ref().map(|pat| {
            Regex::new(pat).unwrap_or_else(|err| {
                panic!("Problem compiling title pattern `{}`: {err:?}", pat);
            })
        });

        let time_begin = chrono::NaiveTime::parse_from_str(&config.time_begin, "%H:%M")
            .unwrap_or_else(|err| {
                panic!("Parse begin time error `{}`: {err:?}", config.time_begin);
            });

        let time_end =
            chrono::NaiveTime::parse_from_str(&config.time_end, "%H:%M").unwrap_or_else(|err| {
                panic!("Parse end time error `{}`: {err:?}", config.time_end);
            });

        match lister.list_windows(&config.user, &config.backend_path) {
            Ok(windows) => {
                for win in windows {
                    add_to_apps(
                        &config.user,
                        &mut apps,
                        &apps_path,
                        win.pid,
                        &cmd_regex,
                        &title_regex,
                        &win.title,
                        config.limit,
                        config.warn_before,
                        &mut warned,
                        time_begin,
                        time_end,
                    )?;
                }
            }
            Err(e) => eprintln!("Error retrieving windows: {}", e),
        }

        // Wait before the next scan.
        thread::sleep(Duration::from_secs(config.interval));
    }
}

fn show_time_used(args: TimeUsedArgs) -> Result<()> {
    let apps_path = resolve_apps_path(&args.apps_path)?;

    let apps = load_apps(&apps_path)?;
    let total = sum_seconds_for_today(&apps);
    println!("{}", fmt_time(total));

    Ok(())
}

fn show_time_remaining(args: TimeRemainingArgs) -> Result<()> {
    let apps_path = resolve_apps_path(&args.apps_path)?;
    let config_path = resolve_config_path(&args.config)?;

    let apps = load_apps(&apps_path)?;
    let config = load_config(&config_path)?;

    let today_date = chrono::Local::now().date_naive();
    let time_end =
        chrono::NaiveTime::parse_from_str(&config.time_end, "%H:%M").unwrap_or_else(|err| {
            panic!("Parse end time error `{}`: {err:?}", config.time_end);
        });
    let today_end_epoch = today_date
        .and_time(time_end)
        .and_local_timezone(chrono::Local)
        .single()
        .unwrap()
        .timestamp();

    let now_epoch = chrono::Local::now().timestamp();
    let total = sum_seconds_for_today(&apps);

    let time_until_end = (today_end_epoch - now_epoch).max(0);
    let limit_remaining = (config.limit - total).max(0);
    let remaining = time_until_end.min(limit_remaining);

    println!("{}", fmt_time(remaining));

    Ok(())
}

fn show_config(args: ConfigArgs) -> Result<()> {
    let config_path = resolve_config_path(&args.config)?;
    let config = load_config(&config_path)?;

    println!("{}", serde_yaml::to_string(&config)?);

    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();

    match args.command {
        Commands::Run(args) => run_monitor(args),
        Commands::TimeUsed(args) => show_time_used(args),
        Commands::TimeRemaining(args) => show_time_remaining(args),
        Commands::ShowConfig(args) => show_config(args),
    }
}
