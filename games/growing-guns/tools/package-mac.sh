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
# Signs ad hoc unless SIGN_IDENTITY names a codesigning identity (a Developer
# ID with notarisation is what lets other Macs open it without a warning).
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

# The game looks beside its executable; point it at Resources instead, and
# keep screenshots and replays out of the (read-only, signed) bundle.
cat > "$app/Contents/MacOS/launch" <<'EOF'
#!/bin/sh
here=$(cd "$(dirname "$0")" && pwd)
r="$here/../Resources"
data="$HOME/Library/Application Support/Growing Guns"
mkdir -p "$data"
cd "$data"
export GG_SOUNDS="$r/audio" GG_MUSIC="$r/music" GG_SFX="$r/sounds" GG_MODS="$r/mods"
export GG_CHARACTERS="$r/assets/characters" WARP_AUDIO_IR_DIR="$r/engine/assets/sound/ir"
export WARP_SCREENSHOTS="$data/screenshots"
[ -d "$r/audio/crowd" ] && export GG_CROWD="$r/audio/crowd"
exec "$here/growing-guns" play --fullscreen "$@"
EOF
chmod +x "$app/Contents/MacOS/launch" "$app/Contents/MacOS/growing-guns"

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
    <key>CFBundleExecutable</key><string>launch</string>
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
 "scope": "Native macOS app bundle. Without a Godot project the footsteps, hurts, deaths, crowd and music are silent. Packaging is not proof of GPU acceptance."}
EOF

# Copied files carry Finder/provenance attributes that codesign rejects.
xattr -cr "$app"
codesign --force --deep --sign "${SIGN_IDENTITY:--}" "$app"
codesign --verify --deep --strict "$app"

# Drag-to-Applications disk image.
stage=$(mktemp -d)
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
hdiutil create -quiet -volname "Growing Guns" -srcfolder "$stage" -fs HFS+ -format UDZO "$destination/growing-guns-mac-$commit.dmg"
echo "$destination/growing-guns-mac-$commit.dmg"
