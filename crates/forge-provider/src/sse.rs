use crate::{ErrorKind, ProviderError};
use std::collections::VecDeque;

#[derive(Default)]
pub(crate) struct SseParser {
    line: Vec<u8>,
    data: Vec<String>,
    after_cr: bool,
    pub events: VecDeque<String>,
}
impl SseParser {
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), ProviderError> {
        for &byte in bytes {
            if self.after_cr {
                self.after_cr = false;
                if byte == b'\n' {
                    continue;
                }
            }
            match byte {
                b'\r' | b'\n' => {
                    let line = std::mem::take(&mut self.line);
                    let line = std::str::from_utf8(&line).map_err(|_| ProviderError {
                        code: None,
                        kind: ErrorKind::InvalidResponse,
                        message: "invalid SSE UTF-8".into(),
                    })?;
                    if line.is_empty() {
                        if !self.data.is_empty() {
                            self.events.push_back(self.data.join("\n"));
                            self.data.clear();
                        }
                    } else if let Some(value) = line.strip_prefix("data:") {
                        self.data
                            .push(value.strip_prefix(' ').unwrap_or(value).to_owned());
                    } else if line == "data" {
                        self.data.push(String::new());
                    }
                    self.after_cr = byte == b'\r';
                }
                _ => self.line.push(byte),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_split_matches_unsplit_with_comments_multiline_crlf_and_unicode() {
        let input = ": keepalive\r\nevent: message\r\ndata: {\r\ndata: \"text\":\"世\"}\r\n\r\ndata: next\r\rdata\n\n";
        let expected = [
            "{\n\"text\":\"世\"}".to_owned(),
            "next".to_owned(),
            String::new(),
        ];
        for split in 0..=input.len() {
            let mut parser = SseParser::default();
            parser.push(&input.as_bytes()[..split]).unwrap();
            parser.push(&input.as_bytes()[split..]).unwrap();
            assert_eq!(parser.events, expected);
            parser.events.clear();
            parser.push(b"data: fresh\n\n").unwrap();
            assert_eq!(parser.events, ["fresh"]);
        }
        let mut parser = SseParser::default();
        for byte in input.as_bytes() {
            parser.push(&[*byte]).unwrap();
        }
        assert_eq!(parser.events, expected);
    }
    #[test]
    fn invalid_utf8_is_invalid_response_and_unterminated_data_is_not_an_event() {
        let mut parser = SseParser::default();
        parser.push(b"data: partial").unwrap();
        assert!(parser.events.is_empty());
        assert_eq!(
            parser.push(b"\xff\n").unwrap_err().kind,
            ErrorKind::InvalidResponse
        );
    }
}
