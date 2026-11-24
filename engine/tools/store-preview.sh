#!/usr/bin/env bash
# The store's five-second gameplay clip, made from the recipe in the game's
# folder and published with your own publishing key. Guide:
# engine/docs/store-previews.md.
#
#   engine/tools/store-preview.sh games/spellstorm             # render and check
#   GAMENIGHT_API_TOKEN=gn_pub_... engine/tools/store-preview.sh games/spellstorm --publish
#
# The clip lands in build/store-preview/GAME (never committed): preview.mp4,
# preview.jpg (its poster), the 150 edit frames and provenance.txt. With
# --publish both files go to the Splitscreen Club API (stored on its S3) and
# become the store page's preview video and poster; nothing else on the page
# changes.
set -euo pipefail

usage() { echo "usage: $0 GAME_DIR [--publish] [--server URL]" >&2; exit 2; }
[ $# -ge 1 ] || usage
game_dir=$1
shift
publish=false
server=${GAMENIGHT_SERVER:-https://gamenight.ontola.io}
while [ $# -gt 0 ]; do
  case $1 in
    --publish) publish=true ;;
    --server) server=${2:?--server URL}; shift ;;
    *) usage ;;
  esac
  shift
done

ROOT=$(cd "$(git rev-parse --show-toplevel)" && pwd)
GAME_DIR=$(cd "$game_dir" && pwd)
recipe=$GAME_DIR/store/preview/recipe.sh
[ -f "$recipe" ] || recipe=$GAME_DIR/preview/recipe.sh
[ -f "$recipe" ] || { echo "no recipe: $GAME_DIR/store/preview/recipe.sh" >&2; exit 1; }
manifest=$GAME_DIR/gamenight.json
[ -f "$manifest" ] || manifest=$GAME_DIR/store/gamenight.json
[ -f "$manifest" ] || { echo "no gamenight.json in $GAME_DIR or its store/" >&2; exit 1; }
id=$(sed -n 's/.*"id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$manifest" | head -1)
[ -n "$id" ] || { echo "no \"id\" in $manifest" >&2; exit 1; }

if $publish; then
  case ${GAMENIGHT_API_TOKEN:-} in
    gn_pub_*) ;;
    *) echo "set GAMENIGHT_API_TOKEN to a publishing key for $id (developer portal)" >&2; exit 1 ;;
  esac
fi

OUT=$ROOT/build/store-preview/$id
EDIT=$OUT/edit
rm -rf "$OUT"
mkdir -p "$EDIT"
export ROOT GAME_DIR OUT EDIT

# For the recipe: append frames FIRST..LAST (0-based, inclusive) of a
# sequence folder (00000.png and on) to the edit.
cut() {
  local dir=$1 first=$2 last=$3 i n
  n=$(find "$EDIT" -name '*.png' | wc -l)
  for ((i = first; i <= last; i++)); do
    cp "$dir/$(printf %05d "$i").png" "$EDIT/$(printf %05d "$n").png"
    n=$((n + 1))
  done
}

# The recipe defines `capture`: it renders its shots into $OUT, cuts 150
# frames (960x540) into $EDIT with `cut`, and sets POSTER to one of them.
POSTER=
# shellcheck source=/dev/null
source "$recipe"
capture
[ -n "$POSTER" ] && [ -f "$EDIT/$POSTER" ] || { echo "the recipe set no POSTER frame in $EDIT" >&2; exit 1; }

frames=$(find "$EDIT" -name '*.png' | wc -l)
[ "$frames" -eq 150 ] || { echo "the edit has $frames frames, the store takes exactly 150 (5 s at 30 fps)" >&2; exit 1; }
distinct=$(for f in "$EDIT"/*.png; do md5sum < "$f"; done | sort -u | wc -l)
[ "$distinct" -ge 100 ] || { echo "only $distinct distinct frames: is it moving?" >&2; exit 1; }

# The store's encoding (gamepad-party docs/gameplay-previews.md).
ffmpeg -loglevel error -y -framerate 30 -i "$EDIT/%05d.png" -c:v libx264 -crf 24 -preset slow \
  -pix_fmt yuv420p -color_range tv -movflags +faststart -an "$OUT/preview.mp4"
ffmpeg -loglevel error -y -i "$EDIT/$POSTER" -q:v 3 "$OUT/preview.jpg"

got=$(ffprobe -v error -count_frames -select_streams v:0 \
  -show_entries stream=width,height,r_frame_rate,nb_read_frames -of csv=p=0 "$OUT/preview.mp4")
[ "$got" = "960,540,30/1,150" ] || { echo "clip is $got, want 960,540,30/1,150" >&2; exit 1; }

commit=$(git -C "$ROOT" rev-parse HEAD)
git -C "$ROOT" diff --quiet HEAD -- || commit="$commit (with uncommitted changes)"
{
  echo "game: $id"
  echo "commit: $commit"
  echo "recipe: ${recipe#"$ROOT"/}"
  echo "made: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  (cd "$OUT" && sha256sum preview.mp4 preview.jpg)
} > "$OUT/provenance.txt"
cat "$OUT/provenance.txt"
echo "watch it before publishing: $OUT/preview.mp4"

$publish || exit 0

api=$server/v1/developer/games/$id
upload() {
  local reply
  reply=$(curl -sS --fail-with-body -X POST -H "Authorization: Bearer $GAMENIGHT_API_TOKEN" \
    -H "Content-Type: application/octet-stream" --data-binary "@$1" "$api/uploads")
  sed -n 's/.*"url"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' <<< "$reply"
}
video=$(upload "$OUT/preview.mp4")
poster=$(upload "$OUT/preview.jpg")
[ -n "$video" ] && [ -n "$poster" ] || { echo "upload gave no url" >&2; exit 1; }
curl -sS --fail-with-body -X PUT -H "Authorization: Bearer $GAMENIGHT_API_TOKEN" -H "Content-Type: application/json" \
  --data "{\"video\":\"$video\",\"poster\":\"$poster\"}" "$api/store"
echo
{ echo "video: $video"; echo "poster: $poster"; } | tee -a "$OUT/provenance.txt"
