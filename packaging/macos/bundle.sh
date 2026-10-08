#!/bin/sh
# Builds dist/Kilo.app. With --install, also copies it to /Applications.
set -eu
cd "$(dirname "$0")/../.."
# --locked: build exactly what Cargo.lock says (CI does too).
cargo build --release --locked -p kilo-mac
APP=dist/Kilo.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/Kilo "$APP/Contents/MacOS/Kilo"
cp packaging/macos/Info.plist "$APP/Contents/Info.plist"
cp packaging/macos/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
# Ad-hoc signature: enough to run locally. Distribution needs a Developer ID.
# The hardened runtime keeps other programs from injecting code into Kilo
# (DYLD_* variables, unsigned libraries) or attaching to it to read the
# session out of its memory. Kilo needs none of its exceptions.
codesign --force --options runtime --sign - "$APP"
codesign --verify --strict "$APP"
if [ "${1:-}" = "--install" ]; then
    # Copied next to the old app first, then swapped in: a failed copy
    # leaves the installed app as it was.
    rm -rf /Applications/.Kilo.app.new
    cp -R "$APP" /Applications/.Kilo.app.new
    rm -rf /Applications/Kilo.app
    mv /Applications/.Kilo.app.new /Applications/Kilo.app
    echo "Installed /Applications/Kilo.app"
fi
du -sh "$APP"
