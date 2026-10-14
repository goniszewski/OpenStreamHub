//! Generated pictures for keypad action tiles that have no artwork of their own: a small icon for what the tile does,
//! above its label. The editor's preview draws the same idea (an icon over the label), so the module and the preview
//! agree. Everything is drawn from shapes and the built-in 5×7 bitmap font, so no font or icon files are read.

use cooltek_sdk_service::display_sync::TileIcon;
use image::{Rgb, RgbImage};

use crate::dial_artwork::glyph;

/// Key image side in pixels.
const SIDE: u32 = 72;
const BACKGROUND: [u8; 3] = [36, 52, 78];
const WHITE: [u8; 3] = [255, 255, 255];
/// Characters per line at the large and the small label size.
const LARGE_CHARS: usize = 6;
const SMALL_CHARS: usize = 12;

type Point = (f32, f32);

fn put(image: &mut RgbImage, x: i32, y: i32, color: [u8; 3]) {
    if x >= 0 && y >= 0 && (x as u32) < SIDE && (y as u32) < SIDE {
        image.put_pixel(x as u32, y as u32, Rgb(color));
    }
}

/// Fills the pixels whose centre satisfies `inside`, within a box.
fn paint(
    image: &mut RgbImage,
    from: Point,
    to: Point,
    color: [u8; 3],
    inside: impl Fn(f32, f32) -> bool,
) {
    for y in from.1.floor() as i32..=to.1.ceil() as i32 {
        for x in from.0.floor() as i32..=to.0.ceil() as i32 {
            if inside(x as f32 + 0.5, y as f32 + 0.5) {
                put(image, x, y, color);
            }
        }
    }
}

fn rect(image: &mut RgbImage, x: f32, y: f32, width: f32, height: f32, color: [u8; 3]) {
    paint(image, (x, y), (x + width, y + height), color, |px, py| {
        px >= x && px < x + width && py >= y && py < y + height
    });
}

fn disc(image: &mut RgbImage, (cx, cy): Point, radius: f32, color: [u8; 3]) {
    paint(
        image,
        (cx - radius, cy - radius),
        (cx + radius, cy + radius),
        color,
        |px, py| (px - cx).hypot(py - cy) <= radius,
    );
}

/// A ring between two radii, limited to the pixels `keep` accepts (relative to the centre).
fn ring(
    image: &mut RgbImage,
    (cx, cy): Point,
    inner: f32,
    outer: f32,
    color: [u8; 3],
    keep: impl Fn(f32, f32) -> bool,
) {
    paint(
        image,
        (cx - outer, cy - outer),
        (cx + outer, cy + outer),
        color,
        |px, py| {
            let distance = (px - cx).hypot(py - cy);
            distance >= inner && distance <= outer && keep(px - cx, py - cy)
        },
    );
}

fn triangle(image: &mut RgbImage, a: Point, b: Point, c: Point, color: [u8; 3]) {
    let side = |p: Point, q: Point, r: Point| (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0);
    let low = (a.0.min(b.0).min(c.0), a.1.min(b.1).min(c.1));
    let high = (a.0.max(b.0).max(c.0), a.1.max(b.1).max(c.1));
    paint(image, low, high, color, |px, py| {
        let p = (px, py);
        let (d1, d2, d3) = (side(a, b, p), side(b, c, p), side(c, a, p));
        !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
    });
}

/// A straight stroke with round ends.
fn stroke(image: &mut RgbImage, a: Point, b: Point, half: f32, color: [u8; 3]) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    paint(
        image,
        (a.0.min(b.0) - half, a.1.min(b.1) - half),
        (a.0.max(b.0) + half, a.1.max(b.1) + half),
        color,
        |px, py| {
            let along = if length == 0.0 {
                0.0
            } else {
                (((px - a.0) * dx + (py - a.1) * dy) / length).clamp(0.0, 1.0)
            };
            (px - (a.0 + along * dx)).hypot(py - (a.1 + along * dy)) <= half
        },
    );
}

fn speaker(image: &mut RgbImage, (x, y): Point) {
    rect(image, x - 12.0, y - 4.0, 6.0, 8.0, WHITE);
    triangle(
        image,
        (x - 6.0, y - 4.0),
        (x + 2.0, y - 12.0),
        (x + 2.0, y + 12.0),
        WHITE,
    );
    triangle(
        image,
        (x - 6.0, y - 4.0),
        (x + 2.0, y + 12.0),
        (x - 6.0, y + 4.0),
        WHITE,
    );
}

