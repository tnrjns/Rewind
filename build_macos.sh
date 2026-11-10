#!/bin/sh
# Builds dist/Rewind.dmg for Apple Silicon Macs.
set -e
cd "$(dirname "$0")"

mkdir -p bin
swiftc -O -target arm64-apple-macos13 macos/rewind-audio.swift -o bin/rewind-audio
# FFmpeg isn't bundled: Rewind downloads it on first launch, like the Windows installer does

python3 -m PyInstaller --noconfirm --clean Rewind-macos.spec
codesign --force --deep --sign - dist/Rewind.app

rm -rf dist/dmg dist/Rewind.dmg
mkdir -p dist/dmg
cp -R dist/Rewind.app dist/dmg/
ln -s /Applications dist/dmg/Applications
hdiutil create -volname Rewind -srcfolder dist/dmg -ov -format UDZO dist/Rewind.dmg
rm -rf dist/dmg
echo "Built dist/Rewind.dmg"
