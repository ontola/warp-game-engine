use render::upscale::{UpscaleMode, UpscaleStatus};

pub fn parse(args: &[String]) -> Result<UpscaleMode, String> {
    let mut found = args.iter().enumerate().filter(|(_, value)| value.as_str() == "--upscale");
    let Some((index, _)) = found.next() else { return Ok(UpscaleMode::Native); };
    if found.next().is_some() { return Err("--upscale must be specified only once".into()); }
    args.get(index + 1).filter(|s| !s.starts_with("--")).ok_or("--upscale requires a mode")?.parse()
}

pub fn metadata(requested: UpscaleMode, actual: &UpscaleStatus) -> serde_json::Value {
    serde_json::json!({"requested": requested.as_str(), "mode": actual.mode.as_str(),
        "evaluated": !actual.views.is_empty(), "views":actual.views.iter().map(|v|
            serde_json::json!({"input_size":v.input,"output_size":v.output})).collect::<Vec<_>>(),
        "hud_resolution":"output", "fallback":false})
}

/// Commit the user's choice only after backend initialization succeeds.
pub fn select(value: &serde_json::Value, current: &mut UpscaleMode,
    apply: impl FnOnce(UpscaleMode) -> Result<(), String>) -> Result<(), String> {
    let requested=value.as_str().ok_or("upscale setting requires a mode string")?.parse()?;
    apply(requested)?;
    *current=requested;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(s:&str)->Vec<String>{s.split_whitespace().map(str::to_owned).collect()}
    #[test] fn requires_an_explicit_valid_mode_and_preserves_native_default() {
        assert_eq!(parse(&args("play --views 4")).unwrap(), UpscaleMode::Native);
        assert_eq!(parse(&args("shot x.png --upscale dlss-balanced")).unwrap(), UpscaleMode::DlssBalanced);
        for invalid in ["--upscale", "--upscale --views 4", "--upscale auto", "--upscale native --upscale dlss-quality"] { assert!(parse(&args(invalid)).is_err(),"{invalid}"); }
    }
    #[test] fn metadata_uses_observed_dimensions_without_inventing_a_ratio() {
        let status=UpscaleStatus { mode:UpscaleMode::DlssQuality, views:vec![render::upscale::UpscaleViewStatus {input:[1276,716],output:[1916,1076]}] };
        let value=metadata(UpscaleMode::DlssQuality,&status);
        assert_eq!(value["views"][0]["input_size"],serde_json::json!([1276,716]));
        assert_eq!(value["views"][0]["output_size"],serde_json::json!([1916,1076]));
        assert_eq!(value["mode"],"dlss-quality");assert_eq!(value["evaluated"],true);
        assert_eq!(metadata(UpscaleMode::DlssQuality,&Default::default())["evaluated"],false);
    }
    #[test] fn failed_runtime_selection_keeps_the_accepted_mode_and_error() {
        let mut mode=UpscaleMode::Native;
        let result=select(&serde_json::json!("dlss-quality"),&mut mode,|_|Err("DLSS runtime unavailable".into()));
        assert_eq!(result.unwrap_err(),"DLSS runtime unavailable");assert_eq!(mode,UpscaleMode::Native);
        select(&serde_json::json!("dlss-balanced"),&mut mode,|_|Ok(())).unwrap();
        assert_eq!(mode,UpscaleMode::DlssBalanced);
        assert!(select(&serde_json::json!(false),&mut mode,|_|panic!("invalid choice must not touch backend")).is_err());
        assert_eq!(mode,UpscaleMode::DlssBalanced);
    }
}
