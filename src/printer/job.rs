use crate::printer::status;

use super::status::Media;

pub struct Info {
    pub media: Media,
    pub num_lines: u32,
    pub page: Page,
    pub prioritize_quality: bool,
    pub cut_each: u8,
    pub auto_cut: bool,
    pub cut_at_end: bool,
    pub high_resolution: bool,
}

pub enum Page {
    Starting,
    Other,
}

impl Info {
    pub fn new(media: Media, num_lines: u32) -> Self {
        Self {
            media,
            num_lines,
            page: Page::Other,
            prioritize_quality: true,
            cut_each: 1,
            auto_cut: true,
            cut_at_end: true,
            high_resolution: false,
        }
    }
    pub fn serialize(&self) -> Vec<u8> {
        let mut command = vec![];

        {
            // print information command
            const MEDIA_TYPE: u8 = 0x02;
            const MEDIA_WIDTH: u8 = 0x04;
            const MEDIA_LENGTH: u8 = 0x08;
            const PRIORITY_GIVEN_TO_PRINT_QUALITY: u8 = 0x40;
            const PRINTER_RECOVERY_ALWAYS_ON: u8 = 0x80;

            let mut command_fragment = [
                0x1B,
                0x69,
                0x7A,
                MEDIA_TYPE
                    | MEDIA_WIDTH
                    | MEDIA_LENGTH
                    | (PRIORITY_GIVEN_TO_PRINT_QUALITY & ((self.prioritize_quality as u8) << 6)
                        | PRINTER_RECOVERY_ALWAYS_ON),
                match self.media.media_type {
                    status::MediaType::ContinuousTape => 0x0A,
                    status::MediaType::DieCutLabels => 0x0B,
                    status::MediaType::None => panic!("no media loaded"),
                },
                self.media.width,
                self.media.length,
                0,
                0,
                0,
                0,
                match self.page {
                    Page::Starting => 0,
                    Page::Other => 1,
                },
                0,
            ];
            command_fragment[7..7 + 4].copy_from_slice(&self.num_lines.to_le_bytes());
            command.extend(command_fragment);
        }

        {
            // cut each page number
            let command_fragment = [0x1B, 0x69, 0x41, self.cut_each];
            command.extend(command_fragment);
        }

        {
            // various mode
            let command_fragment = [0x1B, 0x69, 0x4d, (self.auto_cut as u8) << 6];
            command.extend(command_fragment);
        }

        {
            // expanded mode
            let command_fragment = [
                0x1B,
                0x69,
                0x4b,
                (self.cut_at_end as u8) << 3 | (self.high_resolution as u8) << 6,
            ];
            command.extend(command_fragment);
        }

        {
            // margins
            let mut command_fragment = [0x1B, 0x69, 0x64, 0, 0];
            command_fragment[3..3 + 2]
                .copy_from_slice(&(self.media.to_label().feed_margin as u16).to_le_bytes());
            command.extend(command_fragment);
        }

        command
    }
}
