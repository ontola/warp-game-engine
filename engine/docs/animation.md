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

For a right-handed prop use `TwoHandHold::bind(model.joints())` once and `hold.apply(palette, primary, secondary)` after sampling. Place the prop at the returned primary wrist. The secondary hand reaches as far as the current skeleton allows.

Growing Guns anchors the weapon to the animated right wrist, pitches around the grip and uses the left arm to hold the magazine and transfer onto the growing foregrip. Its first-person view still shows only the weapon. For an identical-camera comparison of the sampled clip and corrected grip, run from the repository root:

```sh
cargo run -p growing-guns-app -- gun-review --grip --model knight --card sniper --clip pistol_run --time 0.24 --pitch 12 --out build/grip-review
```

Inspect `clip.png` and `grip.png`. Other roster models are `paladin` and `ch10`. Omit the card for the base gun. Change time to sample the clip in seconds and pitch to aim in degrees. The support palm turns inward against the attachment. Individual finger closure and a timed reach-to-pick-up action remain separate authoring tasks.

Add `--growth --card sniper` to the grip review to capture 25 frames over the two-second weapon morph. The camera stays fixed relative to the character, and clip time stays fixed so the images isolate the grip response.

## Live Luau preview

Run `cargo run -p growing-guns-app -- animation-preview` for a windowed growth loop. Pass `--model paladin` or `--model ch10` to change character. Save edits to `games/growing-guns/animations/support_hand.luau` while the window is running. The preview and third-person game evaluate the same module. The animation VM is separate from match rules and polls the animation folder every 200 ms. Set `GG_ANIMATIONS` to use a different animation directory.

The function `pose(magazine, foregrip, extension, seconds)` takes two gun-local wrist sockets in metres, the visible foregrip fraction from zero to one, and animation time in seconds. Return a gun-local wrist position and an extra palm roll in degrees. Change the transfer curve, add a wrist lift, or rotate the palm directly in Luau. Warp solves each arm against the resulting contact using that character’s proportions. The window shows the loaded module or its latest error.

In the window, drag the scene to orbit around the gun and scroll to zoom. The lower-left sliders offset the support wrist in gun-local centimetres and adjust palm roll and aim pitch in degrees. Pause freezes the growth and sampled clip while camera controls, sliders and Luau reload stay active. Reset clears the preview offsets, restores the camera and resumes playback. F12 records the current values beside the screenshot. Edits to sliders last for this preview session.
