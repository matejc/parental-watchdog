use std::{
    collections::BTreeMap,
    process::Command,
};

use anyhow::{Context, Result};
use users::get_user_by_name;

const NOTIFY_SEND_CMD: &str = "notify-send";

pub fn run_as_user(user: &str, args: &[&str]) -> Result<String> {
    anyhow::ensure!(!args.is_empty(), "no command specified");

    let output = run_as_user_raw(
        user,
        &["systemctl", "--user", "show-environment", "--output=json"],
        &[],
    )
    .with_context(|| format!("failed to read systemd environment for {user}"))?;
    let environment: Option<BTreeMap<String, String>> = serde_json::from_str(&output)
        .with_context(|| format!("invalid systemd environment for {user}"))?;

    let environment = environment.unwrap_or_default();
    let environment: Vec<(&str, &str)> = environment
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    run_as_user_raw(user, args, &environment)
        .with_context(|| format!("failed to run {:?} as {}", args, user))
}

fn run_as_user_raw(user: &str, args: &[&str], environment: &[(&str, &str)]) -> Result<String> {
    let uid = get_user_by_name(user)
        .with_context(|| format!("user {user} not found"))?
        .uid();
    let mut command = Command::new("runuser");
    command
        .env("XDG_RUNTIME_DIR", format!("/run/user/{:?}", uid))
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path=/run/user/{:?}/bus", uid),
        );
    for &(key, value) in environment {
        anyhow::ensure!(
            !key.is_empty() && !key.contains(['=', '\0']) && !value.contains('\0'),
            "invalid environment entry for {user}"
        );
        command.env(key, value);
    }
    let output = command
        .arg("-u")
        .arg(user)
        .arg("--")
        .args(args)
        .output()
        .with_context(|| format!("failed to run command as {user}"))?;

    if !output.status.success() {
        anyhow::bail!(
            "command exited with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim().to_string()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn fmt_time(seconds: i64) -> String {
    humantime::format_duration(std::time::Duration::from_secs(seconds as u64)).to_string()
}

pub fn send_stop_warning(user: &str, remaining: i64) -> Result<()> {
    let msg = format!("Stopping in {}", fmt_time(remaining));
    println!("Sending warning: '{msg}' ...");
    run_as_user(user, &[NOTIFY_SEND_CMD, &msg])?;
    Ok(())
}