/// Draws an icon centred on `c`, about 28 pixels across.
fn icon(image: &mut RgbImage, kind: TileIcon, c: Point) {
    let (x, y) = c;
    match kind {
        TileIcon::Generic => {
            rect(image, x - 12.0, y - 9.0, 24.0, 18.0, WHITE);
            rect(image, x - 10.0, y - 7.0, 20.0, 14.0, BACKGROUND);
            for column in 0..3 {
                rect(
                    image,
                    x - 7.0 + column as f32 * 6.0,
                    y - 3.0,
                    3.0,
                    3.0,
                    WHITE,
                );
            }
            rect(image, x - 6.0, y + 2.0, 12.0, 2.0, WHITE);
        }
        TileIcon::Volume => {
            speaker(image, (x - 3.0, y));
            for (inner, outer) in [(5.0, 7.0), (10.0, 12.0)] {
                ring(image, (x - 1.0, y), inner, outer, WHITE, |dx, dy| {
                    dx > 0.0 && dy.abs() <= dx
                });
            }
        }
        TileIcon::Mute => {
            speaker(image, (x - 5.0, y));
            stroke(image, (x + 4.0, y - 6.0), (x + 13.0, y + 6.0), 1.6, WHITE);
            stroke(image, (x + 4.0, y + 6.0), (x + 13.0, y - 6.0), 1.6, WHITE);
        }
        TileIcon::Play => triangle(
            image,
            (x - 7.0, y - 12.0),
            (x - 7.0, y + 12.0),
            (x + 11.0, y),
            WHITE,
        ),
        TileIcon::Next => {
            triangle(
                image,
                (x - 11.0, y - 11.0),
                (x - 11.0, y + 11.0),
                (x + 5.0, y),
                WHITE,
            );
            rect(image, x + 7.0, y - 11.0, 4.0, 22.0, WHITE);
        }
        TileIcon::Previous => {
            triangle(
                image,
                (x + 11.0, y - 11.0),
                (x + 11.0, y + 11.0),
                (x - 5.0, y),
                WHITE,
            );
            rect(image, x - 11.0, y - 11.0, 4.0, 22.0, WHITE);
        }
        TileIcon::Stop => rect(image, x - 10.0, y - 10.0, 20.0, 20.0, WHITE),
        TileIcon::Screenshot => {
            rect(image, x - 6.0, y - 12.0, 12.0, 5.0, WHITE);
            rect(image, x - 13.0, y - 8.0, 26.0, 19.0, WHITE);
            disc(image, (x, y + 2.0), 6.5, BACKGROUND);
            disc(image, (x, y + 2.0), 3.5, WHITE);
        }
        TileIcon::Undo | TileIcon::Redo => {
            let flip = if kind == TileIcon::Redo { -1.0 } else { 1.0 };
            ring(image, (x, y + 4.0), 8.0, 11.0, WHITE, |_, dy| dy <= 0.0);
            ring(image, (x, y + 4.0), 8.0, 11.0, WHITE, |dx, dy| {
                dy > 0.0 && dy < 4.0 && dx * flip < 0.0
            });
            let tip = x - flip * 9.5;
            triangle(
                image,
                (tip - 6.0, y + 4.0),
                (tip + 6.0, y + 4.0),
                (tip, y + 12.0),
                WHITE,
            );
        }
        TileIcon::Timer => {
            ring(image, (x, y + 2.0), 10.0, 13.0, WHITE, |_, _| true);
            stroke(image, (x, y + 2.0), (x, y - 6.0), 1.5, WHITE);
            stroke(image, (x, y + 2.0), (x + 5.0, y + 5.0), 1.5, WHITE);
            rect(image, x - 3.0, y - 14.0, 6.0, 3.0, WHITE);
        }
        TileIcon::Web => {
            ring(image, c, 10.0, 12.5, WHITE, |_, _| true);
            rect(image, x - 12.0, y - 1.0, 24.0, 2.0, WHITE);
            rect(image, x - 1.0, y - 12.0, 2.0, 24.0, WHITE);
            paint(
                image,
                (x - 6.0, y - 12.0),
                (x + 6.0, y + 12.0),
                WHITE,
                |px, py| {
                    let value = ((px - x) / 5.5).powi(2) + ((py - y) / 11.5).powi(2);
                    (0.65..=1.0).contains(&value)
                },
            );
        }
        TileIcon::Application => {
            rect(image, x - 13.0, y - 10.0, 26.0, 21.0, WHITE);
            rect(image, x - 11.0, y - 4.0, 22.0, 13.0, BACKGROUND);
            for dot in 0..3 {
                disc(
                    image,
                    (x - 9.0 + dot as f32 * 4.0, y - 7.0),
                    1.1,
                    BACKGROUND,
                );
            }
        }
        TileIcon::Text => {
            rect(image, x - 11.0, y - 11.0, 22.0, 5.0, WHITE);
            rect(image, x - 3.0, y - 11.0, 6.0, 23.0, WHITE);
        }
        TileIcon::Sequence => {
            for row in 0..3 {
                let top = y - 9.0 + row as f32 * 8.0;
                disc(image, (x - 10.0, top + 1.5), 2.0, WHITE);
                rect(image, x - 5.0, top, 16.0, 3.0, WHITE);
            }
        }
        TileIcon::Profile => {
            rect(image, x - 11.0, y - 7.0, 17.0, 3.0, WHITE);
            triangle(
                image,
                (x + 4.0, y - 12.0),
                (x + 4.0, y),
                (x + 13.0, y - 5.5),
                WHITE,
            );
            rect(image, x - 6.0, y + 5.0, 17.0, 3.0, WHITE);
            triangle(
                image,
                (x - 4.0, y),
                (x - 4.0, y + 12.0),
                (x - 13.0, y + 6.5),
                WHITE,
            );
        }
        TileIcon::Obs => {
            rect(image, x - 13.0, y - 3.0, 26.0, 14.0, WHITE);
            rect(image, x - 11.0, y - 1.0, 22.0, 10.0, BACKGROUND);
            rect(image, x - 13.0, y - 11.0, 26.0, 6.0, WHITE);
            for stripe in 0..3 {
                let left = x - 9.0 + stripe as f32 * 7.0;
                stroke(
                    image,
                    (left, y - 5.0),
                    (left + 4.0, y - 11.0),
                    1.3,
                    BACKGROUND,
                );
            }
        }
    }
}

