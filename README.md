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
sudo mkdir /etc/parental-watchdog  # for config files
sudo cp -v ./examples/config.yaml /etc/parental-watchdog/kid.yaml
sudo mkdir /var/lib/parental-watchdog  # for state files
```

Edit the `/etc/parental-watchdog/kid.yaml` file to at least change username and patterns (current patterns should match Steam, Heroic, Minecraft via PrismLauncher, Sober and YouTube in the title - internet browser window).
Edit the `parental-watchdog-kid.service` if you changed the config file name.

```bash
sudo systemctl enable --now ./examples/parental-watchdog-kid.service
```

Note:
- For subsequent edits of the `parental-watchdog-kid.service`, make sure that you run `sudo systemctl daemon-reload` so that the systemd reloads the file and then you need to restart the service manually via `sudo systemctl restart parental-watchdog.service`


## Usage

```
Monitor processes/windows belonging to a given user, accumulate run‑time, warn before a configurable limit and eventually terminate the process

Usage: parental-watchdog <COMMAND>

Commands:
  run             Run the parental watchdog monitor
  time-used       Show time used for today
  time-remaining  Show time left for today
  show-config     Show effective configuration for today
  help            Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

## Develop

```
nix-shell
cargo run -- run -c ./examples/config.yaml -a /tmp/parental-watchdog-kid  # Example command
```
