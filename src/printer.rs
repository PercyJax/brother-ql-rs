//! Everything to do with USB protocol for Brother QL printers
//!
//! Based on the published [Brother QL Series Command Reference](https://download.brother.com/welcome/docp000678/cv_qlseries_eng_raster_600.pdf)
//! Updated and now verified on the [800 Series Documentation](https://download.brother.com/welcome/docp100278/cv_ql800_eng_raster_101.pdf)

use std::convert::TryInto;
use std::time::Duration;
use std::{thread, time::Instant};

use image::DynamicImage;
use thiserror::Error;

use crate::printer::status::{PhaseType, StatusType};
use crate::utils;

use self::constants::{PRINTER_STATUS_SIZE, TIMEOUTS};

pub mod constants;
pub mod job;
pub mod status;

#[derive(Error, Debug)]
pub enum PrinterError {
    #[error("usb")]
    Usb(#[from] rusb::Error),
    #[error("device error: {0}")]
    Device(String),
    #[error("printer error: {0}")]
    Printer(String),
}

type Result<T> = std::result::Result<T, PrinterError>;

fn printer_filter<T: rusb::UsbContext>(device: &rusb::Device<T>) -> bool {
    let descriptor = device.device_descriptor().unwrap();
    if descriptor.vendor_id() == constants::VENDOR_ID && descriptor.product_id() == 0x2049 {
        eprintln!("You must disable Editor Lite mode on your QL-700 before you can print with it");
    }
    descriptor.vendor_id() == constants::VENDOR_ID
        && constants::printer_name_from_id(descriptor.product_id()).is_some()
}

/// Get a vector of all attached and supported Brother QL printers as USB devices from which `ThermalPrinter` structs can be initialized.
pub fn printers() -> Vec<rusb::Device<rusb::GlobalContext>> {
    rusb::DeviceList::new()
        .unwrap()
        .iter()
        .filter(printer_filter)
        .collect()
}

const RASTER_LINE_LENGTH: u8 = 90;

/// The primary interface for dealing with Brother QL printers. Handles all USB communication with the printer.
pub struct ThermalPrinter<T: rusb::UsbContext> {
    pub manufacturer: String,
    pub model: String,
    pub serial_number: String,
    handle: rusb::DeviceHandle<T>,
    in_endpoint: u8,
    out_endpoint: u8,
}
impl<T: rusb::UsbContext> std::fmt::Debug for ThermalPrinter<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {} ({})",
            self.manufacturer, self.model, self.serial_number
        )
    }
}

#[derive(Debug)]
enum State {
    Waiting,
    PrintingStarted,
    PrintingFinished,
    Cooling,
    Errored,
}

/// Orientation of the label
///
/// Normal: label is printed so that you can read text when looking straight on
/// Rotated: label is printed so that you have to turn your head to read the text being printed
pub enum Orientation {
    Normal,
    Rotated,
}
impl<T: rusb::UsbContext> ThermalPrinter<T> {
    /// Create a new `ThermalPrinter` instance using a `rusb` USB device handle.
    ///
    /// Obtain list of connected device handles by calling `printers()`.
    pub fn new(device: rusb::Device<T>) -> Result<Self> {
        let mut handle = device.open()?;
        let mut in_endpoint: Option<u8> = None;
        let mut out_endpoint: Option<u8> = None;

        let config = device.active_config_descriptor()?;
        let interface = config.interfaces().next().ok_or(PrinterError::Device(
            "Brother QL printers should have exactly one interface".into(),
        ))?;
        let interface_descriptor = interface.descriptors().next().ok_or(PrinterError::Device(
            "Brother QL printers should have exactly one interface descriptor".into(),
        ))?;
        for endpoint in interface_descriptor.endpoint_descriptors() {
            if endpoint.transfer_type() != rusb::TransferType::Bulk {
                return Err(PrinterError::Device(
                    "Brother QL printers are defined as using only bulk endpoint communication"
                        .into(),
                ));
            }
            match endpoint.direction() {
                rusb::Direction::In => in_endpoint = Some(endpoint.address()),
                rusb::Direction::Out => out_endpoint = Some(endpoint.address()),
            }
        }
        if in_endpoint.is_none() || out_endpoint.is_none() {
            return Err(PrinterError::Device(
                "Input or output endpoint not found".into(),
            ));
        }

        if let Ok(kd_active) = handle.kernel_driver_active(interface.number()) {
            if kd_active {
                handle.detach_kernel_driver(interface.number())?;
            }
        }
        handle.claim_interface(interface.number())?;

        let device_descriptor = device.device_descriptor()?;

        let printer = ThermalPrinter {
            manufacturer: handle.read_manufacturer_string_ascii(&device_descriptor)?,
            model: handle.read_product_string_ascii(&device_descriptor)?,
            serial_number: handle.read_serial_number_string_ascii(&device_descriptor)?,
            handle,
            in_endpoint: in_endpoint.unwrap(),
            out_endpoint: out_endpoint.unwrap(),
        };

        // Reset printer
        let clear_command = [0x00; 200];
        ThermalPrinter::write(&printer, &clear_command)?;
        let initialize_command = [0x1B, 0x40];
        ThermalPrinter::write(&printer, &initialize_command)?;

        ThermalPrinter::get_status(&printer)?;
        Ok(printer)
    }

