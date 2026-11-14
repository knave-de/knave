# Knave logo assets

`knave-color-light.svg` and `knave-color-dark.svg` are the transparent light- and
dark-theme marks. The monochrome and high-contrast SVGs use the same paths and
viewBox with palette changes for the shell accessibility policies.

The SVGs are the source of truth. `render-brand-assets.sh` renders the themed
variants to checked-in 128×128 transparent PNGs because the shell image API
consumes RGBA pixels. The shell loads those PNGs and does not need an SVG
renderer at runtime. After editing an SVG, regenerate the PNGs with
`./render-brand-assets.sh` (requires `rsvg-convert`).
