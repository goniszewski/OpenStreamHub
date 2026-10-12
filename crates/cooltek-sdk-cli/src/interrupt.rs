//! Ctrl-C/SIGTERM handling shared by recording and live sessions.
//!
//! The first signal sets a cooperative flag so the caller can write its footer
//! and restore device mode. A second signal exits immediately with status 130,
//! skipping any cleanup that is still pending. Signals are counted separately
//! from the flag, so a caller that sets the flag for internal cancellation does
//! not turn the next real signal into a forced exit.
//!
//! `masterhub-service run --exit-on-stdin-eof` adds a stdin watchdog: when the
//! supervising parent dies, the pipe it holds open on our stdin closes, and the
//! watchdog sets the same cooperative flag a first signal sets, so the session
//! ends through the same cleanup (write gate closed, hardware mode restored).

use std::{
    io::{self, Read},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};

/// Set by the stdin watchdog in `masterhub-service` just before it requests a
/// stop, so the session footer can say `stdin_closed` instead of `interrupted`.
pub static STDIN_CLOSED: AtomicBool = AtomicBool::new(false);

/// Exit status used when a repeated interrupt forces termination.
pub const FORCED_EXIT_CODE: i32 = 130;

/// Installs the process-wide handler and returns its cooperative stop flag.
pub fn install() -> Result<Arc<AtomicBool>, String> {
    let stopped = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&stopped);
    let signals = AtomicUsize::new(0);
    ctrlc::set_handler(move || {
        if signal_received(&signals, &signal) {
            eprintln!("second interrupt received; exiting without cleanup");
            std::process::exit(FORCED_EXIT_CODE);
        }
    })
    .map_err(|error| format!("could not install interrupt handler: {error}"))?;
    Ok(stopped)
}

/// Records one signal and sets the stop flag; returns true when a previous
/// signal (not merely a set flag) was already received.
pub fn signal_received(signals: &AtomicUsize, flag: &AtomicBool) -> bool {
    flag.store(true, Ordering::SeqCst);
    signals.fetch_add(1, Ordering::SeqCst) > 0
}

#[cfg(unix)]
const STDIN_ALREADY_CLOSED: &str = "--exit-on-stdin-eof: stdin is already closed; the supervising process must keep the stdin pipe open for the whole run";

/// Refuses a stdin that cannot signal a parent's death: anything but a pipe or
/// socket (`/dev/null`, a regular file, a terminal). Run before the OBS
/// password is read, so a terminal never shows a password prompt that is
/// refused afterwards, and before any device is opened. Nothing is read.
#[cfg(unix)]
pub fn check_stdin_pipe(stdin: impl std::os::fd::AsFd) -> Result<(), String> {
    use std::os::unix::fs::FileTypeExt;
    let file_type = stdin
        .as_fd()
        .try_clone_to_owned()
        .map(std::fs::File::from)
        .and_then(|file| file.metadata())
        .map_err(|_| STDIN_ALREADY_CLOSED.to_owned())?
        .file_type();
    if !file_type.is_fifo() && !file_type.is_socket() {
        return Err("--exit-on-stdin-eof requires stdin to be a pipe or socket held open by the supervising process, not a terminal, file or /dev/null".into());
    }
    Ok(())
}

/// Refuses a stdin pipe whose writer has already closed (after
/// [`check_stdin_pipe`]), even when unread bytes remain: the run would stop
/// as soon as they were drained. Run after the OBS password line and before
/// any device is opened, so `--exit-on-stdin-eof` never starts a session only
/// to stop it at once. Only the kernel descriptor is inspected; nothing is
/// read, so bytes still buffered after the password line are not consumed.
#[cfg(unix)]
pub fn check_stdin_open(stdin: impl std::os::fd::AsFd) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    const CLOSED: &str = STDIN_ALREADY_CLOSED;
    let fd = stdin.as_fd();
    let mut descriptor = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd, zero timeout.
    let ready = unsafe { libc::poll(&mut descriptor, 1, 0) };
    if ready < 0 {
        return Err(format!(
            "--exit-on-stdin-eof: could not check stdin: {}",
            io::Error::last_os_error()
        ));
    }
    if descriptor.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
        return Err(CLOSED.into());
    }
    // Readable with nothing to read is end of file (some systems report a
    // closed pipe that way instead of POLLHUP).
    if descriptor.revents & libc::POLLIN != 0 {
        let mut pending: libc::c_int = 0;
        // SAFETY: FIONREAD writes one c_int.
        if unsafe { libc::ioctl(fd.as_raw_fd(), libc::FIONREAD, &mut pending) } == 0 && pending == 0
        {
            return Err(CLOSED.into());
        }
    }
    Ok(())
}

