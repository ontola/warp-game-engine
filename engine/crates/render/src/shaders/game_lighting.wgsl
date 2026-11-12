const WRITE_STYLE_TAGS = false;
fn game_lighting(s: LightingSample) -> StyledLight {
    return StyledLight(s.radiance, 0u);
}
