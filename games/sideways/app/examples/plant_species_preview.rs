//! Retired v1 flattened species gallery. Its immutable captures remain historical.
//! Detailed assemblies are rendered by tree_species_compare and plant_spray_gallery.
fn main() {
    eprintln!("plant_species_preview was replaced: use tree_species_compare --root GAME_ROOT --out FRESH_DIRECTORY --candidate spruce, or plant_spray_gallery --root GAME_ROOT --out FRESH_DIRECTORY. The old whole-tree flattening preview is disabled for detailed assemblies.");
    std::process::exit(1);
}
