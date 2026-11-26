// SPDX-License-Identifier: MIT OR Apache-2.0
//! The app icon: Gustav Kirchhoff, whose circuit laws every simulator here rests on, from a
//! 19th-century photograph (Smithsonian Libraries, public domain), in black and white on
//! Circuito's gold, `#C99A06`. The PNGs are `assets/icons/hicolor/*/apps/`; the same pixels are
//! the window icon, the logo in the title bar and the mark on the About page.

/// The logo's size in pixels.
pub const SMALL: usize = 128;
/// The window icon's size in pixels.
pub const LARGE: usize = 256;

const PNG_128: &[u8] =
    include_bytes!("../../../assets/icons/hicolor/128x128/apps/dev.danielmiguelt.Circuito.png");
const PNG_256: &[u8] =
    include_bytes!("../../../assets/icons/hicolor/256x256/apps/dev.danielmiguelt.Circuito.png");

/// Decode a PNG into straight RGBA: `(width, height, bytes)`. `None` if it is not a PNG this
/// reader understands; the app then draws without an icon instead of failing.
fn decode(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => {
            buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect()
        }
        png::ColorType::Grayscale => buf.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some((
        usize::try_from(info.width).ok()?,
        usize::try_from(info.height).ok()?,
        rgba,
    ))
}

/// The icon at `size` (`SMALL` or `LARGE`; anything else gets the nearest) as
/// `(width, height, straight RGBA)`.
pub fn rgba(size: usize) -> Option<(usize, usize, Vec<u8>)> {
    decode(if size <= SMALL { PNG_128 } else { PNG_256 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_sizes_decode_to_a_square_with_transparent_corners() {
        for size in [SMALL, LARGE] {
            let (w, h, px) = rgba(size).expect("the embedded icon decodes");
            assert_eq!((w, h), (size, size));
            assert_eq!(px.len(), size * size * 4);
            assert_eq!(px[3], 0, "the corner is transparent");
            let centre = (size / 2 * size + size / 2) * 4;
            assert_eq!(px[centre + 3], 255, "the middle is opaque");
        }
    }

    #[test]
    fn a_size_in_between_gets_the_nearest() {
        assert_eq!(rgba(1).map(|i| i.0), Some(SMALL));
        assert_eq!(rgba(200).map(|i| i.0), Some(LARGE));
        assert_eq!(decode(b"not a png").map(|i| i.0), None);
    }
}
