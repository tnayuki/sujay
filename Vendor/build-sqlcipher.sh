#!/bin/sh
# Rebuild Vendor/CSQLCipher.xcframework from a pinned SQLCipher release.
#
# rekordbox's master.db is a SQLCipher database, so reading it needs SQLCipher rather than
# the system libsqlite3. It is built against CommonCrypto (SQLCIPHER_CRYPTO_CC) instead of
# OpenSSL, which is what keeps this to one self-contained static archive with nothing to
# ship alongside it — the crypto comes from the Security framework the app already links.
#
# The result, CSQLCipher.xcframework, is a committed static asset (like Resources/sujay.icns):
# the build is not part of xcodebuild — run this by hand to bump the version. SQLCipher
# changes a few times a year, and the amalgamation takes minutes to produce, so there is
# nothing to gain from rebuilding it on every compile.
#
# Needs nothing but a C compiler: SQLCipher's configure builds the `jimsh` interpreter its
# amalgamation script runs on, so there is no Tcl to install.
set -eu

SQLCIPHER_VERSION="4.19.0"
DEPLOYMENT_TARGET="15.0"

# SQLITE_HAS_CODEC turns on the encryption layer, SQLCIPHER_CRYPTO_CC selects CommonCrypto as
# its provider, and SQLITE_TEMP_STORE=2 keeps temporary tables in memory so plaintext never
# reaches the disk. The rest is what SQLCipher's own build recommends.
# SQLITE_EXTRA_INIT/SHUTDOWN are not optional: SQLCipher #errors out without them, because
# that is how it hooks its codec into sqlite3_initialize.
DEFINES="-DSQLITE_HAS_CODEC -DSQLCIPHER_CRYPTO_CC -DSQLITE_TEMP_STORE=2 -DSQLITE_THREADSAFE=1 -DSQLITE_EXTRA_INIT=sqlcipher_extra_init -DSQLITE_EXTRA_SHUTDOWN=sqlcipher_extra_shutdown"

HERE="$(cd "$(dirname "$0")" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "==> Fetching SQLCipher v$SQLCIPHER_VERSION"
curl -fsSL "https://github.com/sqlcipher/sqlcipher/archive/refs/tags/v${SQLCIPHER_VERSION}.tar.gz" \
  | tar xz -C "$WORK"
SRC="$WORK/sqlcipher-${SQLCIPHER_VERSION}"

echo "==> Generating the amalgamation"
(
  cd "$SRC"
  ./configure --with-tempstore=yes >/dev/null
  make sqlite3.c >/dev/null
)

echo "==> Compiling (arm64 + x86_64)"
LIBS=""
for arch in arm64 x86_64; do
  # shellcheck disable=SC2086
  clang -arch "$arch" -mmacosx-version-min="$DEPLOYMENT_TARGET" -O2 $DEFINES \
    -c "$SRC/sqlite3.c" -o "$WORK/sqlite3-$arch.o"
  ar rcs "$WORK/libsqlcipher-$arch.a" "$WORK/sqlite3-$arch.o"
  LIBS="$LIBS $WORK/libsqlcipher-$arch.a"
done
# shellcheck disable=SC2086
lipo -create $LIBS -output "$WORK/libsqlcipher.a"

echo "==> Assembling headers + module map"
HEADERS="$WORK/headers"
mkdir -p "$HEADERS"
cp "$SRC/sqlite3.h" "$HEADERS/"
cat > "$HEADERS/module.modulemap" <<'MODULEMAP'
module CSQLCipher {
    header "sqlite3.h"
    export *
}
MODULEMAP

echo "==> Packaging CSQLCipher.xcframework"
rm -rf "$HERE/CSQLCipher.xcframework"
xcodebuild -create-xcframework \
  -library "$WORK/libsqlcipher.a" -headers "$HEADERS" \
  -output "$HERE/CSQLCipher.xcframework"

echo "==> Done: $HERE/CSQLCipher.xcframework (SQLCipher $SQLCIPHER_VERSION)"
