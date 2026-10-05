use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_REPORT_BYTES: usize = 4096;
pub const MAX_LINE_BYTES: usize = 65_536;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub path_hex: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub serial_number: Option<String>,
    pub manufacturer_string: Option<String>,
    pub product_string: Option<String>,
    pub release_number: u16,
    pub interface_number: i32,
    pub usage_page: u16,
    pub usage: u16,
    pub module: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Hardware,
    Synthetic,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Descriptor {
    Available { raw_hex: String },
    Error { message: String },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub schema_version: u32,
    pub source: Source,
    pub started_unix_micros: u64,
    pub identity: Identity,
    pub software_mode_requested: bool,
    pub report_descriptor: Descriptor,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub timestamp_micros: u64,
    pub raw_hex: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    DurationElapsed,
    MaxReports,
    TransportError,
    /// Ctrl-C or SIGTERM ended the capture; reports before it are complete.
    Interrupted,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct End {
    pub timestamp_micros: u64,
    pub report_count: u64,
    pub reason: EndReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Record {
    Session(Session),
    Report(Report),
    End(End),
}

pub fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(DIGITS[(byte >> 4) as usize] as char);
        encoded.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    encoded
}

pub fn decode_hex(raw: &str, maximum_bytes: usize) -> Result<Vec<u8>, String> {
    if !raw.len().is_multiple_of(2) {
        return Err("hex must contain an even number of digits".into());
    }
    if raw.len() / 2 > maximum_bytes {
        return Err(format!("hex exceeds the {maximum_bytes}-byte limit"));
    }
    raw.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let high = hex_digit(pair[0])?;
            let low = hex_digit(pair[1])?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn hex_digit(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("hex must contain only ASCII hexadecimal digits".into()),
    }
}

pub fn write_record(writer: &mut impl Write, record: &Record) -> io::Result<()> {
    let line = serde_json::to_vec(record).map_err(io::Error::other)?;
    if line.len() + 1 > MAX_LINE_BYTES {
        return Err(io::Error::other("capture record exceeds line-size limit"));
    }
    writer.write_all(&line)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

/// A bounded, single-session reader. A required footer detects unfinished captures.
pub struct CaptureReader<R> {
    reader: R,
    line_number: usize,
    report_count: u64,
    last_timestamp: u64,
    end: Option<End>,
}

impl<R: BufRead> CaptureReader<R> {
    pub fn new(mut reader: R) -> Result<(Self, Session), String> {
        let record = read_line(&mut reader, 1)?
            .ok_or_else(|| "capture is empty; expected a session header".to_string())?;
        let Record::Session(session) = record else {
            return Err("line 1: expected a session header".into());
        };
        validate_session(&session).map_err(|error| format!("line 1: {error}"))?;
        Ok((
            Self {
                reader,
                line_number: 1,
                report_count: 0,
                last_timestamp: 0,
                end: None,
            },
            session,
        ))
    }

    pub fn next_report(&mut self) -> Result<Option<Report>, String> {
        if self.end.is_some() {
            return Ok(None);
        }
        self.line_number += 1;
        let record = read_line(&mut self.reader, self.line_number)?.ok_or_else(|| {
            format!(
                "line {}: truncated capture; missing end record",
                self.line_number
            )
        })?;
        match record {
            Record::Session(_) => Err(format!(
                "line {}: multiple sessions are not supported",
                self.line_number
            )),
            Record::Report(report) => {
                self.check_timestamp(report.timestamp_micros)?;
                let raw = decode_hex(&report.raw_hex, MAX_REPORT_BYTES)
                    .map_err(|error| format!("line {}: {error}", self.line_number))?;
                if raw.is_empty() {
                    return Err(format!("line {}: empty input report", self.line_number));
                }
                self.report_count += 1;
                Ok(Some(report))
            }
            Record::End(end) => {
                self.check_timestamp(end.timestamp_micros)?;
                if end.report_count != self.report_count {
                    return Err(format!(
                        "line {}: end report_count {} differs from {} reports",
                        self.line_number, end.report_count, self.report_count
                    ));
                }
                if (end.reason == EndReason::TransportError) != end.error.is_some() {
                    return Err(format!(
                        "line {}: error is required only for transport_error",
                        self.line_number
                    ));
                }
                if end
                    .error
                    .as_ref()
                    .is_some_and(|message| message.trim().is_empty())
                {
                    return Err(format!(
                        "line {}: transport error message must be nonempty",
                        self.line_number
                    ));
                }
                if read_line(&mut self.reader, self.line_number + 1)?.is_some() {
                    return Err(format!(
                        "line {}: data after end record",
                        self.line_number + 1
                    ));
                }
                self.end = Some(end);
                Ok(None)
            }
        }
    }

    pub fn end(&self) -> Option<&End> {
        self.end.as_ref()
    }

    fn check_timestamp(&mut self, timestamp: u64) -> Result<(), String> {
        if timestamp < self.last_timestamp {
            return Err(format!(
                "line {}: timestamps must be nondecreasing",
                self.line_number
            ));
        }
        self.last_timestamp = timestamp;
        Ok(())
    }
}

fn read_line(reader: &mut impl BufRead, line_number: usize) -> Result<Option<Record>, String> {
    use std::io::Read;
    let mut line = Vec::new();
    reader
        .take((MAX_LINE_BYTES + 1) as u64)
        .read_until(b'\n', &mut line)
        .map_err(|error| format!("line {line_number}: could not read capture: {error}"))?;
    if line.is_empty() {
        return Ok(None);
    }
    if line.len() > MAX_LINE_BYTES {
        return Err(format!("line {line_number}: line-size limit exceeded"));
    }
    if line.last() != Some(&b'\n') {
        return Err(format!(
            "line {line_number}: truncated line; missing newline"
        ));
    }
    serde_json::from_slice(&line)
        .map(Some)
        .map_err(|error| format!("line {line_number}: invalid capture record: {error}"))
}

fn validate_session(session: &Session) -> Result<(), String> {
    if session.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "unsupported schema_version {}; expected {SCHEMA_VERSION}",
            session.schema_version
        ));
    }
    let path = decode_hex(&session.identity.path_hex, crate::device::MAX_PATH_BYTES)?;
    if path.is_empty() || path.contains(&0) {
        return Err("identity path must be nonempty and contain no NUL bytes".into());
    }
    if session.software_mode_requested
        && !crate::mode::supported(cooltek_sdk_masterhub::ModuleKind::from_ids(
            session.identity.vendor_id,
            session.identity.product_id,
        ))
    {
        return Err("software mode is supported only for M131E and M4315".into());
    }
    match &session.report_descriptor {
        Descriptor::Available { raw_hex } => {
            if decode_hex(raw_hex, MAX_REPORT_BYTES)?.is_empty() {
                return Err("available descriptor must contain at least one byte".into());
            }
        }
        Descriptor::Error { message } if message.is_empty() => {
            return Err("descriptor error message must be nonempty".into());
        }
        Descriptor::Error { .. } => {}
    }
    Ok(())
}
