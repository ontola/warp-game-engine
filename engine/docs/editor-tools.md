# Warp editor tools

| Tool | Where to open it | What it edits or reports |
| --- | --- | --- |
| Pose viewport | Growing Guns `animation-preview`, or embed `app::editor::PoseViewport` in a Rust game | Pick bones on the model, translate and rotate them, orbit and pan the camera, edit game-supplied parameters, export local edits and model-space positions. |
| Settings inspector | F2 in a game on the Warp shell | Render settings and game-supplied controls. Games register nested menus and typed controls with `Game::debug` and receive changes in `debug_action`. Sideways uses these for vegetation, lighting and weather. |
| Frame profiler | F3 in the native shell. Shift+F3 changes detail | CPU, GPU and memory measurements, including the game's own diagnostic rows. |
| Feedback capture | F4 in a shell game | Image, camera and game-supplied JSON. Growing Guns adds gameplay replay and F11 review. Sideways has a capture inspector for identifying geometry under a pixel. |
| Live development | Warp MCP, Luau hot reload, scripted runs and captures | Inspect or change a running game, replay a scenario and compare images or timings. These tools also work outside a graphical inspector. |

## Adding a pose viewport to another game

1. Bind the skin palette and parent names once with `body::pose_editor::PoseEditor::bind`. Pass it to `PoseViewport::new(editor, home_camera, rows)`. Each optional slider row supplies a label, a positive symmetric limit and a unit. Use `[]` for a skeleton-only view.
2. Enable pointer input with a free cursor. Forward the shell's physical pointer sample and output size to `viewport.pointer`, and forward function keys to `viewport.function_key`. Grabbing a joint sets `paused`; the host decides which animation clock to freeze. Game-specific controls are available in `values`.
3. Sample a fresh skin palette, solve any gameplay constraints, then call `viewport.bones.editor.apply(&mut palette)`. Upload that palette to the character. Read the final wrist here when attaching a prop.
4. Render with `viewport.camera()`. Use `viewport.draw(output_size, model_to_scene, status)` for the HUD. The model transform must match the rendered actor. Return `viewport.snapshot()` through `Game::state` for F4; add your game-specific fields there.

The shared viewport owns scaled logical hit regions, joint picking, drag controls, camera controls and capture field names. Growing Guns supplies its knight, hierarchy, grip sliders, Luau script and saved foregrip pose. SpellStorm can supply a different rig through the same Rust API. There is currently no standalone scene-editing application or Luau binding for this viewport.

## GUI conventions

Warp owns F1 Dev Mode, F2 Graphics, F3 Profiler and F4 Capture. Alt+Enter toggles fullscreen. Games allocate scene shortcuts from F12 downward through F5. Warp windows draw in flat screen coordinates after game HUD optics.

F1 toggles Warp Dev Mode in the native app shell. Its clickable shortcut bar stays visible with zero panels open. Leaving hides the bar and the panels together. Entering restores the previous selection. Click an exposed part of a window to bring it forward. Each header shows the F1 shortcut. Debug text uses one viewport-relative size across panels. Game hosts expose their panels through `Game::editor_windows_open` and `Game::set_editor_windows_open`.

Editor windows display their toggle key in the title bar. Drag the title to move, drag the bottom-right grip to resize, and click x to close. F12 opens the Pose editor with joint and pose controls, and F2 opens Graphics settings, including any pages registered through Game::debug. F3 opens the engine profiler. Growing Guns uses F12 for live explosion tuning and F10 for match tools. F11 opens the Growing Guns replay window. Pose windows keep their layout when toggled during a preview. The profiler releases the mouse and supports wheel scrolling.

Use the existing F2 controls for settings, choices and commands. Use direct viewport interaction for spatial edits. In the pose viewport, left-drag moves a joint in the view plane, right-drag rotates a joint or orbits empty space, middle-drag pans and the wheel zooms. F12 toggles bones.

The pose viewport, F3 profiler and F2 inspector use `render::editor_style` for panel backgrounds, accent rails and text colours, and render through Warp's native HUD. Gameplay menus retain their game-specific styling. The [debug UI guide](debug-ui.md) covers input priority, held buttons and slider behaviour.

## Game-owned editor menus

In the Growing Guns animation preview, open **F2 → Animation preview** to pause playback, show the skeleton, or reset the grip sliders. The toggle values reflect changes made directly in the viewport. Reset grip sliders becomes available after an offset changes.

Rust games implement `Game::debug` to build the current menu model. Start with `app::editor::EditorMenus::default()`, add a `page(id, parent, label, help)`, add each `control(page, id, label, value, help)`, and return `menus.finish()`. A control accepts a button, toggle, numeric range or choice. To disable it while keeping it visible, set the returned control's `enabled` field. Handle changes by stable ID in `Game::debug_action`. Give game pages and actions unique IDs in their respective namespaces starting at `app::debug::GAME_IDS` (1000). The shell supplies nesting, keyboard navigation, pointer handling and labels.

Luau games can export pages of action buttons from their entry module. Declare parents first. Page definitions and callbacks reload with the script. This example adds **F2 → Character → Pose → Reset stance**.

```lua
return {
    editor_pages = {
        { id = 1000, label = "Character" },
        { id = 1001, parent = 1000, label = "Pose", buttons = {
            { id = 1002, label = "Reset stance", help = "Restore the default stance." },
        }},
    },
    editor_action = function(id)
        if id == 1002 then state.stance = "idle" end
    end,
}
```

Use `render::editor_style::EditorButton` for buttons drawn directly in an editor viewport. Use the same rectangle for drawing and hit testing. F2 uses this widget for action buttons, navigation, toggles and steppers; it supplies the enabled and highlighted states.


Space Racer and SpellStorm open an independent F12 scene window through `Game::scene_tuning` and `scene_tuning_mut`. F2 and F12 can be moved, resized, and closed independently. Both use the same live controls. The scene owns its `SceneTuning` and supplies the numerical controls and their defaults. A changed field shows a star and the original value. Restoring that value clears the mark. **Save changes for LLM** copies a JSON diff to the clipboard and writes a timestamped copy under `build/editor-changes/` in the working directory.
