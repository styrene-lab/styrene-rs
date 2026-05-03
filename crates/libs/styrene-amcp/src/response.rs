use crate::error::{AmcpError, Result};

/// AMCP response codes:
///   200 = OK + multiline data (terminated by blank \r\n)
///   201 = OK + single-line data
///   202 = OK (no data)
///   400 = command not understood
///   401 = illegal video channel
///   402 = parameter missing
///   403 = illegal parameter
///   404 = media file not found
///   500 = internal server error
///   501 = internal server error (command-specific)
///   502 = media file unreadable
#[derive(Debug, Clone)]
pub struct AmcpResponse {
    pub code: u16,
    pub body: String,
}

impl AmcpResponse {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.code)
    }

    /// Whether this response code carries a multiline body (terminated by blank line).
    pub fn has_multiline_body(code: u16) -> bool {
        code == 200
    }

    /// Whether this response code carries a single-line data payload.
    pub fn has_single_line_body(code: u16) -> bool {
        code == 201
    }

    /// Whether this response code has no body at all.
    pub fn has_no_body(code: u16) -> bool {
        !Self::has_multiline_body(code) && !Self::has_single_line_body(code)
    }

    pub fn parse(raw: &str) -> Result<Self> {
        let mut lines = raw.lines();
        let status_line =
            lines.next().ok_or_else(|| AmcpError::Protocol("empty response".to_owned()))?;

        let code: u16 =
            status_line.split_whitespace().next().and_then(|s| s.parse().ok()).ok_or_else(
                || AmcpError::Protocol(format!("invalid status line: {status_line}")),
            )?;

        let body: String = lines.collect::<Vec<_>>().join("\n");

        let resp = Self { code, body };

        if resp.is_success() {
            Ok(resp)
        } else {
            Err(AmcpError::Server { code, message: resp.body })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_success_no_body() {
        let resp = AmcpResponse::parse("202 CG OK").unwrap();
        assert_eq!(resp.code, 202);
        assert!(resp.is_success());
        assert!(AmcpResponse::has_no_body(202));
    }

    #[test]
    fn parse_single_line_data() {
        let resp = AmcpResponse::parse("201 INFO OK\r\nsome data").unwrap();
        assert_eq!(resp.code, 201);
        assert!(AmcpResponse::has_single_line_body(201));
    }

    #[test]
    fn parse_multiline_data() {
        let resp = AmcpResponse::parse("200 OK\r\nline1\r\nline2").unwrap();
        assert_eq!(resp.code, 200);
        assert!(AmcpResponse::has_multiline_body(200));
        assert!(resp.body.contains("line1"));
    }

    #[test]
    fn parse_error() {
        let err = AmcpResponse::parse("404 ERROR").unwrap_err();
        match err {
            AmcpError::Server { code, .. } => assert_eq!(code, 404),
            _ => panic!("expected Server error"),
        }
    }

    #[test]
    fn parse_empty() {
        assert!(AmcpResponse::parse("").is_err());
    }

    #[test]
    fn code_classification() {
        assert!(AmcpResponse::has_multiline_body(200));
        assert!(!AmcpResponse::has_multiline_body(201));
        assert!(AmcpResponse::has_single_line_body(201));
        assert!(AmcpResponse::has_no_body(202));
        assert!(AmcpResponse::has_no_body(400));
        assert!(AmcpResponse::has_no_body(500));
    }
}
