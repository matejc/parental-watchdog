use crate::misc::run_as_user;
use std::io::{self};

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub title: String,
    pub pid: u32,
}

/// Trait that defines the “interface” for listing windows.
pub trait WindowLister {
    /// Returns a list of windows (title + pid) or an error.
    fn list_windows(&self, user: &str) -> io::Result<Vec<WindowInfo>>;
}

/* -------------------------------------------------------------------------- */
/* Implementation for kdotool                                                */
/* -------------------------------------------------------------------------- */
pub struct KdotoolLister;

impl WindowLister for KdotoolLister {
    fn list_windows(&self, user: &str) -> io::Result<Vec<WindowInfo>> {
        let output = run_as_user(user, &["kdotool", "search", "--name", "."]).unwrap_or_else(|e| {
            eprintln!("failed to execute kdotool: {}", e);
            String::default()
        });

        let mut result = Vec::new();
        for win_id in output.lines() {
            // Resolve the real PID belonging to the window.
            let pid_str = run_as_user(&user, &["kdotool", "getwindowpid", win_id]).unwrap();
            let pid: u32 = match pid_str.trim().parse() {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Obtain the (potentially refreshed) window title.
            let title = run_as_user(&user, &["kdotool", "getwindowname", win_id]).unwrap();

            result.push(WindowInfo {
                title: title.trim().to_string(),
                pid,
            });
        }
        Ok(result)
    }
}

/* -------------------------------------------------------------------------- */
/* Implementation for niri (via `niri msg`)                                   */
/* -------------------------------------------------------------------------- */
pub struct NiriLister;

impl WindowLister for NiriLister {
    fn list_windows(&self, user: &str) -> io::Result<Vec<WindowInfo>> {
        // `niri msg windows` returns JSON describing each window.
        let output = run_as_user(user, &["niri", "msg", "-j", "windows"]).unwrap_or_else(|e| {
            eprintln!("failed to execute niri: {}", e);
            String::default()
        });

        #[derive(serde::Deserialize)]
        struct NiriWindow {
            pid: u32,
            title: String,
        }

        let parsed: Vec<NiriWindow> = serde_json::from_str(&output).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to parse niri JSON: {}", e),
            )
        })?;

        Ok(parsed
            .into_iter()
            .map(|w| WindowInfo {
                pid: w.pid,
                title: w.title,
            })
            .collect())
    }
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum Backend {
    Kdotool,
    Niri,
}

pub fn make_lister(backend: Backend) -> Box<dyn WindowLister> {
    match backend {
        Backend::Kdotool => Box::new(KdotoolLister),
        Backend::Niri => Box::new(NiriLister),
    }
}
