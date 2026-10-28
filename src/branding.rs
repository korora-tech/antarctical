//! Shared, embedded artwork. Running outside the source tree needs no asset directory.

use std::sync::{Arc, LazyLock};

use gpui::{Image, ImageFormat, Img, Styled, img, px};

pub const APP_ID: &str = "korora.tech.Antarctical";
pub const ICON_PNG: &[u8] = include_bytes!("../data/branding/antarctical-app-icon.png");

static ICON: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Svg,
        include_bytes!("../data/branding/antarctical-app-icon.svg").to_vec(),
    ))
});

static MASCOT: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Svg,
        include_bytes!("../data/branding/antarctical-penguin.svg").to_vec(),
    ))
});

pub fn icon(size: f32) -> Img {
    img(Arc::clone(&ICON)).size(px(size)).flex_shrink_0()
}

pub fn mascot(size: f32) -> Img {
    img(Arc::clone(&MASCOT)).size(px(size)).flex_shrink_0()
}
