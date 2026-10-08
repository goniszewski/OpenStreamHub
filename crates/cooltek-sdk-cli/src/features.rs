//! Read-only Feature report probe: GET FEATURE for descriptor-declared IDs only.
//!
//! The probe is generic over [`ReadOnlyFeatureTransport`], which exposes only a
//! report descriptor read and a GET FEATURE. It has no SET FEATURE, output
//! write, or mode operation, so code holding it cannot change device state.
//! [`HidFeatureReader`] owns the opened HID device on a worker thread that is
//! generic over a private two-method source trait, so the worker body cannot
//! call anything but those two reads either.
//!
//! This is read-only at the HID protocol level: the host sends only
//! GET_REPORT(Feature). What the firmware does internally on a GET is unknown.

use std::{
    cell::Cell,
    error::Error,
    fmt,
    io::Write,
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};

use cooltek_sdk_masterhub::{
    descriptor::{DeclaredFeatureReport, declared_feature_reports},
    hidapi::{HidDevice, HidResult, MAX_REPORT_DESCRIPTOR_SIZE},
};
use serde_json::{Value, json};

use crate::capture::{Identity, SCHEMA_VERSION, encode_hex};

/// Why a read did not produce bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadError {
    /// The request reached the transport and failed or timed out.
    Failed(String),
    /// The request was never sent, for example behind a stalled request.
    NotSent(String),
}

impl fmt::Display for ReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed(message) => write!(formatter, "{message}"),
            Self::NotSent(message) => write!(formatter, "not sent: {message}"),
        }
    }
}

/// The only operations the feature probe can perform. Both are reads.
pub trait ReadOnlyFeatureTransport {
    /// Reads the HID report descriptor, returning its actual length.
    fn get_report_descriptor(
        &self,
        buffer: &mut [u8],
        timeout: Duration,
    ) -> Result<usize, ReadError>;

    /// GET FEATURE for the report ID in `buffer[0]`; returns bytes filled.
    fn get_feature_report(&self, buffer: &mut [u8], timeout: Duration) -> Result<usize, ReadError>;
}

/// Bounds and selection for one probe.
#[derive(Clone, Copy, Debug)]
pub struct ProbeSettings {
    /// Read just this declared ID instead of every declared Feature report.
    pub only_report_id: Option<u8>,
    /// Maximum wait for the descriptor and for each GET FEATURE.
    pub timeout: Duration,
    /// Total deadline for the whole probe; later reports are skipped after it.
    pub duration: Duration,
}

/// The two reads the worker may perform on the device it owns.
///
/// The worker body is generic over this trait, so the compiler rejects any
/// other device call there.
trait FeatureSource: Send + 'static {
    fn descriptor(&self, buffer: &mut [u8]) -> HidResult<usize>;
    fn get_feature(&self, buffer: &mut [u8]) -> HidResult<usize>;
}

impl FeatureSource for HidDevice {
    fn descriptor(&self, buffer: &mut [u8]) -> HidResult<usize> {
        self.get_report_descriptor(buffer)
    }
    fn get_feature(&self, buffer: &mut [u8]) -> HidResult<usize> {
        self.get_feature_report(buffer)
    }
}

/// Request sent to the device-owning worker. Only read operations exist.
#[derive(Debug)]
enum Request {
    Descriptor,
    Feature { report_id: u8, length: usize },
}

/// Owns an opened HID device on a worker thread and exposes only reads.
///
/// The device is moved into the worker; this type keeps no handle to it, so
/// no write method is reachable. A request that exceeds its timeout leaves the
/// worker busy; every later request is then reported as not sent instead of
/// queueing behind it. Exiting the process releases the device.
pub struct HidFeatureReader {
    requests: Sender<Request>,
    replies: Receiver<Result<Vec<u8>, String>>,
    stalled: Cell<bool>,
}

impl HidFeatureReader {
    /// Moves `device` onto a worker thread that serves only read requests.
    pub fn spawn(device: HidDevice) -> Self {
        Self::spawn_source(device)
    }

    fn spawn_source<S: FeatureSource>(source: S) -> Self {
        let (requests, worker_requests) = mpsc::channel::<Request>();
        let (worker_replies, replies) = mpsc::channel();
        thread::spawn(move || {
            for request in worker_requests {
                if worker_replies.send(serve(&source, request)).is_err() {
                    break;
                }
            }
        });
        Self {
            requests,
            replies,
            stalled: Cell::new(false),
        }
    }

