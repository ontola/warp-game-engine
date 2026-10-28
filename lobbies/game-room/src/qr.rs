//! The room QR code as the host renders it (the qrcode crate's SVG: one
//! background rect and one path of `M{x} {y}h{w}v{h}H{x}V{y}` modules),
//! read back into module rectangles.

/// Dark modules as [x, y, w, h], in 0..1 of the image.
pub fn modules(svg: &str) -> Vec<[f32; 4]> {
    let attr = |name: &str| -> Option<f32> {
        let at = svg.find(&format!(" {name}=\""))? + name.len() + 3;
        svg[at..].split('"').next()?.parse().ok()
    };
    let (Some(w), Some(h)) = (attr("width"), attr("height")) else { return vec![] };
    let Some(d) = svg.find(" d=\"").map(|at| &svg[at + 4..]).and_then(|rest| rest.split('"').next()) else { return vec![] };
    let mut out = vec![];
    for cmd in d.split('M').skip(1) {
        // "x y h w v h H x V y"
        let nums: Vec<f32> = cmd.split(|c: char| c.is_ascii_alphabetic() || c == ' ').filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
        if nums.len() >= 4 { out.push([nums[0] / w, nums[1] / h, nums[2] / w, nums[3] / h]); }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_the_hosts_svg() {
        let svg = r##"<?xml version="1.0" standalone="yes"?><svg xmlns="http://www.w3.org/2000/svg" version="1.1" width="100" height="100" viewBox="0 0 100 100" shape-rendering="crispEdges"><rect x="0" y="0" width="100" height="100" fill="#fff"/><path fill="#000" d="M10 20h5v5H10V20M30 40h10v5H30V40"/></svg>"##;
        assert_eq!(super::modules(svg), vec![[0.1, 0.2, 0.05, 0.05], [0.3, 0.4, 0.1, 0.05]]);
        assert!(super::modules("").is_empty());
    }
}
