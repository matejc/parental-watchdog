# Parental Watchdog

Parental control service for tracking open windows (by command line pattern and/or by title pattern) and terminating them when the daily time limit is up.

Warning: currently supports KDE via kdotool and X11 via xdotool, but new backends can be added in the future.

Note: niri window manager is partially supported due to all X11 windows being detected as xwayland-satellite, so after time limit, all X11 including xwayland-satellite will be terminated.

How it works:
- scans for open windows
- if the open window title or process/cmd command matches any pattern it notes down process time (etimes from `ps -p 123456 -o etimes`)
- it sends the warning to the user that in N amount of seconds (default 15min) the matched windows will be terminated
- after the combined window time has reached time limit (default 2h) the matched windows will be terminated (SIGTERM)

Note:
- App persists the data on windows and it works correctly even if the windows with matched patterns are restarted, or even if the whole machine restarts
- The kid's user must not have sudo access, otherwise they can just stop the service

## Runtime dependencies

- Commands (you likely already have installed): runuser, ps
- Command: notify-send - for sending the warning message
- [kdotool](https://github.com/jinliu/kdotool) - for KDE support
- xdotool - for X11 support


## Build

```bash
cargo build --release
```

## Install

For now this is not automated

```bash
sudo cp -v ./target/release/parental-watchdog /usr/bin/
```

Edit the `parental-watchdog.service` file to at least change username and patterns (current patterns should match Steam, Heroic, Minecraft via PrismLauncher, Sober and YouTube in the title - internet browser window).

```bash
sudo systemctl enable --now ./parental-watchdog.service
```

Note:
- if you need to edit the `./parental-watchdog.service`, make sure that you run `sudo systemctl daemon-reload` so that the systemd reloads the file and then do restart the service manually


## Usage

```
Usage: parental-watchdog [OPTIONS] --user <USER> <--cmd-pattern <REGEX>|--title-pattern <REGEX>>

Options:
  -u, --user <USER>                Username that owns the graphical session (mandatory)
      --limit <LIMIT>              Hard time‑limit in seconds (default 7200 ≈ 2 h) [default: 7200]
      --warn-before <WARN_BEFORE>  Seconds before the limit when a warning is shown (default 900 ≈ 15 min) [default: 900]
      --interval <INTERVAL>        Interval between scans, in seconds [default: 10]
  -f, --apps-file <APPS_FILE>      Path to the persistent apps file (default $HOME/.local/share/parental-watchdog) [default: ]
      --cmd-pattern <REGEX>        Regex that must match the command name
      --title-pattern <REGEX>      Regex that must match the window title
  -b, --backend <BACKEND>          Which backend to use: "kdotool" or "niri" [default: kdotool] [possible values: kdotool, niri]
  -h, --help                       Print help
  -V, --version                    Print version
```

## Develop

```
nix-shell
cargo run -- --user $USER --cmd-pattern '^some-example$|somethingelse'  # Example command
```
