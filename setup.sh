#!/usr/bin/env bash
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
dry=0; aur=1; services=1
for arg in "$@"; do
    case "$arg" in
        --dry-run) dry=1 ;;
        --no-aur) aur=0 ;;
        --no-services) services=0 ;;
        --help) printf 'Usage: bash setup.sh [--dry-run] [--no-aur] [--no-services]\n'; exit 0 ;;
        *) printf 'Unknown option: %s\n' "$arg" >&2; exit 2 ;;
    esac
done
mapfile -t packages < "$repo/packages/official.txt"
if ((dry)); then
    printf 'Official packages (%s):\n%s\n' "${#packages[@]}" "${packages[*]}"
    ((aur == 0)) || printf 'AUR packages: %s\n' "$(cat "$repo/packages/aur.txt")"
    printf 'Build: fast-alt-tab, floating-dock, patched grim\nDeploy: %s/home -> %s (back up changed files)\n' "$repo" "$HOME"
    printf 'Services on next boot: NetworkManager, bluetooth, sddm, systemd-timesyncd, keyd; locale en_US.UTF-8 + ja_JP.UTF-8; timezone Asia/Tokyo (enabled=%s)\n' "$services"
    exit 0
fi
[[ $(id -u) != 0 ]] || { printf 'Run as the target regular user with sudo privileges.\n' >&2; exit 1; }
[[ -f /etc/arch-release ]] || { printf 'Arch Linux is required.\n' >&2; exit 1; }
[[ ${XDG_CONFIG_HOME:-"$HOME/.config"} == "$HOME/.config" ]] || { printf 'This repository expects XDG_CONFIG_HOME=$HOME/.config.\n' >&2; exit 1; }
command -v sudo >/dev/null || { printf 'Install sudo and configure privileges first. See README.md.\n' >&2; exit 1; }
if ((services)); then
    for other in gdm lightdm greetd; do
        if systemctl is-enabled --quiet "$other.service" 2>/dev/null; then
            printf 'Another display manager is enabled (%s); use --no-services or disable it first.\n' "$other" >&2
            exit 1
        fi
    done
fi
sudo -v
sudo pacman -Syu --needed --noconfirm "${packages[@]}"
if ((aur)); then
    build_root=$(mktemp -d)
    trap 'rm -rf -- "$build_root"' EXIT
    while IFS= read -r package; do
        [[ -n $package ]] || continue
        # PKGBUILDs execute code as this regular user. See README before running.
        git clone --depth=1 "https://aur.archlinux.org/$package.git" "$build_root/$package"
        (cd "$build_root/$package" && makepkg -si --needed --noconfirm)
    done < "$repo/packages/aur.txt"
fi
build_dir="$repo/.build"
mkdir -p "$build_dir"
cc -O2 -Wall -Wextra -Werror "$repo/src/fast-alt-tab/client.c" -o "$build_dir/fast-alt-tab"
CARGO_TARGET_DIR="$build_dir/cargo" cargo build --release --locked --jobs 2 --manifest-path "$repo/src/dock/rust/Cargo.toml"
sh "$repo/src/dock/rust/build-capture-helper.sh"
python "$repo/scripts/deploy.py" --home "$HOME" --build "$build_dir"
if ((services)); then
    system_backup="/var/lib/arch-dotfiles/backups/$(date +%Y%m%dT%H%M%S.%N)"
    sudo mkdir -p "$system_backup"
    sudo chmod 700 "$system_backup"
    for config in /etc/locale.gen /etc/locale.conf /etc/localtime /etc/vconsole.conf /etc/keyd/default.conf /etc/sddm.conf.d/90-arch-dotfiles.conf; do
        if sudo test -e "$config"; then sudo cp -a --parents "$config" "$system_backup/"; fi
    done
    sudo sed -i -E 's/^#[[:space:]]*((en_US|ja_JP)\.UTF-8[[:space:]]+UTF-8)/\1/' /etc/locale.gen
    sudo locale-gen
    sudo localectl set-locale LANG=en_US.UTF-8
    sudo localectl set-keymap us
    sudo timedatectl set-timezone Asia/Tokyo
    sudo install -Dm644 "$repo/system/keyd.conf" /etc/keyd/default.conf
    sudo install -Dm644 "$repo/system/sddm.conf" /etc/sddm.conf.d/90-arch-dotfiles.conf
    sudo systemctl enable NetworkManager.service bluetooth.service sddm.service systemd-timesyncd.service keyd.service
fi
# No running desktop is restarted during setup.
printf '\nSetup complete. Reboot, select Hyprland in SDDM, and log in.\n'
