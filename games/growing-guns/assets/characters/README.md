# Growing Guns player rig

Warp Humans generates the player geometry and materials. `player-rig.json` holds the joint names and inverse bind transforms. `player.anim.bin` holds only a two-frame rest clip (the bind pose). Gameplay fits each seeded civilian to the shared skeleton and animates it with Warp's body library.

The skeleton comes from the original Growing Guns project's Mixamo-rigged knight. Its pistol animation clips were removed; all animation comes from the body crate's CC0 library. The source checkout was based on `28cca1617ba0b5e8d0dbb76baf34837c4a6b5f65` with local changes. `provenance.json` records the source hashes and the two shipped files. The upstream software notice is in `SOURCE-SOFTWARE-LICENSE.txt`. Third-party asset rights remain with their original owners.
