use std::sync::LazyLock;

use serde::Deserialize;

#[derive(Deserialize)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Deserialize)]
struct Path {
    rect: Rect,
    rx: f64,
    ry: f64,
    clear: bool,
}

#[derive(Deserialize)]
struct Mask {
    width: f64,
    height: f64,
    paths: Vec<Path>,
}

static MASK: LazyLock<Mask> = LazyLock::new(|| codeisland_swift::call(c"tray.mask", ()));
const SUPERSAMPLE: u32 = 4;

pub(super) fn render(size: u32, foreground: u8) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(size as usize * size as usize * 4);
    let samples_per_axis = f64::from(size * SUPERSAMPLE);
    let samples = SUPERSAMPLE * SUPERSAMPLE;

    for y in 0..size {
        for x in 0..size {
            let mut filled = 0;
            for row in 0..SUPERSAMPLE {
                for column in 0..SUPERSAMPLE {
                    let sx = (f64::from(x * SUPERSAMPLE + column) + 0.5) * MASK.width / samples_per_axis;
                    let sy = MASK.height - (f64::from(y * SUPERSAMPLE + row) + 0.5) * MASK.height / samples_per_axis;
                    let filled_here = MASK.paths.iter().rev().find(|path| path.contains(sx, sy))
                        .is_some_and(|path| !path.clear);
                    filled += u32::from(filled_here);
                }
            }
            let alpha = ((filled * 255 + samples / 2) / samples) as u8;
            rgba.extend_from_slice(&[foreground, foreground, foreground, alpha]);
        }
    }
    rgba
}

impl Path {
    fn contains(&self, x: f64, y: f64) -> bool {
        let rect = &self.rect;
        let right = rect.x + rect.width;
        let top = rect.y + rect.height;
        if x < rect.x || x > right || y < rect.y || y > top { return false; }
        let rx = self.rx.min(rect.width / 2.0);
        let ry = self.ry.min(rect.height / 2.0);
        if rx <= 0.0 || ry <= 0.0 { return true; }
        let dx = ((x - (rect.x + rect.width / 2.0)).abs() - (rect.width / 2.0 - rx)).max(0.0) / rx;
        let dy = ((y - (rect.y + rect.height / 2.0)).abs() - (rect.height / 2.0 - ry)).max(0.0) / ry;
        dx * dx + dy * dy <= 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_mask_keeps_the_two_eye_holes_in_both_tints() {
        let size = 32;
        let light = render(size, 255);
        let dark = render(size, 0);
        let at = |x, y| ((y * size + x) * 4) as usize;
        let alpha = |x, y| light[at(x, y) + 3];

        assert_eq!(light.len(), (size * size * 4) as usize);
        assert_eq!(alpha(0, 16), 0);
        assert_eq!(alpha(16, 0), 0);
        assert_eq!(alpha(4, 16), 255);
        assert_eq!(alpha(16, 16), 255);
        assert_eq!(alpha(11, 16), 0);
        assert_eq!(alpha(20, 16), 0);
        assert!(light.chunks_exact(4).any(|pixel| (1..255).contains(&pixel[3])));
        assert_eq!(dark[at(16, 16)..at(16, 16) + 4], [0, 0, 0, 255]);
        assert_eq!(light[at(16, 16)..at(16, 16) + 4], [255, 255, 255, 255]);
        assert!(light.chunks_exact(4).zip(dark.chunks_exact(4)).all(|(a, b)| a[3] == b[3]));
    }
}
