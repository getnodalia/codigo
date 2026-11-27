// SPDX-License-Identifier: MIT OR Apache-2.0
//! The marks of the services Código connects to, drawn tinted like the rest of the icons.
//!
//! The artwork is a white-on-transparent mask of each service's own mark (see `ATTRIBUTION.md`),
//! so it takes the colour of whatever it sits in. The marks are trademarks of GitHub, Inc. and
//! GitLab B.V., shown only to name those services.

use egui::{Color32, ColorImage, Context, Painter, Rect, TextureHandle, TextureOptions};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Brand {
    GitHub,
    GitLab,
}

impl Brand {
    /// The name `widgets::paint_named` knows the mark by.
    pub fn icon_name(self) -> &'static str {
        match self {
            Brand::GitHub => "brand-github",
            Brand::GitLab => "brand-gitlab",
        }
    }

    pub fn from_icon_name(name: &str) -> Option<Brand> {
        match name {
            "brand-github" => Some(Brand::GitHub),
            "brand-gitlab" => Some(Brand::GitLab),
            _ => None,
        }
    }

    fn png(self) -> &'static [u8] {
        match self {
            Brand::GitHub => include_bytes!("../../../assets/brand/github-mark.png"),
            Brand::GitLab => include_bytes!("../../../assets/brand/gitlab-mark.png"),
        }
    }

    fn key(self) -> egui::Id {
        egui::Id::unique(("codigo-brand-mark", self.icon_name()))
    }
}

/// Decode a mark into RGBA pixels: `(width, height, rgba)`.
fn decode(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|[g, a]| [*g, *g, *g, *a])
            .collect(),
        _ => return None,
    };
    Some((info.width as usize, info.height as usize, rgba))
}

fn texture(ctx: &Context, brand: Brand) -> Option<TextureHandle> {
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(brand.key())) {
        return Some(t);
    }
    let (w, h, rgba) = decode(brand.png())?;
    let image = ColorImage::from_rgba_unmultiplied([w, h], &rgba);
    let handle = ctx.load_texture(brand.icon_name(), image, TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(brand.key(), handle.clone()));
    Some(handle)
}

/// Paint a mark in `rect`, as wide and tall as fits, in the colour `ink`.
pub fn paint(painter: &Painter, rect: Rect, brand: Brand, ink: Color32) {
    let Some(tex) = texture(painter.ctx(), brand) else {
        return;
    };
    let side = rect.width().min(rect.height());
    let square = Rect::from_center_size(rect.center(), egui::vec2(side, side));
    painter.image(
        tex.id(),
        square,
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        ink,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_marks_decode_to_a_square_mask() {
        for brand in [Brand::GitHub, Brand::GitLab] {
            let (w, h, rgba) = decode(brand.png()).expect("decodes");
            assert_eq!(w, h, "{brand:?}");
            assert_eq!(rgba.len(), w * h * 4);
            assert!(
                rgba.as_chunks::<4>().0.iter().any(|p| p[3] == 255),
                "{brand:?} has ink"
            );
            assert!(
                rgba.as_chunks::<4>().0.iter().any(|p| p[3] == 0),
                "{brand:?} has room"
            );
        }
    }

    #[test]
    fn icon_names_round_trip() {
        for brand in [Brand::GitHub, Brand::GitLab] {
            assert_eq!(Brand::from_icon_name(brand.icon_name()), Some(brand));
        }
        assert_eq!(Brand::from_icon_name("cloud"), None);
    }
}
