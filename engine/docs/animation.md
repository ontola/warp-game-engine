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

Growing Guns anchors the weapon to the animated right wrist, pitches around the grip and uses the left arm to support the receiver or growing foregrip. Its first-person view still shows only the weapon. For an identical-camera comparison of the sampled clip and corrected grip, run from the repository root:

```sh
cargo run -p growing-guns-app -- gun-review --grip --model knight --card sniper --clip pistol_run --time 0.24 --pitch 12 --out build/grip-review
```

Inspect `clip.png` and `grip.png`. Other roster models are `paladin` and `ch10`. Omit the card for the base gun. Change time to sample the clip in seconds and pitch to aim in degrees. Wrist orientation, finger closure and a timed reach-to-pick-up action remain separate authoring tasks.

Add `--growth --card sniper` to the grip review to capture 25 frames over the two-second weapon morph. The camera stays fixed relative to the character, and clip time stays fixed so the images isolate the grip response.
