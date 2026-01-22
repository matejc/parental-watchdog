#!/usr/bin/env bash

set -eo pipefail

TARGET_USER="${1:?Username as first argument is required}"
CMD_PATTERN="${2:?Cmd pattern as second argument is required}"
TITLE_PATTERN="${3:?Cmd pattern as second argument is required}"
LIMIT_SECONDS=$((2*60*60))
WARN_BEFORE_SECONDS=$((15*60))
INTERVAL=10
APPS_FILE="${APPS_FILE:-$HOME/.apps}"

NOTIFY_SEND_CMD="${NOTIFY_SEND_CMD:-"notify-send"}"
KDOTOOL_CMD="${KDOTOOL_CMD:-"kdotool"}"

declare -A apps

load_from_file() {
    touch "$APPS_FILE"
    while read -r key value
    do
        apps["$key"]="$value"
    done < "$APPS_FILE"
}
load_from_file

save_to_file() {
    echo -n > "$APPS_FILE"
    for key in "${!apps[@]}"
    do
        value=${apps["$key"]}
        echo "$key $value" >> "$APPS_FILE"
    done
}

sum_seconds() {
    seconds=0
    for key in "${!apps[@]}"
    do
        if [[ "$key" == seconds:*:*:$(date +"%Y-%m-%d") ]]
        then
            pseconds=${apps["$key"]}
            seconds=$(( seconds + pseconds ))
        fi
    done
    echo $seconds
}

cleanup() {
    echo "Saving apps ..." >&2
    save_to_file
}

runasuser() {
    DISPLAY=":0" XDG_RUNTIME_DIR="/run/user/$(getent passwd "$TARGET_USER" | cut -d: -f3)" DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(getent passwd "$TARGET_USER" | cut -d: -f3)/bus" runuser -u $TARGET_USER -- "$@"
}

send_stop_warning() {
    total_seconds=$1
    if [ -z "$_notification_sent" ]
    then
        runasuser "$NOTIFY_SEND_CMD" "Stopping in $(( LIMIT_SECONDS - total_seconds ))s"
        _notification_sent="1"
    fi
}

add_to_apps() {
    pid=$1
    IFS=' ' read -r seconds comm command < <(ps --no-headers -p $pid -o etimes,comm,command)
    title="$3"

    if [[ "$command" =~ $CMD_PATTERN ]] || [[ "$title" =~ $TITLE_PATTERN ]]
    then
        key="$comm:$pid:$(date +"%Y-%m-%d")"

        if [[ -z "${apps["seconds:$key"]}" ]]
        then
            apps["seconds:$key"]="$seconds"
        else
            old_seconds="${apps["seconds:$key"]}"
            apps["seconds:$key"]="$(( old_seconds + (seconds - old_seconds) ))"
        fi

        total_seconds=$(sum_seconds)
        echo "Saved: $key => ${apps["seconds:$key"]} ($total_seconds/$LIMIT_SECONDS)"

        if (( total_seconds > (LIMIT_SECONDS - WARN_BEFORE_SECONDS) )) && (( total_seconds < LIMIT_SECONDS ))
        then
            send_stop_warning $total_seconds
        elif (( total_seconds > LIMIT_SECONDS ))
        then
            echo "Killing $pid, after ${total_seconds}s has been reached: cmd='$comm', title='$title'"
            kill -TERM "$pid" || true
        fi

        return 0
    fi

    return 1
}

trap cleanup EXIT

while true; do
    while IFS='|' read -r winId clientPid title rest
    do
        pid="$(runasuser $KDOTOOL_CMD getwindowpid "$winId")"
        title="$(runasuser $KDOTOOL_CMD getwindowname "$winId")"
        if [[ -n $pid && -n $title ]]
        then
            if add_to_apps "$pid" "$winId" "$title"
            then  # save only first window
                break
            fi || true
        fi
    done < <(runasuser $KDOTOOL_CMD search --name .)

    sleep "$INTERVAL"
done
