use image::{
    buffer::ConvertBuffer,
    imageops::{self, dither, resize, ColorMap},
    DynamicImage, GrayImage, ImageBuffer, Luma,
};

use crate::printer::Orientation;

pub(crate) fn rasterize_image_to_ql_tiff(image: GrayImage) -> Vec<[u8; 90]> {
    let width = image.width() as usize;
    let height = image.height() as usize;

    let mut lines = Vec::with_capacity(width);
    for row in 0..height {
        let mut line = [0; 90]; // Always 90 for regular sized printers like the QL-700 (with a 0x00 byte to start)
                                // let mut line_byte = 7;
                                // Bit index counts backwards
                                // First nibble (bits 7 through 4) in the second byte is blank
                                // let mut line_bit_index: i8 = 0;
        for col in 0_usize..720 {
            let line_byte = ((719 / 8) - (col as isize / 8)) as usize;
            let line_bit_index = col % 8;
            if col >= width {
                break;
            }
            let luma_pixel = image.get_pixel(col as u32, row as u32); // + 3 was here in TS code -- not sure if needed
            let value: u8 = if luma_pixel[0] > 0xFF / 2 { 0 } else { 1 };
            line[line_byte] |= value << line_bit_index;
        }
        lines.push(line);
    }
    lines
}

pub(crate) fn dither_luma8_image(image: &mut GrayImage) {
    struct BlackAndWhite {}
    impl ColorMap for BlackAndWhite {
        type Color = Luma<u8>;

        fn index_of(&self, color: &Self::Color) -> usize {
            if color.0[0] < (u8::MAX) / 2 {
                0
            } else {
                1
            }
        }

        fn map_color(&self, color: &mut Self::Color) {
            if color.0[0] < (u8::MAX) / 2 {
                color.0[0] = u8::MIN;
            } else {
                color.0[0] = u8::MAX;
            }
        }

        fn lookup(&self, index: usize) -> Option<Self::Color> {
            match index {
                0 => Some(Luma::<u8>::from([u8::MIN])),
                1 => Some(Luma::<u8>::from([u8::MAX])),
                _ => None,
            }
        }

        fn has_lookup(&self) -> bool {
            true
        }
    }

    let color_map = BlackAndWhite {};
    dither(image, &color_map)
}

pub(crate) fn convert_image_to_luma_u8(image: DynamicImage) -> GrayImage {
    match image {
        DynamicImage::ImageLuma8(i) => i,
        DynamicImage::ImageLumaA8(i) => i.convert(),
        DynamicImage::ImageLuma16(i) => i.convert(),
        DynamicImage::ImageLumaA16(i) => i.convert(),
        DynamicImage::ImageRgb8(i) => i.convert(),
        DynamicImage::ImageRgba8(i) => i.convert(),
        DynamicImage::ImageRgb16(i) => i.convert(),
        DynamicImage::ImageRgba16(i) => i.convert(),
        DynamicImage::ImageRgb32F(i) => i.convert(),
        DynamicImage::ImageRgba32F(i) => i.convert(),
        _ => unimplemented!(),
    }
}

pub(crate) fn resize_and_rotate_image<I>(image: I, orientation: Orientation, final_width: u32) -> I
where
    I: image::GenericImageView,
    I::Pixel: 'static,
    <I::Pixel as image::Pixel>::Subpixel: 'static,
    I: From<
        ImageBuffer<
            <I as image::GenericImageView>::Pixel,
            Vec<<<I as image::GenericImageView>::Pixel as image::Pixel>::Subpixel>,
        >,
    >,
{
    let owidth = image.width();
    let oheight = image.height();

    let nwidth = final_width;
    let nheight = match orientation {
        Orientation::Normal => {
            (f64::from(nwidth) / (f64::from(owidth)) * f64::from(oheight)).floor() as u32
        }
        Orientation::Rotated => {
            (f64::from(nwidth) / (f64::from(oheight)) * f64::from(owidth)).floor() as u32
        }
    };

    let image = match orientation {
        Orientation::Normal => image,
        Orientation::Rotated => imageops::rotate90(&image).into(),
    };

    resize(
        &image,
        nwidth,
        nheight,
        image::imageops::FilterType::Lanczos3,
    )
    .into()
}