/// Live runs need Unix for bounded stdout anyway; refuse the flag elsewhere.
#[cfg(not(unix))]
pub fn check_stdin_pipe<T>(_stdin: T) -> Result<(), String> {
    Err("--exit-on-stdin-eof is supported only on Unix".into())
}

/// See the Unix version; unreachable after [`check_stdin_pipe`] refused.
#[cfg(not(unix))]
pub fn check_stdin_open<T>(_stdin: T) -> Result<(), String> {
    Err("--exit-on-stdin-eof is supported only on Unix".into())
}

/// What the stdin watchdog reads: on Unix it also exposes its descriptor, so a
/// non-blocking stdin can be waited on with `poll` instead of read as closed.
#[cfg(unix)]
pub trait WatchedStdin: Read + std::os::fd::AsFd + Send + 'static {}
#[cfg(unix)]
impl<T: Read + std::os::fd::AsFd + Send + 'static> WatchedStdin for T {}
/// What the stdin watchdog reads.
#[cfg(not(unix))]
pub trait WatchedStdin: Read + Send + 'static {}
#[cfg(not(unix))]
impl<T: Read + Send + 'static> WatchedStdin for T {}

/// Starts the stdin watchdog thread: it reads and discards `stdin` until end of
/// file or a read error, then calls [`stdin_closed`]. Data never stops the run,
/// and neither does `EINTR` or `EWOULDBLOCK` (a non-blocking stdin shared with
/// the supervisor): it then waits in `poll` until stdin is readable or hung up.
/// The thread is detached; it ends with the process.
pub fn watch_stdin(
    stdin: impl WatchedStdin,
    interrupted: Arc<AtomicBool>,
    closed: &'static AtomicBool,
) -> Result<thread::JoinHandle<()>, String> {
    watch_stdin_impl(stdin, interrupted, closed, None)
}

/// The longest control line the watchdog keeps; a longer one is dropped whole.
pub const MAX_STDIN_LINE: usize = 64 * 1024;

/// [`watch_stdin`] that also hands each complete line of stdin to `on_line`
/// (without its newline), on a separate handler thread. The watchdog never
/// waits for a handler, so a blocked HID call cannot hide parent death.
/// Lines are bounded by [`MAX_STDIN_LINE`]; invalid UTF-8 is dropped. A full
/// 16-line handler queue stops the run rather than blocking EOF detection.
pub fn watch_stdin_lines(
    stdin: impl WatchedStdin,
    interrupted: Arc<AtomicBool>,
    closed: &'static AtomicBool,
    mut on_line: impl FnMut(&str) + Send + 'static,
) -> Result<thread::JoinHandle<()>, String> {
    let (sender, receiver) = std::sync::mpsc::sync_channel::<String>(16);
    let handler_stop = Arc::clone(&interrupted);
    thread::Builder::new()
        .name("stdin-control".into())
        .spawn(move || {
            while let Ok(line) = receiver.recv() {
                if handler_stop.load(Ordering::SeqCst) {
                    break;
                }
                on_line(&line);
            }
        })
        .map_err(|error| format!("could not start the stdin control handler: {error}"))?;
    watch_stdin_impl(stdin, interrupted, closed, Some(sender))
}

