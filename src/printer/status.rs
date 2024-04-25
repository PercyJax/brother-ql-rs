//! A representation of the status message Brother QL printers use
//!
//! Includes:
//! * Model name
//! * Loaded media
//! * Current operation
//! * Any errors that have occurred
//!
use super::constants::*;
#[derive(Debug, Clone, Copy)]
pub enum MediaType {
    None,
    ContinuousTape,
    DieCutLabels,
}

#[derive(Debug, Clone, Copy)]
pub struct Media {
    pub media_type: MediaType,
    pub width: u8,
    pub length: u8,
}
impl Media {
    pub fn to_label(&self) -> Label {
        let length = if self.length == 0 {
            None
        } else {
            Some(self.length)
        };
        label_data(self.width, length).expect("Printer reported invalid label dimensions")
    }
}

#[derive(Debug, PartialEq)]
pub enum StatusType {
    ReplyToStatusRequest,
    PrintingCompleted,
    ErrorOccurred,
    TurnedOff,
    Notification,
    PhaseChange,
}

#[derive(Debug, PartialEq)]
pub enum PhaseType {
    WaitingToReceive,
    PrintingState,
}

#[derive(Debug, PartialEq)]
pub enum Notification {
    NotAvailable,
    CoolingStarted,
    CoolingFinished,
}

#[derive(Debug)]
pub struct Response {
    pub model: &'static str,
    pub status_type: StatusType,
    pub errors: Vec<&'static str>,
    pub phase_type: PhaseType,
    pub notification: Notification,
    pub media: Media,
}