    /// Resizes, rasterizes, and sends an image to the printer that is the width of the currently loaded label
    /// and the height of the image when scaled to the original aspect ratio (for Orientation::Normal) and
    /// rotated 90 degrees (for Orientation::Rotated).
    ///
    /// Only supported on endless labels. Untested behavior on die-cut labels
    pub fn print_image(
        &self,
        image: DynamicImage,
        orientation: Orientation,
        dither: bool,
        copies: usize,
    ) -> Result<status::Response> {
        let status = self.get_status()?;

        // Resize and Rotate
        let image = utils::resize_and_rotate_image(
            image,
            orientation,
            status.media.to_label().dots_printable.0,
        );

        // Grayscale
        let mut image = utils::convert_image_to_luma_u8(image);

        // Dither
        if dither {
            utils::dither_luma8_image(&mut image);
        }

        // Rasterize
        let lines = utils::rasterize_image_to_ql_tiff(image);

        // Print
        self.cmd_print(lines, copies, 1)?;

        self.cmd_status_request()
    }

    /// Invalidate
    ///
    /// Send 400 bytes of 0x00
    fn cmd_invalidate(&self) {
        loop {
            match self.write(&[0x00_u8; 400]) {
                Ok(_) => break,
                Err(PrinterError::Usb(rusb::Error::Busy)) => {
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
                Err(e) => {
                    eprintln!("unexpected error encountered during invalidate: {e}");
                }
            }
        }
    }

    /// Initialize
    ///
    /// Sends the initialize request
    fn cmd_initialize(&self) -> Result<()> {
        self.write_with_timeout(&[0x00_u8; 400], TIMEOUTS.general)
    }

    /// Status information request
    ///
    /// Request status from printer and wait until status is returned
    fn cmd_status_request(&self) -> Result<status::Response> {
        let command = [0x1B, 0x69, 0x53];
        self.write_with_timeout(&command, Duration::from_secs(5))?;

        let start = Instant::now();
        loop {
            let status = self.read();

            if Instant::now() > start + TIMEOUTS.general {
                return Err(PrinterError::Usb(rusb::Error::Timeout));
            }

            match status {
                Err(PrinterError::Usb(rusb::Error::Timeout)) => (),
                Err(e) => return Err(e),
                Ok(r) => return Ok(r),
            }
        }
    }

    /// Send control codes
    fn cmd_control_codes(&self, media: status::Media, num_lines: u32, cut_each: u8) -> Result<()> {
        let mut new_job = job::Info::new(media, num_lines);
        new_job.cut_each = cut_each;
        self.write_with_timeout(new_job.serialize().as_slice(), TIMEOUTS.general)
    }

    /// Send raster data/main print loop
    fn cmd_print(&self, lines: Vec<[u8; 90]>, copies: usize, cut_each: u8) -> Result<()> {
        // Invalidate
        self.cmd_invalidate();

        // Initialize
        self.cmd_initialize()?;

        // Status Information Request
        let status = self.cmd_status_request()?;
        let PhaseType::WaitingToReceive = status.phase_type else {
            return Err(PrinterError::Printer("printer in invalid phase".into()));
        };

        let mut state = State::Waiting;

        // Print Loop
        let mut printed_copies = 0;
        loop {
            // Control Codes
            self.cmd_control_codes(
                status.media,
                lines
                    .len()
                    .try_into()
                    .expect("cannot cast result from lines.len() into u32"),
                cut_each,
            )?;

            // Send raster data
            for line in lines.iter() {
                let mut raster_command = vec![0x67, 0x00, RASTER_LINE_LENGTH];
                raster_command.extend_from_slice(line);
                'line: loop {
                    match state {
                        State::Waiting | State::PrintingStarted => (),
                        e => {
                            return Err(PrinterError::Printer(format!(
                                "unexpected status at start of line print: {e:?}"
                            )))
                        }
                    }
                    if let Err(_) = self.write_with_timeout(&raster_command, TIMEOUTS.line_print) {
                        // Only acceptable error in sending raster line here is for cooling
                        self.read_loop(&mut state);
                        let State::PrintingStarted = state else {
                            return Err(PrinterError::Printer(format!(
                                "unexpected state during cooldown: {state:?}"
                            )));
                        };
                    }
                    break 'line;
                }
            }

            if copies > (printed_copies + 1) {
                // Print without feeding
                self.write_with_timeout(&[0x0c], TIMEOUTS.line_print)?;
            } else {
                // Print with feeding
                self.write_with_timeout(&[0x1a], TIMEOUTS.line_print)?;
            };

            // Verify
            self.read_loop(&mut state);
            let State::Waiting = state else {
                return Err(PrinterError::Printer(format!(
                    "unexpected state during verification: {state:?}"
                )));
            };

            printed_copies += 1;

            if printed_copies >= copies {
                break;
            }
        }
        Ok(())
    }

