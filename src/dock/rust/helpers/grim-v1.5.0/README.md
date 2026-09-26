# grim

Grab images from a Wayland compositor. Works great with [slurp].

## Example usage

Screenshoot all outputs:

```sh
grim
```

Screenshoot a specific output:

```sh
grim -o DP-1
```

Screenshoot a region:

```sh
grim -g "10,20 300x400"
```

Select a region and screenshoot it:

```sh
grim -g "$(slurp)"
```

Use a custom filename:

```sh
grim $(xdg-user-dir PICTURES)/$(date +'%s_grim.png')
```

Screenshoot and copy to clipboard:

```sh
grim - | wl-copy
```

Grab a screenshot from the focused monitor under Sway, using `swaymsg` and
`jq`:

```sh
grim -o "$(swaymsg -t get_outputs | jq -r '.[] | select(.focused) | .name')"
```

Grab a screenshot from the focused window under Sway, using `swaymsg` and
`jq`:

```sh
grim -T "$(swaymsg -t get_tree | jq -j '.. | select(.type?) | select(.focused).foreign_toplevel_identifier')"
```

Pick a color, using ImageMagick:

```sh
grim -g "$(slurp -p)" -t ppm - | convert - -format '%[pixel:p{0,0}]' txt:-
```

## Building from source

Install dependencies:

* meson
* wayland
* pixman
* libpng
* libjpeg (optional)

Then run:

```sh
meson build
ninja -C build
```

To run directly, use `build/grim`, or if you would like to do a system
installation (in `/usr/local` by default), run `ninja -C build install`.

## Contributing

Report bugs and send patches on [GitLab].

Join the IRC channel: [#emersion on Libera Chat].

## License

MIT

[slurp]: https://github.com/emersion/slurp
[GitLab]: https://gitlab.freedesktop.org/emersion/grim
[#emersion on Libera Chat]: ircs://irc.libera.chat/#emersion
