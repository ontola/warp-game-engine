#!/bin/sh
# Rebuild app/assets/characters from the free source packs (see its README).
# Usage: tools/import-skins.sh DIR, with the KayKit Skeletons files in DIR/kaykit:
# Characters/gltf/*.glb, Assets/gltf/*.gltf and .bin, and skeleton_texture.png.
set -e
K="$1/kaykit"
OUT="$(dirname "$0")/../app/assets/characters"
conv() { cargo run --locked --release -p assets --example gltf_character -- "$@"; }
common="--clip idle=Idle --clip hit=Hit_A! --clip die=Death_C_Skeletons!"
conv "$K/Skeleton_Warrior.glb" --out "$OUT" --name skeleton --scale 0.7 $common \
  --clip walk=Walking_D_Skeletons --clip attack=1H_Melee_Attack_Chop! \
  --attach "$K/Skeleton_Blade.gltf=handslot.r"
conv "$K/Skeleton_Rogue.glb" --out "$OUT" --name bone_archer --scale 0.7 $common \
  --clip walk=Walking_C --clip attack=2H_Ranged_Shoot! \
  --attach "$K/Skeleton_Crossbow.gltf=handslot.r"
conv "$K/Skeleton_Mage.glb" --out "$OUT" --name necromancer --scale 0.75 $common \
  --clip walk=Walking_A --clip attack=Spellcast_Shoot! \
  --attach "$K/Skeleton_Staff.gltf=handslot.r"
conv "$K/Skeleton_Warrior.glb" --out "$OUT" --name gravekeeper --scale 0.7 $common \
  --clip walk=Walking_D_Skeletons --clip attack=2H_Melee_Attack_Chop! \
  --attach "$K/Skeleton_Axe.gltf=handslot.r"