    /// Wait for feedback
    ///
    /// Wait for phase change notifications, cooldown notifications, errors, and ready-to-receive
    fn read_loop(&self, state: &mut State) {
        loop {
            let Ok(status) = self.read() else {
                *state = State::Errored;
                return;
            };
            match state {
                State::Waiting => {
                    let PhaseType::PrintingState = status.phase_type else {
                        *state = State::Errored;
                        return;
                    };
                    *state = State::PrintingStarted;
                    continue;
                }
                State::PrintingStarted => match status.status_type {
                    StatusType::PrintingCompleted => {
                        *state = State::PrintingFinished;
                        continue;
                    }
                    StatusType::Notification => match status.notification {
                        status::Notification::CoolingStarted => {
                            *state = State::Cooling;
                            continue;
                        }
                        _ => {
                            *state = State::Errored;
                            return;
                        }
                    },
                    _ => {
                        *state = State::Errored;
                        return;
                    }
                },
                State::PrintingFinished => match status.status_type {
                    StatusType::PhaseChange => {
                        *state = State::Waiting;
                        return;
                    }
                    _ => {
                        *state = State::Errored;
                        return;
                    }
                },
                State::Cooling => match status.status_type {
                    StatusType::Notification => match status.notification {
                        status::Notification::CoolingFinished => {
                            *state = State::PrintingStarted;
                            return;
                        }
                        _ => {
                            *state = State::Errored;
                            return;
                        }
                    },
                    _ => {
                        *state = State::Errored;
                        return;
                    }
                },
                State::Errored => {
                    return;
                }
            }
        }
    }

    /// Get the currently loaded label size.
    pub fn current_label(&self) -> Result<constants::Label> {
        let media = self.get_status()?.media;
        constants::label_data(
            media.width,
            match media.length {
                0 => None,
                _ => Some(media.length),
            },
        )
        .ok_or(PrinterError::Printer(
            "Unknown media loaded in printer".into(),
        ))
    }

    /// Get the current status of the printer including possible errors, media type, and model name.
    pub fn get_status(&self) -> Result<status::Response> {
        let status_command = [0x1B, 0x69, 0x53];
        self.write(&status_command)?;
        self.read()
    }

