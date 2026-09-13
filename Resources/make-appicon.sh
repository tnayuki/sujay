#!/bin/sh
# Rasterises the icon SVGs into Assets.xcassets/AppIcon.appiconset.
# Run by hand after editing AppIcon.svg or AppIcon-small.svg; the build does not run it.
# Needs rsvg-convert (brew install librsvg).
#
# Every slot gets its own file even where two slots are the same number of
# pixels: actool drops sizes that share a filename, and the .icns it writes
# then comes out missing 32, 512 and 1024.
set -eu

cd "$(dirname "$0")"
out=Assets.xcassets/AppIcon.appiconset

# AppIcon-small.svg drops the platter grooves and thickens the waveform ring so
# that the 16 and 32 pixel slots stay legible.
render() { # <pixels> <name>
  case "$1" in
    16|32) src=AppIcon-small.svg ;;
    *)     src=AppIcon.svg ;;
  esac
  rsvg-convert -w "$1" -h "$1" -o "$out/$2" "$src"
}

render 16   icon_16x16.png
render 32   icon_16x16@2x.png
render 32   icon_32x32.png
render 64   icon_32x32@2x.png
render 128  icon_128x128.png
render 256  icon_128x128@2x.png
render 256  icon_256x256.png
render 512  icon_256x256@2x.png
render 512  icon_512x512.png
render 1024 icon_512x512@2x.png

echo "wrote $out"
