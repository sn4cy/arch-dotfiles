#!/bin/sh
# Serialize requests and keep the lock held until hyprlock exits.
exec /usr/bin/flock -n "${XDG_RUNTIME_DIR:?}/screen-lock.lock" /bin/sh -c 'pgrep -u "$(id -u)" -x hyprlock >/dev/null || exec /usr/bin/hyprlock --grace 0'