    fn call(
        &self,
        request: Request,
        buffer: &mut [u8],
        timeout: Duration,
    ) -> Result<usize, ReadError> {
        if self.stalled.get() {
            return Err(ReadError::NotSent(
                "an earlier request is still pending on the device".into(),
            ));
        }
        self.requests
            .send(request)
            .map_err(|_| ReadError::NotSent("HID worker stopped".into()))?;
        match self.replies.recv_timeout(timeout) {
            Ok(Ok(bytes)) if bytes.len() <= buffer.len() => {
                buffer[..bytes.len()].copy_from_slice(&bytes);
                Ok(bytes.len())
            }
            Ok(Ok(bytes)) => Err(ReadError::Failed(format!(
                "transport returned {} bytes for a {}-byte buffer",
                bytes.len(),
                buffer.len()
            ))),
            Ok(Err(message)) => Err(ReadError::Failed(message)),
            Err(RecvTimeoutError::Timeout) => {
                self.stalled.set(true);
                Err(ReadError::Failed(format!(
                    "no reply within {} ms",
                    timeout.as_millis()
                )))
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err(ReadError::Failed("HID worker stopped".into()))
            }
        }
    }
}

/// Performs one read on the worker thread; over-long counts become errors.
fn serve<S: FeatureSource>(source: &S, request: Request) -> Result<Vec<u8>, String> {
    let (mut buffer, result) = match request {
        Request::Descriptor => {
            let mut buffer = vec![0; MAX_REPORT_DESCRIPTOR_SIZE];
            let result = source.descriptor(&mut buffer);
            (buffer, result)
        }
        Request::Feature { report_id, length } => {
            let mut buffer = vec![0; length.max(1)];
            buffer[0] = report_id;
            let result = source.get_feature(&mut buffer);
            (buffer, result)
        }
    };
    match result {
        Ok(received) if received <= buffer.len() => {
            buffer.truncate(received);
            Ok(buffer)
        }
        Ok(received) => Err(format!(
            "transport reported {received} bytes for a {}-byte buffer",
            buffer.len()
        )),
        Err(error) => Err(error.to_string()),
    }
}

impl ReadOnlyFeatureTransport for HidFeatureReader {
    fn get_report_descriptor(
        &self,
        buffer: &mut [u8],
        timeout: Duration,
    ) -> Result<usize, ReadError> {
        self.call(Request::Descriptor, buffer, timeout)
    }

    fn get_feature_report(&self, buffer: &mut [u8], timeout: Duration) -> Result<usize, ReadError> {
        let report_id = *buffer
            .first()
            .ok_or_else(|| ReadError::NotSent("feature buffer is empty".into()))?;
        let request = Request::Feature {
            report_id,
            length: buffer.len(),
        };
        self.call(request, buffer, timeout)
    }
}

/// Counts written in the closing summary line.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProbeSummary {
    /// GETs that returned bytes.
    pub ok: usize,
    /// GETs that reached the transport and failed or timed out.
    pub error: usize,
    /// Reports not started because the total deadline had elapsed.
    pub skipped: usize,
    /// Reports never sent, for example behind a stalled request.
    pub not_sent: usize,
}

/// Reads the descriptor, then GETs each declared Feature report once.
///
/// Writes one JSON line for the session, one per requested report, and a
/// summary. A failing GET is recorded and does not stop later reports. An
/// unreadable or malformed descriptor, or an undeclared `--report-id`, ends
/// the probe before any GET FEATURE and returns an error after the summary.
pub fn probe<T: ReadOnlyFeatureTransport + ?Sized>(
    transport: &T,
    identity: &Identity,
    settings: ProbeSettings,
    writer: &mut impl Write,
) -> Result<ProbeSummary, Box<dyn Error>> {
    probe_with_clock(transport, identity, settings, writer, &Instant::now)
}

