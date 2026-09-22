//! Ordinary-terminal image rendering for the optional lesson pet.

use std::io::Cursor;

use image::{AnimationDecoder, DynamicImage, Rgba, RgbaImage, codecs::gif::GifDecoder, imageops};
use ratatui::{Frame, layout::Rect, style::Color};
use ratatui_image::{Image, Resize, picker::Picker, protocol::Protocol};

use super::TuiError;

const CAT_GIF: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/learn-cat.gif"
));

/// Decoded cat frames rendered with Unicode half-block cells.
pub(super) struct CatAnimation {
    right_frames: Vec<Protocol>,
    left_frames: Vec<Protocol>,
}

impl CatAnimation {
    pub(super) fn load(area: Rect, surface: Color) -> Result<Self, TuiError> {
        let decoder = GifDecoder::new(Cursor::new(CAT_GIF)).map_err(TuiError::Image)?;
        let frames = decoder
            .into_frames()
            .collect_frames()
            .map_err(TuiError::Image)?;
        let background = color_to_rgba(surface);
        let mut picker = Picker::halfblocks();
        picker.set_background_color(background);

        let mut right_frames = Vec::with_capacity(frames.len());
        let mut left_frames = Vec::with_capacity(frames.len());
        for frame in frames {
            let image =
                DynamicImage::ImageRgba8(flatten_transparency(frame.into_buffer(), background));
            right_frames.push(
                picker
                    .new_protocol(image.clone(), area, Resize::Fit(None))
                    .map_err(TuiError::ImageRenderer)?,
            );
            let flipped = DynamicImage::ImageRgba8(imageops::flip_horizontal(&image.to_rgba8()));
            left_frames.push(
                picker
                    .new_protocol(flipped, area, Resize::Fit(None))
                    .map_err(TuiError::ImageRenderer)?,
            );
        }

        Ok(Self {
            right_frames,
            left_frames,
        })
    }

    pub(super) fn render(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        frame_index: usize,
        facing_right: bool,
    ) {
        if self.right_frames.is_empty() {
            return;
        }

        let frames = if facing_right {
            &self.right_frames
        } else {
            &self.left_frames
        };
        let Some(image) = frames.get(frame_index % frames.len()) else {
            return;
        };
        frame.render_widget(Image::new(image), area);
    }

    pub(super) fn frame_count(&self) -> usize {
        self.right_frames.len()
    }
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};
    use ratatui::{layout::Rect, style::Color};

    use super::{CatAnimation, Protocol, flatten_transparency};

    #[test]
    fn embedded_cat_is_a_multiframe_halfblock_animation() {
        let animation = CatAnimation::load(Rect::new(0, 0, 16, 9), Color::Rgb(14, 20, 29))
            .expect("cat GIF decodes");

        assert!(animation.right_frames.len() >= 2);
        assert_eq!(animation.right_frames.len(), animation.left_frames.len());
        assert!(
            animation
                .right_frames
                .iter()
                .all(|frame| matches!(frame, Protocol::Halfblocks(_)))
        );
    }

    #[test]
    fn transparent_pixels_use_the_active_surface_instead_of_black() {
        let image = RgbaImage::from_pixel(1, 1, Rgba([255, 120, 40, 0]));
        let flattened = flatten_transparency(image, Rgba([14, 20, 29, u8::MAX]));

        assert_eq!(flattened.get_pixel(0, 0), &Rgba([14, 20, 29, u8::MAX]));
    }
}

fn color_to_rgba(color: Color) -> Rgba<u8> {
    match color {
        Color::Rgb(red, green, blue) => Rgba([red, green, blue, u8::MAX]),
        Color::Black => Rgba([0, 0, 0, u8::MAX]),
        Color::DarkGray => Rgba([127, 127, 127, u8::MAX]),
        Color::Gray => Rgba([229, 229, 229, u8::MAX]),
        Color::White => Rgba([255, 255, 255, u8::MAX]),
        _ => Rgba([14, 20, 29, u8::MAX]),
    }
}

fn flatten_transparency(mut image: RgbaImage, background: Rgba<u8>) -> RgbaImage {
    for pixel in image.pixels_mut() {
        let alpha = u16::from(pixel[3]);
        let inverse_alpha = u16::from(u8::MAX) - alpha;
        for channel in 0..3 {
            let blended = (u16::from(pixel[channel]) * alpha
                + u16::from(background[channel]) * inverse_alpha
                + u16::from(u8::MAX / 2))
                / u16::from(u8::MAX);
            pixel[channel] = u8::try_from(blended).unwrap_or(u8::MAX);
        }
        pixel[3] = u8::MAX;
    }
    image
}
