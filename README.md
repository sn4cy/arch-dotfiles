# Arch dotfiles

An Arch Linux desktop setup with Hyprland, Ghostty, Neovim, Japanese input, a floating dock, and an Alt+Tab switcher.

## Quick start

Run as a regular user with sudo access on an installed Arch Linux system:

```bash
sudo pacman -Syu --needed git && git clone https://github.com/sn4cy/arch-dotfiles.git && bash arch-dotfiles/setup.sh
```

The setup installs packages, builds the desktop tools, deploys configuration files, and enables services for the next boot. Reboot when it finishes, then select Hyprland in SDDM.

## Requirements

- Arch Linux with the OS installation completed.
- A regular user with sudo access.
- An internet connection and suitable graphics drivers.
- The default configuration directory, `~/.config`.

The script does not partition disks, install the OS, configure a bootloader, or create user accounts.

## Desktop

- Hyprland with blur, transparency, workspace shortcuts, and automatic display detection.
- Ghostty with the TokyoNight theme.
- Fcitx5 and Mozc for Japanese input.
- Neovim with NvChad, completion, language servers, and automatic saving when leaving Insert mode.
- A floating dock and an Alt+Tab switcher built from source.
- Tide Island, with Waybar available as a fallback.
- Screen locking, screenshots, volume controls, and brightness controls.
- PipeWire, NetworkManager, Bluetooth, desktop portals, and SDDM.

The default system settings use the US keyboard layout, `en_US.UTF-8`, and the `Asia/Tokyo` timezone. Both English and Japanese UTF-8 locales are generated. Use `--no-services` to keep system settings unchanged.

## Options

Run these commands from the repository directory:

```bash
bash setup.sh --dry-run      # Preview the setup without making changes.
bash setup.sh --no-aur       # Skip AUR packages; a fresh installation uses Waybar.
bash setup.sh --no-services  # Skip service and system configuration.
```

The default setup builds Tide Island from the AUR using `makepkg`. Official packages are installed with a full system upgrade through `pacman -Syu`. Arch Linux packages follow rolling updates. Rust dependencies are pinned in `Cargo.lock`; Neovim plugins are recorded in `lazy-lock.json` and downloaded on first launch.

## Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| Super+T | Open Ghostty |
| Super+E | Open Dolphin |
| Super+A or Super+Space | Open the application launcher |
| Super+Q | Close the active window |
| Super+L | Lock the session |
| Super+Shift+S | Capture a screen region |
| Super+1–0 | Switch workspaces |
| Super+Shift+1–0 | Move a window to a workspace |
| Alt+Tab | Switch windows |

The keyd configuration maps a CapsLock tap to Ctrl+Space. Hold CapsLock with h/j/k/l for arrow keys, or with a/d to switch to TTY1/TTY2.

## Backups and restore

Changed user files are backed up to:

```text
~/.local/state/arch-dotfiles/backups/<timestamp>/files/
```

Files with identical content are left in place when the setup runs again. Restore a backup with:

```bash
python scripts/deploy.py --restore "$HOME/.local/state/arch-dotfiles/backups/<timestamp>"
```

Replace `<timestamp>` with the backup directory name. Restore stops if a deployed file has been edited after installation. It restores user files only; installed packages and system settings remain.

Existing system configuration files are saved under `/var/lib/arch-dotfiles/backups/<timestamp>/` before replacement.

## Local configuration

Add machine-specific Hyprland settings to `~/.config/hypr/local.lua`. The setup leaves this file untouched.

```lua
hl.monitor({ output = "eDP-1", mode = "preferred", position = "auto", scale = 1.5 })
```

## References

- [Hyprland configuration](https://wiki.hypr.land/Configuring/Start/)
- [Arch Linux system maintenance](https://wiki.archlinux.org/title/System_maintenance)
- [Hyprland desktop portals](https://wiki.archlinux.org/title/Hyprland)

## License

Project code and configurations are provided under the [MIT License](LICENSE). Vendored components retain their own copyright notices and licenses, including [grim](src/dock/rust/helpers/grim-v1.5.0/LICENSE) and the [Neovim starter configuration](home/.config/nvim/LICENSE). Installed packages are covered by their respective licenses.