    fn read(&self) -> Result<status::Response> {
        let mut response = [0; PRINTER_STATUS_SIZE];
        loop {
            let bytes_read = self.handle.read_bulk(
                self.in_endpoint,
                &mut response,
                Duration::from_millis(500),
            )?;
            if bytes_read == 0 {
                thread::sleep(TIMEOUTS.cooldown);
                continue;
            } else if bytes_read != PRINTER_STATUS_SIZE || response[0] != 0x80 {
                eprint!("invalid response received from printer: {response:?}");
            }
            break;
        }
        Self::interpret_response(response)
    }

    fn interpret_response(response: [u8; PRINTER_STATUS_SIZE]) -> Result<status::Response> {
        // response[0]: Print head mark - Fixed at 80h
        // response[1]: Size - Fixed at 20h
        // response[2]: Reserved - Fixed at “B” (42h)
        if response[0..=2] != [0x80, 0x20, 0x42] {
            return Err(PrinterError::Printer(format!(
                "Invalid response from printer: response[0..=2] != [0x80, 0x20, 0x42] = {:?}",
                &response[0..=2],
            )));
        }

        // response[3]: Series code
        match response[3] {
            // 0x30 => (), // some printers that are not the 800-series will have this. uncomment once tested
            0x34 => (),
            _ => {
                return Err(PrinterError::Printer(format!(
                    "Invalid response from printer: response[3] = {:?}",
                    response[3]
                )))
            }
        }

        // response[4]: Model code
        let model = match response[4] {
            // uncomment each once tested
            // 0x4F => "QL-500/550",
            // 0x31 => "QL-560",
            // 0x32 => "QL-570",
            // 0x33 => "QL-580N",
            // 0x51 => "QL-650TD",
            // 0x35 => "QL-700",
            0x38 => "QL-800",
            // 0x39 => "QL-810W",
            // 0x41 => "QL-820NWB",
            // 0x50 => "QL-1050",
            // 0x34 => "QL-1060N",
            _ => return Err(PrinterError::Printer("Unknown model".into())),
        };

        // response[5]: Reserved - Fixed at “0” (30h)
        if response[5] != 0x30 {
            return Err(PrinterError::Printer(format!(
                "Invalid response from printer: response[5] = {:?}",
                response[5]
            )));
        }

        // response[6]: Reserved - Fixed at “0” (30h) [seems to differ in older printers]
        match response[6] {
            0x30 => (), // 800 series
            0x00 => (), // previous models
            _ => {
                return Err(PrinterError::Printer(
                    "Invalid response from printer".into(),
                ))
            }
        }

        // response[7]: Reserved - Fixed at “00h”
        if response[7] != 0x00 {
            return Err(PrinterError::Printer(format!(
                "Invalid response from printer: response[7] = {:?}",
                response[7]
            )));
        }

        // response[8]: Error information 1
        // response[9]: Error information 2
        let mut errors = Vec::new();
        fn error_if(byte: u8, flag: u8, message: &'static str, errors: &mut Vec<&'static str>) {
            if byte & flag != 0 {
                errors.push(message);
            }
        }
        error_if(response[8], 0x01, "No media when printing", &mut errors);
        error_if(response[8], 0x02, "End of media (die-cut)", &mut errors);
        error_if(response[8], 0x04, "Tape cutter jam", &mut errors);
        error_if(response[8], 0x10, "Main unit in use", &mut errors);
        error_if(response[8], 0x20, "Printer turned off", &mut errors);
        error_if(response[8], 0x40, "High-voltage adapter", &mut errors);
        error_if(response[8], 0x80, "Fan doesn't work", &mut errors);
        error_if(response[9], 0x01, "Replace Media", &mut errors);
        error_if(response[9], 0x02, "Expansion buffer full", &mut errors);
        error_if(response[9], 0x04, "Communication error", &mut errors);
        error_if(response[9], 0x08, "Communication buffer full", &mut errors);
        error_if(response[9], 0x10, "Cover open", &mut errors);
        error_if(response[9], 0x20, "Cancel key", &mut errors);
        error_if(response[9], 0x40, "Cannot feed", &mut errors);
        error_if(response[9], 0x80, "System error", &mut errors);

        // response[10]: Media width
        let width = response[10];

        // response[11]: Media type
        let media_type = match response[11] {
            0x0A | 0x4A => status::MediaType::ContinuousTape,
            0x0B | 0x4B => status::MediaType::DieCutLabels,
            0x00 => status::MediaType::None,
            _ => return Err(PrinterError::Printer("unknown media type".into())),
        };

        // response[12]: Reserved - Fixed at 00h
        // response[13]: Reserved - Fixed at 00h
        if response[12..=13] != [0x00, 0x00] {
            return Err(PrinterError::Printer(format!(
                "Invalid response from printer: response[12..=13] = {:?}",
                &response[12..=13]
            )));
        }

        // response[14]: Reserved - Fixed at 3Fh or Unset
        // response[15]: Mode - Unset/unknown

        // response[16]: Reserved - Fixed at 00h
        if response[16] != 0x00 {
            return Err(PrinterError::Printer(format!(
                "Invalid response from printer: response[16] = {:?}",
                response[16]
            )));
        }

        // response[17]: Media length
        let length = response[17];

        // response[18]: Status type
        let status_type = match response[18] {
            0x00 => status::StatusType::ReplyToStatusRequest,
            0x01 => status::StatusType::PrintingCompleted,
            0x02 => status::StatusType::ErrorOccurred,
            0x04 => status::StatusType::TurnedOff,
            0x05 => status::StatusType::Notification,
            0x06 => status::StatusType::PhaseChange,
            _ => {
                return Err(PrinterError::Printer(
                    "Invalid status type (response[18]) from printer".into(),
                ))
            }
        };

        // response[19]: Phase type
        // response[20]: Phase number (higher order bytes)
        // response[21]: Phase number (lower order bytes)
        let phase_type = match response[19] {
            0x00 => PhaseType::WaitingToReceive,
            0x01 => PhaseType::PrintingState,
            _ => {
                return Err(PrinterError::Printer(format!(
                    "Invalid phase type: response[19] = {:?}",
                    response[19]
                )))
            }
        };
        // TODO:: figure out what phase numbers mean

        // response[22]: Notification number
        let notification = match response[22] {
            0x00 => status::Notification::NotAvailable,
            0x03 => status::Notification::CoolingStarted,
            0x04 => status::Notification::CoolingFinished,
            _ => {
                return Err(PrinterError::Printer(format!(
                    "Invalid notification: response[22] = {:?}",
                    response[22]
                )))
            }
        };

        // response[23]: Reserved - Fixed at 00h
        // response[24..=31]: Reserved - Fixed at 00h
        //   this is not always true - response[25] seems to be 0x01 at least some times

        Ok(status::Response {
            model,
            status_type,
            errors,
            media: status::Media {
                media_type,
                width,
                length,
            },
            phase_type,
            notification,
        })
    }

