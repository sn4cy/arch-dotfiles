#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
source_dir="$project_dir/helpers/grim-v1.5.0"
build_dir="$source_dir/build"
if [ -f "$build_dir/meson-private/coredata.dat" ]; then
    meson setup --reconfigure "$build_dir" "$source_dir" --buildtype=release -Dman-pages=disabled
else
    meson setup "$build_dir" "$source_dir" --buildtype=release -Dman-pages=disabled
fi
meson compile -C "$build_dir" -j 2
if "$build_dir/test-capture-failure"; then
    echo 'Capture failure regression test unexpectedly succeeded' >&2
    exit 1
else
    helper_status=$?
    [ "$helper_status" -eq 1 ] || exit 1
fi
printf 'PASS: NULL output capture failure exits cleanly (status 1)\n'
