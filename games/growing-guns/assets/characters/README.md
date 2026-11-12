# Growing Guns player rig

Warp Humans generates the player geometry and materials. `player-rig.json` holds the joint names, inverse bind transforms, clip bounds and socket tracks. `player.anim.bin` contains the sampled pistol idle, run and jump palettes used by the animation tools. Gameplay fits each seeded civilian to the shared skeleton and animates it with Warp's body library.

The rig and clips come from the original Growing Guns project's Mixamo-rigged knight and pistol animations. The source checkout was based on `28cca1617ba0b5e8d0dbb76baf34837c4a6b5f65` with local changes. `provenance.json` records the source hashes and the two shipped files. The upstream software notice is in `SOURCE-SOFTWARE-LICENSE.txt`. Third-party asset rights remain with their original owners.
