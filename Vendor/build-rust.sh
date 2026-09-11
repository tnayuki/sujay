#!/bin/sh
# Build the Rust core as a static library for the Xcode target.
#
# Run by the "Build Rust core" script phase of sujay.xcodeproj before Swift
# compiles. Produces .build/rust/$CONFIGURATION/libsujay_ffi.a for every
# architecture in $ARCHS (lipo'd together), which LIBRARY_SEARCH_PATHS points at.
# Unlike hukan's vendored xcframeworks this is built every time, because the
# Rust core changes in the same commits as the Swift that drives it; cargo is
# incremental, so a no-change run is a second.
#
# A universal Release build needs `rustup target add x86_64-apple-darwin`.
set -eu

cd "${SRCROOT:-$(dirname "$0")/..}"
export PATH="$HOME/.cargo/bin:$PATH"

configuration="${CONFIGURATION:-Debug}"
case "$configuration" in
  Release) profile=release; cargo_flags="--release" ;;
  *)       profile=debug;   cargo_flags="" ;;
esac

out="$PWD/.build/rust/$configuration"
mkdir -p "$out"

libs=""
for arch in ${ARCHS:-arm64}; do
  case "$arch" in
    arm64)  target=aarch64-apple-darwin ;;
    x86_64) target=x86_64-apple-darwin ;;
    *) echo "build-rust.sh: unsupported arch $arch" >&2; exit 1 ;;
  esac
  # shellcheck disable=SC2086
  cargo build -p sujay-ffi --target "$target" $cargo_flags
  libs="$libs target/$target/$profile/libsujay_ffi.a"
done

# shellcheck disable=SC2086
lipo -create $libs -output "$out/libsujay_ffi.a"
echo "build-rust.sh: $out/libsujay_ffi.a ($ARCHS)"
