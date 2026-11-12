# Procedural contacts on animated characters

Start with the locomotion or action clip. Sample it once per actor, blend it, then use a contact to put a wrist at a point on a prop. Rigid and skinned renderers can consume the corrected palette. Keep the solve in the presentation code.

For a standard humanoid, bind the arm once when loading the actor. At render time, express the prop socket in the same model space as the sampled palette. This example assumes the gun socket represents the wrist and uses a model-space outward direction as the fallback elbow hint.

```rust
let support_arm = body::reach::Reach::arm(model.joints(), true).ok_or("missing left arm")?;
// After sampling the current clip into palette:
let wrist = model_to_world.inverse().transform_point3(gun_to_world.transform_point3(socket));
support_arm.apply(&mut palette, wrist, outward, grip_weight);
```

Each solve measures the upper and lower segment lengths in the current pose. It keeps the sampled elbow bend plane where possible, preserves hand orientation, carries finger joints along, and clamps unreachable targets. Weight ranges from zero (authored clip) to one (full reach). For a differently named skeleton, use `Reach::bind` to map both pivots and affected descendants once. Use the same palette layout at sampling and binding.

For a right-handed prop use `TwoHandHold::bind(model.joints())` once and `hold.apply(palette, primary, secondary)` after sampling. Use `with_arm_proportions(upper, forearm)` at bind time for a character-specific held-pose correction. Both reach limits and mesh deformation use those factors along the bone axes. Place the prop at the returned primary wrist. The secondary hand reaches as far as the current skeleton allows.

Growing Guns anchors the weapon to the animated right wrist, pitches around the grip and uses the left arm to hold the magazine and transfer onto the growing foregrip. The first-person hands follow the weapon sockets. For an identical-camera comparison of the sampled clip and corrected grip, run from the repository root:

```sh
cargo run -p growing-guns-app -- gun-review --grip --seed 3 --card sniper --clip pistol_run --time 0.24 --pitch 12 --out build/grip-review
```

Inspect `clip.png` and `grip.png`. Use `--seed` to inspect another civilian. Omit the card for the base gun. Change time to sample the clip in seconds and pitch to aim in degrees. The support palm turns inward against the attachment. In the game, the trigger hand takes the floor rifle first and the support hand follows. The pickup switches between open and gripping hand meshes. Individual finger curls remain an authoring task.

Add `--growth --card sniper` to the grip review to capture 25 frames over the two-second weapon morph. The camera stays fixed relative to the character, and clip time stays fixed so the images isolate the grip response.

## Live Luau preview

Run `cargo run -p growing-guns-app -- animation-preview` for a windowed growth loop. Pass `--seed 52` to change character. Save edits to `games/growing-guns/animations/support_hand.luau` while the window is running. The preview and third-person game evaluate the same module. The animation VM is separate from match rules and polls the animation folder every 200 ms. Set `GG_ANIMATIONS` to use a different animation directory.

The function `pose(magazine, foregrip, extension, seconds)` takes two gun-local wrist sockets in metres, the visible foregrip fraction from zero to one, and animation time in seconds. Return a gun-local wrist position and an extra palm roll in degrees. Change the transfer curve, add a wrist lift, or rotate the palm directly in Luau. Warp solves each arm against the resulting contact using that character’s proportions. The window shows the loaded module or its latest error.

In the window, drag the scene to orbit around the gun and scroll to zoom. The lower-left sliders offset the support wrist in gun-local centimetres and adjust palm roll and aim pitch in degrees. Pause freezes the growth and sampled clip while camera controls, sliders and Luau reload stay active. Reset clears the preview offsets, restores the camera and resumes playback. F12 records the current values beside the screenshot. Edits to sliders last for this preview session.

The preview opens with the skeleton visible. F2 toggles the overlay. Left-drag a joint to translate it in the view plane. Right-drag a joint to rotate it, right-drag empty space to orbit, middle-drag to pan, and scroll to zoom. Grabbing a joint pauses playback. The gun follows the final right wrist through the rigid `JointMount` constraint. Click a joint or use the arrows above the local XYZ rotation sliders. The highlighted links show the selected joint and its children. The panel shows the distance to its first child. The editor applies offsets after IK, so moving a shoulder also moves the elbow and hand and can intentionally break left-hand contact. F12 saves every named joint, parent, model-space position in metres and local translation in metres and Euler offset in degrees in the screenshot JSON. Other games can embed the same interactive viewport through `app::editor::PoseViewport`. See the [integration guide](editor-tools.md#adding-a-pose-viewport-to-another-game).
