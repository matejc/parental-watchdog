use crate::misc::run_as_user;
use std::io::{self};
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub title: String,
    pub pid: u32,
}

/// Trait that defines the "interface" for listing windows.
pub trait WindowLister {
    /// Returns a list of windows (title + pid) or an error.
    fn list_windows(&self, user: &str, backend_path: &str) -> io::Result<Vec<WindowInfo>>;
}

/* -------------------------------------------------------------------------- */
/* Implementation for kdotool                                                */
/* -------------------------------------------------------------------------- */
pub struct KdotoolLister;

impl WindowLister for KdotoolLister {
    fn list_windows(&self, user: &str, backend_path: &str) -> io::Result<Vec<WindowInfo>> {
        let exec_path = if backend_path.is_empty() {
            "kdotool"
        } else {
            backend_path
        };

        let output = run_as_user(user, &[exec_path, "search", "--name", "."]).unwrap_or_else(|e| {
            eprintln!("failed to execute kdotool: {}", e);
            String::default()
        });

        let mut result = Vec::new();
        for win_id in output.lines() {
            // Resolve the real PID belonging to the window.
            let pid_str = run_as_user(&user, &[exec_path, "getwindowpid", win_id]).unwrap();
            let pid: u32 = match pid_str.trim().parse() {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Obtain the (potentially refreshed) window title.
            let title = run_as_user(&user, &[exec_path, "getwindowname", win_id]).unwrap();

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
    fn list_windows(&self, user: &str, backend_path: &str) -> io::Result<Vec<WindowInfo>> {
        let exec_path = if backend_path.is_empty() {
            "niri"
        } else {
            backend_path
        };

        // `niri msg windows` returns JSON describing each window.
        let output = run_as_user(user, &[exec_path, "msg", "-j", "windows"]).unwrap_or_else(|e| {
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

/* -------------------------------------------------------------------------- */
/* Implementation for xdotool                                                */
/* -------------------------------------------------------------------------- */
pub struct XdotoolLister;

impl WindowLister for XdotoolLister {
    fn list_windows(&self, user: &str, backend_path: &str) -> io::Result<Vec<WindowInfo>> {
        let exec_path = if backend_path.is_empty() {
            "xdotool"
        } else {
            backend_path
        };

        let output = run_as_user(user, &[exec_path, "search", "--onlyvisible", "--name", "."])
            .unwrap_or_else(|e| {
                eprintln!("failed to execute xdotool: {}", e);
                String::default()
            });

        let mut result = Vec::new();
        for win_id in output.lines() {
            // Resolve the real PID belonging to the window.
            let pid_str = run_as_user(&user, &[exec_path, "getwindowpid", win_id]).unwrap();
            let pid: u32 = match pid_str.trim().parse() {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Obtain the (potentially refreshed) window title.
            let title = run_as_user(&user, &[exec_path, "getwindowname", win_id]).unwrap();

            result.push(WindowInfo {
                title: title.trim().to_string(),
                pid,
            });
        }
        Ok(result)
    }
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum Backend {
    Kdotool,
    Niri,
    Xdotool,
}

impl FromStr for Backend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "kdotool" => Ok(Backend::Kdotool),
            "niri" => Ok(Backend::Niri),
            "xdotool" => Ok(Backend::Xdotool),
            _ => Err(format!("unknown backend: {}", s)),
        }
    }
}

pub fn make_lister(backend: String) -> Box<dyn WindowLister> {
    match Backend::from_str(backend.as_str()) {
        Ok(Backend::Kdotool) => Box::new(KdotoolLister),
        Ok(Backend::Niri) => Box::new(NiriLister),
        Ok(Backend::Xdotool) => Box::new(XdotoolLister),
        Err(e) => panic!("Unknown lister `{}`: {e:?}", backend),
    }
}
