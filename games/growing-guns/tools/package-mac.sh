#!/bin/sh
# Growing Guns as a macOS app in a drag-to-Applications disk image:
#   Growing Guns.app with the game in Contents/MacOS and assets/characters,
#   mods, sounds (the patches), audio and music (the Godot project's
#   recordings and stems, taken from a Godot checkout because they are not in
#   this repository) and the engine's reverb in Contents/Resources.
#
#   cargo build --release -p growing-guns-app
#   games/growing-guns/tools/package-mac.sh target/release/growing-guns ~/dev/gg-package ~/dev/growing-guns
#
# Signs ad hoc by default. For other Macs to open it without a warning, sign
# with a Developer ID and notarise with a notarytool keychain profile (made
# once with `xcrun notarytool store-credentials NAME`):
#   SIGN_IDENTITY="Developer ID Application: Argu B.V." NOTARY_PROFILE=NAME \
#       games/growing-guns/tools/package-mac.sh ...
set -eu
[ $# -ge 2 ] || { echo "usage: $0 EXECUTABLE OUT_DIRECTORY [GODOT_PROJECT]" >&2; exit 2; }
executable=$1
destination=$2
godot=${3:-}
game_root=$(cd "$(dirname "$0")/.." && pwd)
repo_root=$(cd "$game_root/../.." && pwd)
[ -e "$destination" ] && { echo "Package output must be a new directory" >&2; exit 1; }
version=$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' "$game_root/gamenight.json" | head -n 1)
commit=$(git -C "$repo_root" rev-parse --short HEAD 2>/dev/null || echo unknown)
# Without a catalogue version, the commit date (2026.11.3).
[ -n "$version" ] || version=$(git -C "$repo_root" log -1 --format=%cd --date=format:%Y.%-m.%-d 2>/dev/null || echo 0.0.0)

app="$destination/Growing Guns.app"
resources="$app/Contents/Resources"
mkdir -p "$app/Contents/MacOS" "$resources/assets" "$resources/sounds" "$resources/engine/assets/sound"
cp "$executable" "$app/Contents/MacOS/growing-guns"
cp -R "$game_root/assets/characters" "$resources/assets/"
cp -R "$game_root/mods" "$resources/"
cp "$game_root/gamenight.json" "$resources/"
cp "$game_root"/sounds/*.luau "$resources/sounds/"
cp -R "$repo_root/engine/assets/sound/ir" "$resources/engine/assets/sound/"
recordings=none
if [ -n "$godot" ]; then
    mkdir -p "$resources/audio" "$resources/music"
    cp "$godot"/assets/audio/*.wav "$resources/audio/"
    cp "$godot"/assets/music/*.wav "$resources/music/"
    if [ -d "$godot/addons/crowd_sound/samples" ]; then cp -R "$godot/addons/crowd_sound/samples" "$resources/audio/crowd"; fi
    recordings=$(cd "$godot" && pwd)
fi

# The game finds Contents/Resources itself and keeps screenshots, replays
# and its pipeline cache in Application Support (`use_app_bundle`).
chmod +x "$app/Contents/MacOS/growing-guns"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Growing Guns</string>
    <key>CFBundleDisplayName</key><string>Growing Guns</string>
    <key>CFBundleIdentifier</key><string>io.ontola.growing-guns</string>
    <key>CFBundleVersion</key><string>$version.$commit</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleExecutable</key><string>growing-guns</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.action-games</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

# The logo, centred on a transparent square, at every icon size.
iconset=$(mktemp -d)/AppIcon.iconset
mkdir -p "$iconset"
sips -s format png -Z 900 "$game_root/assets/branding/logo.png" --out "$iconset/logo.png" >/dev/null
sips --padToHeightWidth 1024 1024 "$iconset/logo.png" --out "$iconset/icon_512x512@2x.png" >/dev/null
rm "$iconset/logo.png"
for size in 16 32 128 256 512; do
    sips -z $size $size "$iconset/icon_512x512@2x.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    [ $size -lt 512 ] && sips -z $((size * 2)) $((size * 2)) "$iconset/icon_512x512@2x.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$resources/AppIcon.icns"

cat > "$resources/package.json" <<EOF
{"executable_sha256": "$(shasum -a 256 "$executable" | cut -d' ' -f1)", "commit": "$commit", "recordings_from": "$recordings",
 "scope": "Native macOS app bundle. Without a Godot project the footsteps, hurts, deaths and music are silent. Packaging is not proof of GPU acceptance."}
EOF

# Copied files carry Finder/provenance attributes that codesign rejects.
xattr -cr "$app"
identity=${SIGN_IDENTITY:--}
if [ "$identity" = - ]; then
    codesign --force --sign - "$app"
else
    # Notarisation needs the hardened runtime and a secure timestamp. The game
    # needs no entitlements: no JIT (Luau interprets), no camera/microphone.
    codesign --force --options runtime --timestamp --sign "$identity" "$app"
fi
codesign --verify --deep --strict "$app"

# Drag-to-Applications disk image.
dmg="$destination/growing-guns-mac-$commit.dmg"
stage=$(mktemp -d)
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
hdiutil create -quiet -volname "Growing Guns" -srcfolder "$stage" -fs HFS+ -format UDZO "$dmg"
if [ "$identity" != - ]; then codesign --force --timestamp --sign "$identity" "$dmg"; fi
if [ -n "${NOTARY_PROFILE:-}" ]; then
    xcrun notarytool submit "$dmg" --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple "$dmg"
    spctl --assess --type open --context context:primary-signature -vv "$dmg"
fi
echo "$dmg"
