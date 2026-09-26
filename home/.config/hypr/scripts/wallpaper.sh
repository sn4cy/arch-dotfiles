#!/bin/sh
for attempt in 1 2 3 4 5; do
    awww img "$HOME/.local/share/arch-dotfiles/wallpaper.png" --transition-type none && exit 0
    sleep 1
done
exit 1
