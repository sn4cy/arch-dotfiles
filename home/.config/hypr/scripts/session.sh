#!/bin/sh
systemctl --user import-environment WAYLAND_DISPLAY HYPRLAND_INSTANCE_SIGNATURE XDG_CURRENT_DESKTOP
dbus-update-activation-environment --systemd WAYLAND_DISPLAY HYPRLAND_INSTANCE_SIGNATURE XDG_CURRENT_DESKTOP
systemctl --user restart screen-lock-idle.service fast-alt-tab.service floating-dock.service
/usr/lib/polkit-kde-authentication-agent-1 &
nm-applet &
blueman-applet &
wait
