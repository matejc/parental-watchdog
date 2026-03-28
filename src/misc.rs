use std::{
    io,
    process::{Command, Stdio},
};

use anyhow::{Context, Result};
use users::get_user_by_name;

const NOTIFY_SEND_CMD: &str = "notify-send";

pub fn run_as_user(user: &str, args: &[&str]) -> Result<String> {
    let uid = get_user_by_name(user).unwrap().uid();
    let output = Command::new("runuser")
        .env("XDG_RUNTIME_DIR", format!("/run/user/{:?}", uid))
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path=/run/user/{:?}/bus", uid),
        )
        .arg("-u")
        .arg(user)
        .arg("--")
        .args(args)
        .output()
        .with_context(|| format!("failed to run {:?} as {}", args, user))?;

    if !output.status.success() {
        anyhow::bail!(
            "command {:?} exited with status {}: {}",
            args,
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

pub fn run_command(cmd: &str, args: &[&str]) -> io::Result<String> {
    let output = Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()?;

    if !output.status.success() {
        Err(io::Error::new(
            io::ErrorKind::Other,
            format!("command `{}` exited with status {}", cmd, output.status),
        ))
    } else {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}
