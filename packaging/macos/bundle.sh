#!/bin/sh
# Builds dist/Kilo.app. With --install, also copies it to /Applications.
set -eu
cd "$(dirname "$0")/../.."
cargo build --release -p kilo-mac
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
if [ "${1:-}" = "--install" ]; then
    rm -rf /Applications/Kilo.app
    cp -R "$APP" /Applications/Kilo.app
    echo "Installed /Applications/Kilo.app"
fi
du -sh "$APP"