fn watch_stdin_impl(
    stdin: impl WatchedStdin,
    interrupted: Arc<AtomicBool>,
    closed: &'static AtomicBool,
    sender: Option<std::sync::mpsc::SyncSender<String>>,
) -> Result<thread::JoinHandle<()>, String> {
    thread::Builder::new()
        .name("stdin-watchdog".into())
        .spawn(move || {
            let mut stdin = stdin;
            let mut chunk = [0u8; 1024];
            let mut line: Vec<u8> = Vec::new();
            let mut overflowed = false;
            loop {
                match stdin.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(count) => {
                        for &byte in &chunk[..count] {
                            if byte == b'\n' {
                                if !overflowed
                                    && let Some(sender) = &sender
                                    && let Ok(text) = std::str::from_utf8(&line)
                                    && sender.try_send(text.to_owned()).is_err()
                                {
                                    // The last stderr line becomes the reason the app shows for the exit.
                                    eprintln!(
                                        "masterhub-service: control requests arrived faster than the module could take them; stopping the run"
                                    );
                                    interrupted.store(true, Ordering::SeqCst);
                                    return;
                                }
                                line.clear();
                                overflowed = false;
                            } else if !overflowed {
                                if line.len() >= MAX_STDIN_LINE {
                                    overflowed = true;
                                    line.clear();
                                } else {
                                    line.push(byte);
                                }
                            }
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        if !wait_readable(&stdin) {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            stdin_closed(&interrupted, closed);
        })
        .map_err(|error| format!("could not start the stdin watchdog: {error}"))
}

/// Blocks until a non-blocking stdin is worth reading again. False when it is
/// hung up or failed with nothing left to read, or `poll` itself failed.
#[cfg(unix)]
fn wait_readable(stdin: &impl std::os::fd::AsFd) -> bool {
    use std::os::fd::AsRawFd;
    let fd = stdin.as_fd().as_raw_fd();
    let mut descriptor = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd; no timeout.
    if unsafe { libc::poll(&mut descriptor, 1, -1) } < 0 {
        return io::Error::last_os_error().kind() == io::ErrorKind::Interrupted;
    }
    if descriptor.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
        let mut pending: libc::c_int = 0;
        // SAFETY: FIONREAD writes one c_int.
        let queried = unsafe { libc::ioctl(fd, libc::FIONREAD, &mut pending) } == 0;
        return queried && pending > 0;
    }
    true
}

/// Without `poll`, retry a non-blocking stdin after a short pause.
#[cfg(not(unix))]
fn wait_readable<T>(_stdin: &T) -> bool {
    thread::sleep(std::time::Duration::from_millis(10));
    true
}

/// Requests the same cooperative stop as a first signal, and marks `closed`
/// so the footer reports `stdin_closed`, but only when this call is what set
/// the stop flag: a signal or an internal cancellation that set it first, even
/// at the same moment, keeps its own reason. Never counts as a signal.
pub fn stdin_closed(interrupted: &AtomicBool, closed: &AtomicBool) {
    if interrupted
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        closed.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        time::{Duration, Instant},
    };

    fn leaked() -> &'static AtomicBool {
        Box::leak(Box::new(AtomicBool::new(false)))
    }

    fn wait_for(flag: &AtomicBool, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if flag.load(Ordering::SeqCst) {
                return true;
            }
            thread::sleep(Duration::from_millis(2));
        }
        flag.load(Ordering::SeqCst)
    }

    #[test]
    fn stdin_data_never_stops_and_eof_requests_the_signal_stop() {
        let (reader, mut writer) = io::pipe().unwrap();
        let interrupted = Arc::new(AtomicBool::new(false));
        let closed = leaked();
        let handle = watch_stdin(reader, Arc::clone(&interrupted), closed).unwrap();
        writer.write_all(b"noise\nmore noise").unwrap();
        writer.write_all(&[0; 4096]).unwrap();
        thread::sleep(Duration::from_millis(100));
        assert!(!interrupted.load(Ordering::SeqCst));
        assert!(!closed.load(Ordering::SeqCst));
        assert!(!handle.is_finished());
        let dropped = Instant::now();
        drop(writer);
        assert!(wait_for(&interrupted, Duration::from_secs(1)));
        assert!(dropped.elapsed() < Duration::from_secs(1));
        assert!(closed.load(Ordering::SeqCst));
        handle.join().unwrap();
        // It sets only the cooperative flag: the next real signal is still the
        // first one, not a forced exit.
        let signals = AtomicUsize::new(0);
        assert!(!signal_received(&signals, &interrupted));
    }

    #[test]
    fn a_stop_requested_first_keeps_its_own_reason() {
        let interrupted = AtomicBool::new(true);
        let closed = AtomicBool::new(false);
        stdin_closed(&interrupted, &closed);
        assert!(interrupted.load(Ordering::SeqCst));
        assert!(!closed.load(Ordering::SeqCst));
        // The same through the watchdog: EOF after a signal changes nothing.
        let (reader, writer) = io::pipe().unwrap();
        drop(writer);
        let interrupted = Arc::new(AtomicBool::new(false));
        let signals = AtomicUsize::new(0);
        assert!(!signal_received(&signals, &interrupted));
        let closed = leaked();
        watch_stdin(reader, Arc::clone(&interrupted), closed)
            .unwrap()
            .join()
            .unwrap();
        assert!(interrupted.load(Ordering::SeqCst));
        assert!(!closed.load(Ordering::SeqCst));
        // Not set first: this call sets both.
        let interrupted = AtomicBool::new(false);
        let closed = AtomicBool::new(false);
        stdin_closed(&interrupted, &closed);
        assert!(interrupted.load(Ordering::SeqCst) && closed.load(Ordering::SeqCst));
    }

    #[cfg(unix)]
    #[test]
    fn a_non_blocking_empty_stdin_waits_instead_of_reading_as_closed() {
        use std::os::fd::AsRawFd;
        let (reader, writer) = io::pipe().unwrap();
        let fd = reader.as_raw_fd();
        // SAFETY: plain flag changes on a descriptor this test owns.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            assert!(flags >= 0);
            assert_eq!(libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK), 0);
        }
        let interrupted = Arc::new(AtomicBool::new(false));
        let closed = leaked();
        let handle = watch_stdin(reader, Arc::clone(&interrupted), closed).unwrap();
        thread::sleep(Duration::from_millis(200));
        assert!(!interrupted.load(Ordering::SeqCst));
        assert!(!handle.is_finished());
        let dropped = Instant::now();
        drop(writer);
        assert!(wait_for(&interrupted, Duration::from_secs(1)));
        assert!(dropped.elapsed() < Duration::from_secs(1));
        assert!(closed.load(Ordering::SeqCst));
        handle.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn only_an_open_pipe_passes_the_start_check() {
        let (reader, mut writer) = io::pipe().unwrap();
        assert_eq!(check_stdin_pipe(&reader), Ok(()));
        assert_eq!(check_stdin_open(&reader), Ok(()));
        writer.write_all(b"buffered\n").unwrap();
        assert_eq!(check_stdin_open(&reader), Ok(()));
        // Intended: a pipe whose writer is gone is refused even while it still
        // holds unread bytes, since the run would hit EOF right after them.
        drop(writer);
        let error = check_stdin_open(&reader).unwrap_err();
        assert!(error.contains("already closed"), "{error}");
        // Still a pipe, so only the second check refuses it.
        assert_eq!(check_stdin_pipe(&reader), Ok(()));
        let (empty, writer) = io::pipe().unwrap();
        drop(writer);
        assert!(
            check_stdin_open(&empty)
                .unwrap_err()
                .contains("already closed")
        );
        let null = std::fs::File::open("/dev/null").unwrap();
        assert!(
            check_stdin_pipe(&null)
                .unwrap_err()
                .contains("requires stdin to be a pipe")
        );
        let file = std::fs::File::open(env!("CARGO_MANIFEST_PATH")).unwrap();
        assert!(
            check_stdin_pipe(&file)
                .unwrap_err()
                .contains("requires stdin to be a pipe")
        );
    }

    #[test]
    fn first_signal_requests_cleanup_and_second_forces_exit() {
        let signals = AtomicUsize::new(0);
        let flag = AtomicBool::new(false);
        assert!(!signal_received(&signals, &flag));
        assert!(flag.load(Ordering::SeqCst));
        assert!(signal_received(&signals, &flag));
    }

    #[test]
    fn internal_cancellation_does_not_count_as_a_signal() {
        let signals = AtomicUsize::new(0);
        // A live session sets the shared flag for drain/worker cancellation.
        let flag = AtomicBool::new(true);
        assert!(!signal_received(&signals, &flag));
        assert!(signal_received(&signals, &flag));
    }
}