/// [`probe`] with an injected monotonic clock, for deterministic tests.
pub fn probe_with_clock<T: ReadOnlyFeatureTransport + ?Sized>(
    transport: &T,
    identity: &Identity,
    settings: ProbeSettings,
    writer: &mut impl Write,
    now: &dyn Fn() -> Instant,
) -> Result<ProbeSummary, Box<dyn Error>> {
    let started = now();
    let elapsed = || now().saturating_duration_since(started);
    let mut header = json!({
        "type": "features_session",
        "schema_version": SCHEMA_VERSION,
        "source": "hardware",
        "read_only": true,
        "identity": identity,
        "timeout_ms": settings.timeout.as_millis(),
        "duration_ms": settings.duration.as_millis(),
    });
    let mut summary = ProbeSummary::default();

    let mut raw = vec![0; MAX_REPORT_DESCRIPTOR_SIZE];
    let descriptor = transport
        .get_report_descriptor(&mut raw, settings.timeout.min(settings.duration))
        .map_err(|error| error.to_string())
        .and_then(|length| match length {
            0 => Err("transport returned an empty report descriptor".to_owned()),
            length if length > raw.len() => {
                Err("transport returned an invalid descriptor length".to_owned())
            }
            length => Ok(&raw[..length]),
        });
    let descriptor = match descriptor {
        Ok(descriptor) => descriptor,
        Err(message) => {
            header["report_descriptor"] = json!({ "status": "error", "error": message });
            return stop(writer, &header, elapsed(), summary, message);
        }
    };
    header["report_descriptor"] = json!({
        "status": "available",
        "length": descriptor.len(),
        "raw_hex": encode_hex(descriptor),
    });
    let declared = match declared_feature_reports(descriptor) {
        Ok(declared) => declared,
        Err(error) => {
            let message = error.to_string();
            header["report_descriptor"]["parse_error"] = json!(message);
            return stop(writer, &header, elapsed(), summary, message);
        }
    };
    header["declared_feature_ids"] = declared.iter().map(|r| r.report_id).collect();
    header["declared_feature_reports"] = declared
        .iter()
        .map(|report| {
            json!({
                "report_id": report.report_id,
                "declared_bits": report.bits,
                "declared_length": report.length,
            })
        })
        .collect();

    let requested: Vec<DeclaredFeatureReport> = match settings.only_report_id {
        None => declared,
        Some(id) => match declared.into_iter().find(|report| report.report_id == id) {
            Some(report) => vec![report],
            None => {
                header["requested_report_ids"] = json!([]);
                let message = format!(
                    "report ID {id} ({id:#04x}) is not declared as a Feature report by this \
                     descriptor; nothing was requested"
                );
                return stop(writer, &header, elapsed(), summary, message);
            }
        },
    };
    header["requested_report_ids"] = requested.iter().map(|r| r.report_id).collect();
    write_line(writer, &header)?;

    for report in &requested {
        let mut line = json!({
            "type": "feature",
            "report_id": report.report_id,
            "declared_length": report.length,
        });
        let remaining = settings.duration.saturating_sub(elapsed());
        if remaining.is_zero() {
            line["status"] = json!("skipped");
            line["error"] = json!("total deadline elapsed before this request");
            summary.skipped += 1;
        } else {
            let mut buffer = vec![0; report.length];
            buffer[0] = report.report_id;
            match transport.get_feature_report(&mut buffer, settings.timeout.min(remaining)) {
                Ok(received) if received <= buffer.len() => {
                    line["status"] = json!("ok");
                    line["received_length"] = json!(received);
                    line["raw_hex"] = json!(encode_hex(&buffer[..received]));
                    summary.ok += 1;
                }
                Ok(received) => {
                    line["status"] = json!("error");
                    line["error"] = json!(format!(
                        "transport reported {received} bytes for a {}-byte buffer",
                        buffer.len()
                    ));
                    summary.error += 1;
                }
                Err(ReadError::Failed(message)) => {
                    line["status"] = json!("error");
                    line["error"] = json!(message);
                    summary.error += 1;
                }
                Err(ReadError::NotSent(reason)) => {
                    line["status"] = json!("not_sent");
                    line["reason"] = json!(reason);
                    summary.not_sent += 1;
                }
            }
        }
        write_line(writer, &line)?;
    }
    write_line(writer, &summary_line(elapsed(), summary, None))?;
    Ok(summary)
}

fn stop(
    writer: &mut impl Write,
    header: &Value,
    elapsed: Duration,
    summary: ProbeSummary,
    message: String,
) -> Result<ProbeSummary, Box<dyn Error>> {
    write_line(writer, header)?;
    write_line(writer, &summary_line(elapsed, summary, Some(&message)))?;
    Err(message.into())
}

fn summary_line(elapsed: Duration, summary: ProbeSummary, error: Option<&str>) -> Value {
    let mut line = json!({
        "type": "features_summary",
        "requested": summary.ok + summary.error + summary.skipped + summary.not_sent,
        "ok": summary.ok,
        "error": summary.error,
        "skipped": summary.skipped,
        "not_sent": summary.not_sent,
        "elapsed_micros": u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX),
    });
    if let Some(error) = error {
        line["status"] = json!("aborted");
        line["error"] = json!(error);
    } else {
        line["status"] = json!("completed");
    }
    line
}