    fn write(&self, data: &[u8]) -> Result<()> {
        self.write_with_timeout(data, Duration::from_millis(500))
    }

    fn write_with_timeout(&self, data: &[u8], timeout: Duration) -> Result<()> {
        self.handle.write_bulk(self.out_endpoint, data, timeout)?;
        Ok(())
    }
}

// #[cfg(test)]
// mod tests {
//     use crate::printer::{printers, ThermalPrinter};
//     #[test]
//     fn connect() {
//         let printer_list = printers();
//         assert!(printer_list.len() > 0, "No printers found");
//         let mut printer = ThermalPrinter::new(printer_list.into_iter().next().unwrap()).unwrap();
//         printer.init().unwrap();
//     }

//     use std::path::PathBuf;
//     #[test]
//     #[ignore]
//     fn print() {
//         let printer_list = printers();
//         assert!(printer_list.len() > 0, "No printers found");
//         let mut printer = ThermalPrinter::new(printer_list.into_iter().next().unwrap()).unwrap();
//         let label = printer.init().unwrap().media.to_label();

//         let mut rasterizer =
//             crate::text::TextRasterizer::new(label, PathBuf::from("./Space Mono Bold.ttf"));
//         rasterizer.set_second_row_image(PathBuf::from("./logos/BuildGT Mono.png"));
//         let lines = rasterizer.rasterize("Ryan Petschek", Some("Computer Science"), 1.2, false);

//         dbg!(printer.print(lines).unwrap());
//     }
// }