#[cfg(all(test, unix))]
mod control_regressions {
    use super::*;
    use std::{io::Write, os::unix::net::UnixStream, sync::mpsc, time::Duration};
    #[test]
    fn parent_death_is_seen_during_a_blocked_control_handler() {
        let (reader, mut parent) = UnixStream::pair().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let closed = Box::leak(Box::new(AtomicBool::new(false)));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let handle = watch_stdin_lines(reader, stopped.clone(), closed, move |_| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap(); // stands in for an outstanding HID operation
        })
        .unwrap();
        parent
            .write_all(b"{\"id\":1,\"type\":\"led_state\"}\n")
            .unwrap();
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        drop(parent);
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !stopped.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        let observed = stopped.load(Ordering::SeqCst);
        release_tx.send(()).unwrap();
        handle.join().unwrap();
        assert!(
            observed,
            "parent death was not seen until the HID handler returned"
        );
    }
    #[test]
    fn a_full_handler_queue_stops_without_waiting_for_hardware() {
        let (reader, mut parent) = UnixStream::pair().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let closed = Box::leak(Box::new(AtomicBool::new(false)));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let handle = watch_stdin_lines(reader, stopped.clone(), closed, move |_| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        })
        .unwrap();
        parent.write_all(b"first\n").unwrap();
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        parent.write_all("queued\n".repeat(17).as_bytes()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !stopped.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        let observed = stopped.load(Ordering::SeqCst);
        release_tx.send(()).unwrap();
        drop(parent);
        handle.join().unwrap();
        assert!(observed, "a full queue must not block the watchdog");
    }
}
