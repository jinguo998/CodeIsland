// StatusItemController.menuBarIcon draws this mask on an 18-point canvas.
const LOGICAL_SIZE: f64 = 18.0;
const SUPERSAMPLE: u32 = 4;

pub(super) fn render(size: u32, foreground: u8) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(size as usize * size as usize * 4);
    let step = LOGICAL_SIZE / f64::from(size * SUPERSAMPLE);
    let samples = SUPERSAMPLE * SUPERSAMPLE;

    for y in 0..size {
        for x in 0..size {
            let mut filled = 0;
            for row in 0..SUPERSAMPLE {
                for column in 0..SUPERSAMPLE {
                    let sx = (f64::from(x * SUPERSAMPLE + column) + 0.5) * step;
                    let sy = (f64::from(y * SUPERSAMPLE + row) + 0.5) * step;
                    filled += u32::from(inside_mark(sx, sy));
                }
            }
            let alpha = ((filled * 255 + samples / 2) / samples) as u8;
            rgba.extend_from_slice(&[foreground, foreground, foreground, alpha]);
        }
    }
    rgba
}

fn inside_mark(x: f64, y: f64) -> bool {
    let dy = y - 9.0;
    let cap = x - x.clamp(5.5, 12.5);
    let inside_pill = cap * cap + dy * dy <= 4.0 * 4.0;
    let inside_eye = [6.6, 11.4]
        .into_iter()
        .any(|center| (x - center) * (x - center) + dy * dy < 1.3 * 1.3);
    inside_pill && !inside_eye
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