fn write_line(writer: &mut impl Write, value: &Value) -> Result<(), Box<dyn Error>> {
    serde_json::to_writer(&mut *writer, value)?;
    writeln!(writer)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use cooltek_sdk_masterhub::ReportTransport;

    use super::*;
    use crate::capture::decode_hex;

    // Report descriptors captured from hardware by `masterhub-diag record`
    // (first `report_descriptor.raw_hex` of each capture).
    const M032LE_DESCRIPTOR_HEX: &str = "050c0901a1010a00ff150026ff007508963f00850291020a00ff150026ff007508963f00850181020a00ff150026ff007508961f008503b1040a00ff150026ff007508961f00852db1040a00ff150026ff007508961f00852eb1040a00ff150026ff007508961f00852fb1040a00ff150026ff007508961f008530b1040a00ff150026ff007508961f008531b1040a00ff150026ff007508961f008536b104c0";
    const M003K_DESCRIPTOR_HEX: &str = "050c0901a1010a00ff150026ff007508963f00850291020a00ff150026ff007508963f00850181020a00ff150026ff007508961f008503b1040a00ff150026ff007508961f00852db1040a00ff150026ff007508961f00852eb1040a00ff150026ff007508961f00852fb1040a00ff150026ff007508961f008530b1040a00ff150026ff007508961f008531b1040a00ff150026ff007508961f008532b1040a00ff150026ff007508961f008534b1040a00ff150026ff007508961f008536b104c0";
    const M131E_DESCRIPTOR_HEX: &str = "050c0901a1010a00ff150026ff007508963f01850291020a00ff150026ff007508963f00850181020a00ff150026ff007508961f008503b1040a00ff150026ff007508961f008547b1040a00ff150026ff007508961f008548b1040a00ff150026ff007508961f008549b1040a00ff150026ff007508961f00854ab1040a00ff150026ff007508961f00854bb1040a00ff150026ff007508961f00854cb1040a00ff150026ff007508961f00854db1040a00ff150026ff007508961f00854eb104c0";

    const DEFAULT_TIMEOUT: Duration = Duration::from_millis(500);

    #[derive(Clone, Debug, PartialEq)]
    enum Call {
        Descriptor,
        GetFeature {
            report_id: u8,
            length: usize,
            timeout: Duration,
        },
        Forbidden(&'static str),
    }

    enum Reply {
        Bytes(Vec<u8>),
        Fail(&'static str),
        NotSent(&'static str),
        Overlong,
    }

    /// A fake that could also write (it implements [`ReportTransport`]); every
    /// write-capable method records a forbidden call that tests must not see.
    /// Each GET advances a virtual clock instead of sleeping.
    struct Fake {
        descriptor: Result<Vec<u8>, &'static str>,
        replies: HashMap<u8, Reply>,
        calls: RefCell<Vec<Call>>,
        clock: Cell<Instant>,
        advance_per_get: Duration,
    }

    impl Fake {
        fn new(descriptor_hex: &str) -> Self {
            Self {
                descriptor: Ok(decode_hex(descriptor_hex, 4096).unwrap()),
                replies: HashMap::new(),
                calls: RefCell::new(Vec::new()),
                clock: Cell::new(Instant::now()),
                advance_per_get: Duration::ZERO,
            }
        }

        fn calls(&self) -> Vec<Call> {
            self.calls.borrow().clone()
        }

        fn requested_ids(&self) -> Vec<u8> {
            self.calls()
                .iter()
                .filter_map(|call| match call {
                    Call::GetFeature { report_id, .. } => Some(*report_id),
                    _ => None,
                })
                .collect()
        }
    }

    fn get(report_id: u8) -> Call {
        Call::GetFeature {
            report_id,
            length: 32,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    impl ReadOnlyFeatureTransport for Fake {
        fn get_report_descriptor(
            &self,
            buffer: &mut [u8],
            timeout: Duration,
        ) -> Result<usize, ReadError> {
            assert!(!timeout.is_zero());
            self.calls.borrow_mut().push(Call::Descriptor);
            let bytes = self
                .descriptor
                .as_ref()
                .map_err(|message| ReadError::Failed((*message).into()))?;
            buffer[..bytes.len()].copy_from_slice(bytes);
            Ok(bytes.len())
        }

        fn get_feature_report(
            &self,
            buffer: &mut [u8],
            timeout: Duration,
        ) -> Result<usize, ReadError> {
            assert!(!timeout.is_zero());
            let report_id = buffer[0];
            self.calls.borrow_mut().push(Call::GetFeature {
                report_id,
                length: buffer.len(),
                timeout,
            });
            self.clock.set(self.clock.get() + self.advance_per_get);
            match self.replies.get(&report_id) {
                Some(Reply::Bytes(bytes)) => {
                    buffer[..bytes.len()].copy_from_slice(bytes);
                    Ok(bytes.len())
                }
                Some(Reply::Fail(message)) => Err(ReadError::Failed((*message).into())),
                Some(Reply::NotSent(reason)) => Err(ReadError::NotSent((*reason).into())),
                Some(Reply::Overlong) => Ok(buffer.len() + 1),
                None => {
                    buffer[1..].fill(0xaa);
                    Ok(buffer.len())
                }
            }
        }
    }

    impl ReportTransport for Fake {
        fn read_timeout(&self, _: &mut [u8], _: i32) -> HidResult<usize> {
            self.calls
                .borrow_mut()
                .push(Call::Forbidden("read_timeout"));
            Ok(0)
        }
        fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
            self.calls
                .borrow_mut()
                .push(Call::Forbidden("untimed get_feature_report"));
            Ok(0)
        }
        fn send_feature_report(&self, _: &[u8]) -> HidResult<()> {
            self.calls
                .borrow_mut()
                .push(Call::Forbidden("send_feature_report"));
            Ok(())
        }
        fn write_output(&self, data: &[u8]) -> HidResult<usize> {
            self.calls
                .borrow_mut()
                .push(Call::Forbidden("write_output"));
            Ok(data.len())
        }
    }

    fn identity(product_id: u16, module: &str) -> Identity {
        Identity {
            path_hex: "4465765372767349443a31".into(),
            vendor_id: 0x2516,
            product_id,
            serial_number: Some("SERIAL".into()),
            manufacturer_string: Some("COOLER MASTER".into()),
            product_string: None,
            release_number: 0,
            interface_number: 0,
            usage_page: 12,
            usage: 1,
            module: Some(module.into()),
        }
    }

    fn settings(only_report_id: Option<u8>) -> ProbeSettings {
        ProbeSettings {
            only_report_id,
            timeout: DEFAULT_TIMEOUT,
            duration: Duration::from_secs(10),
        }
    }

    fn parse_lines(output: Vec<u8>) -> Vec<Value> {
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn run_with(
        fake: &Fake,
        settings: ProbeSettings,
    ) -> (Result<ProbeSummary, String>, Vec<Value>) {
        let mut output = Vec::new();
        let result = probe_with_clock(
            fake,
            &identity(516, "M032LE"),
            settings,
            &mut output,
            &|| fake.clock.get(),
        )
        .map_err(|error| error.to_string());
        (result, parse_lines(output))
    }

    fn run(fake: &Fake, only: Option<u8>) -> (Result<ProbeSummary, String>, Vec<Value>) {
        run_with(fake, settings(only))
    }

    fn assert_no_forbidden_calls(fake: &Fake) {
        for call in fake.calls() {
            assert!(
                !matches!(call, Call::Forbidden(_)),
                "probe issued a forbidden call: {call:?}"
            );
        }
    }

    #[test]
    fn reads_every_declared_roller_feature_report_once_with_declared_lengths() {
        let fake = Fake::new(M032LE_DESCRIPTOR_HEX);
        let (result, lines) = run(&fake, None);
        let ids = [0x03, 0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x36];
        assert_eq!(
            result.unwrap(),
            ProbeSummary {
                ok: 7,
                ..ProbeSummary::default()
            }
        );
        let mut expected = vec![Call::Descriptor];
        expected.extend(ids.iter().map(|&report_id| get(report_id)));
        assert_eq!(fake.calls(), expected);

        let header = &lines[0];
        assert_eq!(header["type"], "features_session");
        assert_eq!(header["read_only"], true);
        assert_eq!(header["identity"]["product_id"], 516);
        assert_eq!(header["identity"]["module"], "M032LE");
        assert_eq!(header["report_descriptor"]["length"], 160);
        assert_eq!(
            header["report_descriptor"]["raw_hex"],
            M032LE_DESCRIPTOR_HEX
        );
        assert_eq!(header["declared_feature_ids"], json!(ids));
        assert_eq!(header["requested_report_ids"], json!(ids));
        assert_eq!(
            header["declared_feature_reports"][0],
            json!({"report_id": 3, "declared_bits": 248, "declared_length": 32})
        );
        assert_eq!(lines.len(), 1 + ids.len() + 1);
        for (line, id) in lines[1..=ids.len()].iter().zip(ids) {
            assert_eq!(line["type"], "feature");
            assert_eq!(line["report_id"], id);
            assert_eq!(line["declared_length"], 32);
            assert_eq!(line["status"], "ok");
            assert_eq!(line["received_length"], 32);
            assert_eq!(
                line["raw_hex"],
                format!("{id:02x}{}", "aa".repeat(31)).as_str()
            );
        }
        let summary = lines.last().unwrap();
        assert_eq!(summary["type"], "features_summary");
        assert_eq!(summary["status"], "completed");
        assert_eq!(
            summary,
            &json!({
                "type": "features_summary", "status": "completed", "requested": 7,
                "ok": 7, "error": 0, "skipped": 0, "not_sent": 0, "elapsed_micros": 0,
            })
        );
        assert_no_forbidden_calls(&fake);
    }

    #[test]
    fn undeclared_ids_are_never_requested_for_any_captured_descriptor() {
        for (hex, declared) in [
            (
                M032LE_DESCRIPTOR_HEX,
                vec![0x03, 0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x36],
            ),
            (
                M003K_DESCRIPTOR_HEX,
                vec![0x03, 0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x32, 0x34, 0x36],
            ),
            (
                M131E_DESCRIPTOR_HEX,
                vec![0x03, 0x47, 0x48, 0x49, 0x4a, 0x4b, 0x4c, 0x4d, 0x4e],
            ),
        ] {
            let fake = Fake::new(hex);
            let (result, _) = run(&fake, None);
            assert!(result.is_ok());
            assert_eq!(fake.requested_ids(), declared);
            // Input ID 1 and output ID 2 are never fetched as features.
            assert!(!fake.requested_ids().contains(&1));
            assert!(!fake.requested_ids().contains(&2));
            assert_no_forbidden_calls(&fake);

            // An undeclared --report-id (here the output ID) sends nothing.
            let fake = Fake::new(hex);
            let (result, lines) = run(&fake, Some(2));
            assert!(
                result
                    .unwrap_err()
                    .contains("not declared as a Feature report")
            );
            assert_eq!(fake.calls(), [Call::Descriptor]);
            assert_eq!(lines.len(), 2);
            assert_eq!(lines[0]["requested_report_ids"], json!([]));
            assert_eq!(lines[1]["status"], "aborted");
        }
    }

    #[test]
    fn report_id_option_selects_exactly_one_declared_report() {
        let fake = Fake::new(M003K_DESCRIPTOR_HEX);
        let (result, lines) = run(&fake, Some(0x32));
        assert_eq!(result.unwrap().ok, 1);
        assert_eq!(fake.calls(), [Call::Descriptor, get(0x32)]);
        assert_eq!(
            lines[0]["declared_feature_ids"].as_array().unwrap().len(),
            9
        );
        assert_eq!(lines[0]["requested_report_ids"], json!([0x32]));
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn failing_and_short_gets_are_recorded_without_stopping_later_reports() {
        let mut fake = Fake::new(M032LE_DESCRIPTOR_HEX);
        fake.replies.insert(0x03, Reply::Fail("device stalled"));
        fake.replies
            .insert(0x2d, Reply::Bytes(vec![0x2d, 0x01, 0x02]));
        fake.replies.insert(0x2e, Reply::Overlong);
        fake.replies.insert(0x2f, Reply::Bytes(Vec::new()));
        let (result, lines) = run(&fake, None);
        assert_eq!(
            result.unwrap(),
            ProbeSummary {
                ok: 5,
                error: 2,
                ..ProbeSummary::default()
            }
        );
        assert_eq!(fake.requested_ids().len(), 7);

        assert_eq!(lines[1]["status"], "error");
        assert_eq!(lines[1]["report_id"], 3);
        assert_eq!(lines[1]["declared_length"], 32);
        assert!(
            lines[1]["error"]
                .as_str()
                .unwrap()
                .contains("device stalled")
        );
        assert!(lines[1].get("raw_hex").is_none());

        // Fewer bytes than declared: only the bytes actually received.
        assert_eq!(lines[2]["status"], "ok");
        assert_eq!(lines[2]["declared_length"], 32);
        assert_eq!(lines[2]["received_length"], 3);
        assert_eq!(lines[2]["raw_hex"], "2d0102");

        assert_eq!(lines[3]["status"], "error");
        assert!(
            lines[3]["error"]
                .as_str()
                .unwrap()
                .contains("33 bytes for a 32-byte")
        );

        assert_eq!(lines[4]["received_length"], 0);
        assert_eq!(lines[4]["raw_hex"], "");

        let summary = lines.last().unwrap();
        assert_eq!(summary["error"], 2);
        assert_eq!(summary["ok"], 5);
        assert_eq!(summary["not_sent"], 0);
        assert_no_forbidden_calls(&fake);
    }

    #[test]
    fn not_sent_reports_are_counted_apart_from_errors() {
        let mut fake = Fake::new(M032LE_DESCRIPTOR_HEX);
        for id in [0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x36] {
            fake.replies
                .insert(id, Reply::NotSent("earlier request pending"));
        }
        let (result, lines) = run(&fake, None);
        assert_eq!(
            result.unwrap(),
            ProbeSummary {
                ok: 1,
                not_sent: 6,
                ..ProbeSummary::default()
            }
        );
        for line in &lines[2..8] {
            assert_eq!(line["status"], "not_sent");
            assert_eq!(line["reason"], "earlier request pending");
            assert!(line.get("error").is_none());
        }
        let summary = lines.last().unwrap();
        assert_eq!(
            (
                &summary["requested"],
                &summary["error"],
                &summary["not_sent"]
            ),
            (&json!(7), &json!(0), &json!(6))
        );
    }

    #[test]
    fn descriptor_failures_end_before_any_get_feature() {
        let mut fake = Fake::new(M032LE_DESCRIPTOR_HEX);
        fake.descriptor = Err("not permitted");
        let (result, lines) = run(&fake, None);
        assert!(result.unwrap_err().contains("not permitted"));
        assert_eq!(fake.calls(), [Call::Descriptor]);
        assert_eq!(lines[0]["report_descriptor"]["status"], "error");
        assert_eq!(lines[1]["type"], "features_summary");
        assert_eq!(lines[1]["status"], "aborted");

        let mut truncated = decode_hex(M032LE_DESCRIPTOR_HEX, 4096).unwrap();
        truncated.pop();
        let mut fake = Fake::new(M032LE_DESCRIPTOR_HEX);
        fake.descriptor = Ok(truncated);
        let (result, lines) = run(&fake, None);
        assert!(
            result
                .unwrap_err()
                .contains("Invalid HID report descriptor")
        );
        assert_eq!(fake.calls(), [Call::Descriptor]);
        assert_eq!(lines[0]["report_descriptor"]["length"], 159);
        assert!(lines[0]["report_descriptor"]["parse_error"].is_string());

        let mut fake = Fake::new(M032LE_DESCRIPTOR_HEX);
        fake.descriptor = Ok(Vec::new());
        let (result, _) = run(&fake, None);
        assert!(result.unwrap_err().contains("empty report descriptor"));
        assert_eq!(fake.calls(), [Call::Descriptor]);
        assert_no_forbidden_calls(&fake);
    }

    #[test]
    fn total_deadline_skips_remaining_reports_and_bounds_each_wait() {
        // Virtual time: each GET takes 30 ms of a 50 ms budget; no real sleep.
        let mut fake = Fake::new(M032LE_DESCRIPTOR_HEX);
        fake.advance_per_get = Duration::from_millis(30);
        let (result, lines) = run_with(
            &fake,
            ProbeSettings {
                only_report_id: None,
                timeout: DEFAULT_TIMEOUT,
                duration: Duration::from_millis(50),
            },
        );
        assert_eq!(
            result.unwrap(),
            ProbeSummary {
                ok: 2,
                skipped: 5,
                ..ProbeSummary::default()
            }
        );
        // Each wait is capped by the remaining budget, not the 500 ms timeout.
        assert_eq!(
            fake.calls(),
            [
                Call::Descriptor,
                Call::GetFeature {
                    report_id: 0x03,
                    length: 32,
                    timeout: Duration::from_millis(50),
                },
                Call::GetFeature {
                    report_id: 0x2d,
                    length: 32,
                    timeout: Duration::from_millis(20),
                },
            ]
        );
        assert_eq!(lines[3]["report_id"], 0x2e);
        for line in &lines[3..8] {
            assert_eq!(line["status"], "skipped");
            assert_eq!(line["error"], "total deadline elapsed before this request");
        }
        let summary = lines.last().unwrap();
        assert_eq!(
            summary,
            &json!({
                "type": "features_summary", "status": "completed", "requested": 7,
                "ok": 2, "error": 0, "skipped": 5, "not_sent": 0, "elapsed_micros": 60_000,
            })
        );
        assert_no_forbidden_calls(&fake);
    }

    #[test]
    fn stalled_reader_reports_later_requests_as_not_sent_without_sending_them() {
        // Exercise the timeout and stall logic without HID: no worker replies.
        let (requests, received) = mpsc::channel();
        let (_replies_sender, replies) = mpsc::channel();
        let reader = HidFeatureReader {
            requests,
            replies,
            stalled: Cell::new(false),
        };
        let mut buffer = [0x03, 0, 0];
        let error = reader
            .get_feature_report(&mut buffer, Duration::from_millis(5))
            .unwrap_err();
        assert_eq!(error, ReadError::Failed("no reply within 5 ms".into()));
        let error = reader
            .get_feature_report(&mut buffer, Duration::from_millis(5))
            .unwrap_err();
        assert!(matches!(&error, ReadError::NotSent(reason) if reason.contains("still pending")));
        let sent: Vec<Request> = received.try_iter().collect();
        assert_eq!(sent.len(), 1);
        assert!(matches!(
            sent[0],
            Request::Feature {
                report_id: 3,
                length: 3
            }
        ));
    }

    /// Report ID and buffer length of each GET the worker source received.
    type SourceLog = Arc<Mutex<Vec<(u8, usize)>>>;

    /// A worker source: logs each buffer it is handed and replies from a table.
    /// Report 0x03 can be made to block until `gate` is released.
    struct FakeSource {
        descriptor: Vec<u8>,
        log: SourceLog,
        overlong_id: Option<u8>,
        gate: Option<Mutex<mpsc::Receiver<()>>>,
    }

    impl FeatureSource for FakeSource {
        fn descriptor(&self, buffer: &mut [u8]) -> HidResult<usize> {
            buffer[..self.descriptor.len()].copy_from_slice(&self.descriptor);
            Ok(self.descriptor.len())
        }
        fn get_feature(&self, buffer: &mut [u8]) -> HidResult<usize> {
            self.log.lock().unwrap().push((buffer[0], buffer.len()));
            if buffer[0] == 0x03
                && let Some(gate) = &self.gate
            {
                // Blocks until the test drops the sender.
                let _ = gate.lock().unwrap().recv();
            }
            if Some(buffer[0]) == self.overlong_id {
                return Ok(buffer.len() + 1);
            }
            buffer[1] = 0x5a;
            Ok(2)
        }
    }

    fn source(gate: Option<mpsc::Receiver<()>>) -> (FakeSource, SourceLog) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let source = FakeSource {
            descriptor: decode_hex(M032LE_DESCRIPTOR_HEX, 4096).unwrap(),
            log: Arc::clone(&log),
            overlong_id: Some(0x36),
            gate: gate.map(Mutex::new),
        };
        (source, log)
    }

    #[test]
    fn worker_passes_report_id_and_declared_length_and_guards_overlong_counts() {
        let (source, log) = source(None);
        let reader = HidFeatureReader::spawn_source(source);
        let mut output = Vec::new();
        let summary = probe(
            &reader,
            &identity(516, "M032LE"),
            settings(None),
            &mut output,
        )
        .unwrap();
        assert_eq!(
            summary,
            ProbeSummary {
                ok: 6,
                error: 1,
                ..ProbeSummary::default()
            }
        );
        // buffer[0] is the report ID; length is the declared 31 bytes + ID.
        let ids = [0x03, 0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x36];
        assert_eq!(
            *log.lock().unwrap(),
            ids.iter().map(|&id| (id, 32)).collect::<Vec<_>>()
        );
        let lines = parse_lines(output);
        assert_eq!(lines[0]["report_descriptor"]["length"], 160);
        assert_eq!(lines[1]["raw_hex"], "035a");
        assert_eq!(lines[1]["received_length"], 2);
        assert_eq!(lines[7]["report_id"], 0x36);
        assert_eq!(lines[7]["status"], "error");
        assert!(
            lines[7]["error"]
                .as_str()
                .unwrap()
                .contains("33 bytes for a 32-byte buffer")
        );
    }

    #[test]
    fn worker_stall_on_first_report_leaves_the_rest_not_sent() {
        let (release, gate) = mpsc::channel();
        let (source, log) = source(Some(gate));
        let reader = HidFeatureReader::spawn_source(source);
        let mut output = Vec::new();
        let summary = probe(
            &reader,
            &identity(516, "M032LE"),
            ProbeSettings {
                only_report_id: None,
                timeout: Duration::from_millis(20),
                duration: Duration::from_secs(10),
            },
            &mut output,
        )
        .unwrap();
        // The stalled request itself was sent and timed out; nothing after it
        // reached the worker.
        assert_eq!(
            summary,
            ProbeSummary {
                error: 1,
                not_sent: 6,
                ..ProbeSummary::default()
            }
        );
        let lines = parse_lines(output);
        assert_eq!(lines[1]["status"], "error");
        assert_eq!(lines[1]["error"], "no reply within 20 ms");
        for line in &lines[2..8] {
            assert_eq!(line["status"], "not_sent");
            assert!(line["reason"].as_str().unwrap().contains("still pending"));
        }
        assert_eq!(lines[8]["not_sent"], 6);
        assert_eq!(lines[8]["error"], 1);
        drop(release);
        assert_eq!(*log.lock().unwrap(), [(0x03, 32)]);
    }
}
