use barcoders::{
    generators::{self},
    sym::ean13::EAN13,
};
use image::{ImageBuffer, Rgba};
use qrcodegen::QrCode;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum BarcodeError {
    #[error("overflow error: {0}")]
    Overflow(String),
}

pub enum EAN13Data {
    EncodedPrice { sku: usize, price: f32 },
    Simple(String),
}

pub fn generate_ean13_barcode(
    data: EAN13Data,
    _name: String,
    _description: String,
    _link: Option<String>,
) -> Result<image::ImageBuffer<image::Rgba<u8>, Vec<u8>>, BarcodeError> {
    let mut label = ImageBuffer::new(696, 150);
    label.iter_mut().for_each(|c| *c = u8::MAX);

    // Barcode
    {
        let data = match data {
            EAN13Data::EncodedPrice { sku, price } => {
                if sku > 99999 {
                    return Err(BarcodeError::Overflow("sku".into()));
                }
                let cents = (price * 100.0).floor() as usize;
                if cents > 99999 {
                    return Err(BarcodeError::Overflow("price".into()));
                }
                format!("20{:05}{:05}", sku, cents)
            }
            EAN13Data::Simple(s) => s,
        };

        let barcode = generators::image::Image::image_buffer(1)
            .generate_buffer(EAN13::new(data).unwrap().encode())
            .unwrap();

        let barcode =
            image::imageops::resize(&barcode, 200, 100, image::imageops::FilterType::Nearest);

        image::imageops::overlay(&mut label, &barcode, 496, 0);
    }

    /* // QR Code
    if let Some(link) = link {
        let qr = QrCode::encode_text(&link, qrcodegen::QrCodeEcc::High)
            .map_err(|_| BarcodeError::Overflow("link".into()))?;
        let size = qr.size().abs() as u32;
        if size < 21 || size > 177 {
            return Err(BarcodeError::Overflow("qr".into()));
        }

        let mut qr_img = ImageBuffer::new(size, size);

        for x in 0..size {
            for y in 0..size {
                qr_img.put_pixel(
                    x,
                    y,
                    match qr.get_module(x as i32, y as i32) {
                        true => {
                            println!("{x}, {y} = true");
                            Rgba([u8::MIN, u8::MIN, u8::MIN, u8::MAX])
                        }
                        false => {
                            println!("{x}, {y} = false");
                            Rgba([u8::MAX, u8::MAX, u8::MAX, u8::MAX])
                        }
                    },
                );
            }
        }

        let margin = 20;
        let final_size = 210;
        let qr_img = image::imageops::resize(
            &qr_img,
            final_size - (2 * margin),
            final_size - (2 * margin),
            image::imageops::FilterType::Nearest,
        );

        image::imageops::overlay(&mut label, &qr_img, margin as i64, margin as i64);
    } */

    Ok(label)
}

pub fn generate_barcode_large(
    sku: usize,
    price: f32,
    _name: String,
    _description: String,
    link: Option<String>,
) -> Result<image::ImageBuffer<image::Rgba<u8>, Vec<u8>>, BarcodeError> {
    let mut label = ImageBuffer::new(696, 270);
    label.iter_mut().for_each(|c| *c = u8::MAX);

    // Barcode
    {
        if sku > 99999 {
            return Err(BarcodeError::Overflow("sku".into()));
        }
        let cents = (price * 100.0).floor() as usize;
        if cents > 99999 {
            return Err(BarcodeError::Overflow("price".into()));
        }
        let data = format!("20{:05}{:05}", sku, cents);

        let barcode = generators::image::Image::image_buffer(1)
            .generate_buffer(EAN13::new(data).unwrap().encode())
            .unwrap();

        let barcode =
            image::imageops::resize(&barcode, 350, 230, image::imageops::FilterType::Nearest);

        image::imageops::overlay(&mut label, &barcode, 346, 0);
    }

    // QR Code
    if let Some(link) = link {
        let qr = QrCode::encode_text(&link, qrcodegen::QrCodeEcc::High)
            .map_err(|_| BarcodeError::Overflow("link".into()))?;
        let size = qr.size().abs() as u32;
        if size < 21 || size > 177 {
            return Err(BarcodeError::Overflow("qr".into()));
        }

        let mut qr_img = ImageBuffer::new(size, size);

        for x in 0..size {
            for y in 0..size {
                qr_img.put_pixel(
                    x,
                    y,
                    match qr.get_module(x as i32, y as i32) {
                        true => {
                            println!("{x}, {y} = true");
                            Rgba([u8::MIN, u8::MIN, u8::MIN, u8::MAX])
                        }
                        false => {
                            println!("{x}, {y} = false");
                            Rgba([u8::MAX, u8::MAX, u8::MAX, u8::MAX])
                        }
                    },
                );
            }
        }

        let margin = 20;
        let qr_img = image::imageops::resize(
            &qr_img,
            192 - (2 * margin),
            192 - (2 * margin),
            image::imageops::FilterType::Nearest,
        );

        image::imageops::overlay(
            &mut label,
            &qr_img,
            (69 + margin) as i64,
            (40 + margin) as i64,
        );
    }

    Ok(label)
}