fn draw_text(image: &mut RgbImage, line: &str, top: i32, scale: i32) {
    let advance = 6 * scale;
    let width = line.chars().count() as i32 * advance - scale;
    let mut left = (SIDE as i32 - width) / 2;
    for character in line.chars() {
        for (row, bits) in (0i32..).zip(glyph(character)) {
            for column in 0..5 {
                if bits & (0b10000 >> column) != 0 {
                    rect(
                        image,
                        (left + column * scale) as f32,
                        (top + row * scale) as f32,
                        scale as f32,
                        scale as f32,
                        WHITE,
                    );
                }
            }
        }
        left += advance;
    }
}

/// Greedy word wrap; `None` when a word is longer than a line or there are more than `lines` lines.
fn wrap(label: &str, width: usize, lines: usize) -> Option<Vec<String>> {
    let mut wrapped: Vec<String> = Vec::new();
    for word in label.split_whitespace() {
        if word.chars().count() > width {
            return None;
        }
        match wrapped.last_mut() {
            Some(last) if last.chars().count() + 1 + word.chars().count() <= width => {
                last.push(' ');
                last.push_str(word);
            }
            _ => wrapped.push(word.to_owned()),
        }
    }
    (wrapped.len() <= lines).then_some(wrapped)
}

/// The label as lines at the largest size that fits: two lines of 6 at double size, else three lines of 12 at normal
/// size, cut with a final dot when it still does not fit.
fn lines_of(label: &str) -> (Vec<String>, i32) {
    if let Some(lines) = wrap(label, LARGE_CHARS, 2) {
        return (lines, 2);
    }
    if let Some(lines) = wrap(label, SMALL_CHARS, 3) {
        return (lines, 1);
    }
    let text = label.split_whitespace().collect::<Vec<_>>().join(" ");
    let characters: Vec<char> = text.chars().take(3 * SMALL_CHARS).collect();
    let mut lines: Vec<String> = characters
        .chunks(SMALL_CHARS)
        .map(|chunk| chunk.iter().collect::<String>().trim().to_owned())
        .collect();
    if text.chars().count() > 3 * SMALL_CHARS
        && let Some(last) = lines.last_mut()
    {
        last.pop();
        last.push('.');
    }
    (lines, 1)
}

