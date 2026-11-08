#!/bin/sh
# Render the checked-in official SVG paths to the shell's 128×128 UiImage assets.
set -eu

assets=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
for name in \
    knave-color-dark \
    knave-color-light \
    knave-monochrome-dark \
    knave-monochrome-light \
    knave-color-light-high-contrast
do
    rsvg-convert --format png --width 128 --height 128 \
        --output "$assets/$name.png" "$assets/$name.svg"
done
