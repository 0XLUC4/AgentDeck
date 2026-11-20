use std::sync::Arc;

use gpui_kit::*;

use crate::theme::ui;

pub(super) fn avatar(seed: &str, size: f32) -> impl IntoElement + use<> {
    let image = Image::from_bytes(ImageFormat::Svg, svg(seed).into_bytes());
    img(Arc::new(image)).size(ui(size)).flex_shrink_0()
}

// Integer outlines share a 24-pixel grid, so eyes and smiles survive at sidebar sizes.
const HEADS: [&str; 4] = [
    "M5 6h14v1h2v9h-2v2H5v-2H3V8h2z",
    "M8 5h8v1h3v2h2v7h-2v2h-3v1H8v-1H5v-2H3V9h2V7h3z",
    "M4 5h3v2h10V5h3v3h1v8h-3v2H6v-2H3V8h1z",
    "M6 6h12v2h2v7h-1v2h-3v1H8v-1H5v-2H4V8h2z",
];
const TOPS: [&str; 4] = [
    "M11 3h2v4h-2z M10 2h4v2h-4z",
    "M7 3h2v4H7z M15 3h2v4h-2z",
    "M12 3h5v1h-3v3h-2z M16 2h2v2h-2z",
    "M9 3h4v1h2v3h-3V5H9z",
];
const EARS: [&str; 3] = [
    "M1 10h3v4H1z M20 10h3v4h-3z",
    "M2 8h2v7H2z M20 8h2v7h-2z",
    "M1 9h3v3H2v1H1z M20 9h3v4h-1v-1h-2z",
];
const EYES: [&str; 5] = [
    "M7 9h3v4H7z M14 9h3v4h-3z",
    "M6 11h1V9h3v2h1v1H9v-1H8v1H6z M13 11h1V9h3v2h1v1h-2v-1h-1v1h-2z",
    "M7 9h3v4H7z M14 10h3v1h1v1h-4z",
    "M6 10h5v2H6z M13 10h5v2h-5z",
    "M6 9h5v4H6z M13 9h5v4h-5z",
];
const MOUTHS: [&str; 4] = [
    "M9 14h2v1h2v-1h2v2h-2v1h-2v-1H9z",
    "M10 14h4v2h-4z",
    "M9 14h2v1h4v1h-5v-1H9z",
    "M10 14h4v1h-1v1h-2v-1h-1z",
];
const BODIES: [&str; 3] = [
    "M8 17h8v2h2v2h-3v2h-3v-2h-1v2H8v-2H6v-2h2z",
    "M9 17h6v2h3v2h-2v2h-3v-2h-2v2H8v-2H6v-2h3z",
    "M9 17h6v1h2v1h2v2h-3v1H8v-1H5v-2h2v-1h2z",
];

fn svg(seed: &str) -> String {
    // FNV-1a and a final avalanche distribute neighboring names across independent features.
    let mut hash = seed.bytes().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d049bb133111eb);
    hash ^= hash >> 31;
    let mut pick = |count: usize| {
        let index = hash % count as u64;
        hash /= count as u64;
        index as usize
    };
    let (paper, ink, trim) = if pick(2) == 0 {
        ("#efefef", "#272727", "#8a8a8a")
    } else {
        ("#282828", "#efefef", "#969696")
    };
    let head = HEADS[pick(HEADS.len())];
    let top = TOPS[pick(TOPS.len())];
    let ears = EARS[pick(EARS.len())];
    let eyes = EYES[pick(EYES.len())];
    let mouth = MOUTHS[pick(MOUTHS.len())];
    let body = BODIES[pick(BODIES.len())];
    let cheeks = [
        "M5 13h2v1H5z M17 13h2v1h-2z",
        "M5 14h2v1H5z M17 14h2v1h-2z",
        "",
    ][pick(3)];
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96" viewBox="0 0 24 24" shape-rendering="crispEdges"><rect width="24" height="24" rx="3" fill="{paper}"/><path fill="{trim}" d="{ears} {top}"/><path fill="{ink}" d="{body} {head}"/><path fill="{paper}" d="{eyes} {mouth} M10 19h4v1h-4z"/><path fill="{trim}" d="M6 7h4v1H6z {cheeks}"/></svg>"##
    )
}

#[cfg(test)]
mod tests {
    use super::svg;
    use std::collections::HashSet;

    #[test]
    fn seeds_are_stable_and_diverse() {
        for seed in ["", "Curie", "🐱", "<&>"] {
            assert_eq!(svg(seed), svg(&seed.to_owned()));
        }
        let robots: HashSet<_> = (0..64).map(|i| svg(&format!("worker-{i}"))).collect();
        assert!(robots.len() >= 60);
        assert!(
            robots
                .iter()
                .any(|svg| svg.contains("rx=\"3\" fill=\"#282828\""))
        );
        assert!(
            robots
                .iter()
                .any(|svg| svg.contains("rx=\"3\" fill=\"#efefef\""))
        );
    }
}