/// The picture of an action tile: its icon above its label, or a larger icon alone without a label.
pub fn draw(kind: TileIcon, label: &str) -> RgbImage {
    let mut image = RgbImage::from_pixel(SIDE, SIDE, Rgb(BACKGROUND));
    let (lines, scale) = lines_of(label);
    if lines.is_empty() {
        icon(&mut image, kind, (36.0, 36.0));
        return image;
    }
    let line_height = 8 * scale;
    let text_height = lines.len() as i32 * line_height - scale;
    let text_top = SIDE as i32 - 6 - text_height;
    icon(
        &mut image,
        kind,
        (36.0, (text_top as f32 - 4.0) / 2.0 + 1.0),
    );
    for (index, line) in lines.iter().enumerate() {
        draw_text(
            &mut image,
            line,
            text_top + index as i32 * line_height,
            scale,
        );
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICONS: [TileIcon; 17] = [
        TileIcon::Generic,
        TileIcon::Volume,
        TileIcon::Mute,
        TileIcon::Play,
        TileIcon::Next,
        TileIcon::Previous,
        TileIcon::Stop,
        TileIcon::Screenshot,
        TileIcon::Undo,
        TileIcon::Redo,
        TileIcon::Timer,
        TileIcon::Web,
        TileIcon::Application,
        TileIcon::Text,
        TileIcon::Sequence,
        TileIcon::Profile,
        TileIcon::Obs,
    ];

    fn lit(image: &RgbImage, region: (u32, u32, u32, u32)) -> usize {
        let mut count = 0;
        for y in region.1..region.3 {
            for x in region.0..region.2 {
                if image.get_pixel(x, y).0 != BACKGROUND {
                    count += 1;
                }
            }
        }
        count
    }

    #[test]
    fn every_icon_draws_something_of_its_own_in_the_picture_area() {
        let mut seen = Vec::new();
        for kind in ICONS {
            let image = draw(kind, "");
            assert!(
                lit(&image, (14, 14, 58, 58)) > 40,
                "{kind:?} is nearly empty"
            );
            assert_eq!(
                lit(&image, (0, 0, 72, 8)),
                0,
                "{kind:?} reaches the top edge"
            );
            assert!(!seen.contains(&image), "{kind:?} looks like another icon");
            seen.push(image);
        }
    }

    #[test]
    fn a_label_goes_under_the_icon_and_the_icon_stays_clear_of_it() {
        let image = draw(TileIcon::Play, "Mute");
        assert!(lit(&image, (0, 40, 72, 72)) > 30, "label");
        assert!(lit(&image, (10, 4, 62, 36)) > 40, "icon");
        for kind in ICONS {
            let tall = draw(kind, "Previous track");
            let icon_area = lit(&tall, (0, 0, 72, 36));
            assert!(icon_area > 40 && icon_area < 900, "{kind:?}: {icon_area}");
        }
    }

    #[test]
    fn labels_take_the_largest_size_that_fits() {
        assert_eq!(lines_of("Mute"), (vec!["Mute".into()], 2));
        assert_eq!(
            lines_of("Play / pause"),
            (vec!["Play /".into(), "pause".into()], 2)
        );
        assert_eq!(lines_of("Screenshot"), (vec!["Screenshot".into()], 1));
        assert_eq!(
            lines_of("Previous track"),
            (vec!["Previous".into(), "track".into()], 1)
        );
        assert_eq!(lines_of("  "), (Vec::<String>::new(), 2));
        let (long, scale) = lines_of(&"w".repeat(80));
        assert_eq!((long.len(), scale), (3, 1));
        assert!(long[2].ends_with('.'));
    }

    #[test]
    fn drawing_is_deterministic() {
        assert_eq!(
            draw(TileIcon::Timer, "Break"),
            draw(TileIcon::Timer, "Break")
        );
        assert_ne!(
            draw(TileIcon::Timer, "Break"),
            draw(TileIcon::Timer, "Lunch")
        );
    }
}
