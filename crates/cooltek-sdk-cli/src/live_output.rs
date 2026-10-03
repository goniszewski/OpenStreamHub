//! Deadline-limited stdout writes for the live service's JSON records.
//!
//! One record consists of every [`Write::write`] call up to [`Write::flush`].
//! The first nonempty write starts its deadline; subsequent writes do not extend
//! it. A failed record permanently disables the writer, because a partial JSON
//! record cannot safely be retried. No background thread or stdout lock is used.

use std::io::{self, Write};
use std::time::Duration;

#[cfg(unix)]
const MAX_TIMEOUT: Duration = Duration::from_millis(500);
#[cfg(unix)]
const MAX_RECORD_BYTES: usize = 65_536;

/// A live stdout writer with a total deadline and size limit for each record.
///
/// On Unix, this owns a close-on-exec duplicate of stdout and sets `O_NONBLOCK`.
/// Duplicated descriptors share file status flags, so stdout must have one owner
/// during this writer's lifetime. Dropping the writer restores the original
/// status flags before closing the duplicate.
///
/// Only pipes, sockets, and terminals are accepted. Regular-file writes can
/// block in the kernel despite `O_NONBLOCK`; use an actively consumed pipe,
/// such as `tee`, when recording live output to a file.
/// On other platforms construction returns an unsupported-platform error.
pub struct TimedStdout {
    #[cfg(unix)]
    writer: unix::RecordWriter<unix::FdSink>,
}

impl TimedStdout {
    /// Duplicate stdout and configure a record timeout of at most 500 ms.
    ///
    /// Zero timeouts are rejected. Each record may contain at most 65,536 bytes,
    /// including its newline. `flush` completes the record without calling
    /// `fsync`; writes go directly to the descriptor.
    pub fn new(timeout: Duration) -> Result<Self, String> {
        #[cfg(unix)]
        {
            unix::validate_timeout(timeout).map_err(|error| error.to_string())?;
            let sink = unix::FdSink::duplicate(libc::STDOUT_FILENO)
                .map_err(|error| format!("cannot configure bounded stdout: {error}"))?;
            Ok(Self {
                writer: unix::RecordWriter::new(sink, timeout),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = timeout;
            Err("bounded live stdout logging is supported only on Unix".into())
        }
    }
}

impl Write for TimedStdout {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        #[cfg(unix)]
        {
            self.writer.write(buffer)
        }
        #[cfg(not(unix))]
        {
            let _ = buffer;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "bounded live stdout logging is supported only on Unix",
            ))
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        {
            self.writer.flush()
        }
        #[cfg(not(unix))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "bounded live stdout logging is supported only on Unix",
            ))
        }
    }
}

#[cfg(unix)]
mod unix {
    use super::{MAX_RECORD_BYTES, MAX_TIMEOUT};
    use std::fs::File;
    use std::io::{self, Write};
    use std::os::fd::{AsRawFd, FromRawFd, RawFd};
    use std::os::unix::fs::FileTypeExt;
    use std::time::{Duration, Instant};

    pub(super) fn validate_timeout(timeout: Duration) -> io::Result<()> {
        if timeout.is_zero() || timeout > MAX_TIMEOUT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "stdout record timeout must be greater than zero and at most 500 ms",
            ));
        }
        Ok(())
    }

    pub(super) trait Sink: Send {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize>;
        fn wait_writable(&mut self, deadline: Instant) -> io::Result<()>;

        fn now(&self) -> Instant {
            Instant::now()
        }
    }

    pub(super) struct FdSink {
        file: File,
        original_flags: libc::c_int,
    }

    impl FdSink {
        pub(super) fn duplicate(fd: RawFd) -> io::Result<Self> {
            // F_DUPFD_CLOEXEC avoids leaking this descriptor into action children.
            let duplicated = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
            if duplicated < 0 {
                return Err(io::Error::last_os_error());
            }
            // The duplicate is owned here and is closed on every failure path.
            let file = unsafe { File::from_raw_fd(duplicated) };
            let file_type = file.metadata()?.file_type();
            if !file_type.is_fifo()
                && !file_type.is_socket()
                && unsafe { libc::isatty(duplicated) } != 1
            {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "live output deadlines require a terminal or pipe; use an actively read pipe (for example tee) for file capture",
                ));
            }
            let original_flags = unsafe { libc::fcntl(duplicated, libc::F_GETFL) };
            if original_flags < 0 {
                return Err(io::Error::last_os_error());
            }
            if unsafe { libc::fcntl(duplicated, libc::F_SETFL, original_flags | libc::O_NONBLOCK) }
                < 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                file,
                original_flags,
            })
        }
    }

    impl Sink for FdSink {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            let written =
                unsafe { libc::write(self.file.as_raw_fd(), buffer.as_ptr().cast(), buffer.len()) };
            if written < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(written as usize)
            }
        }

        fn wait_writable(&mut self, deadline: Instant) -> io::Result<()> {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(timeout_error());
            }
            // Round upward; RecordWriter checks the original deadline again
            // before writing, including after interrupted or rounded polls.
            let timeout_ms = remaining.as_millis().saturating_add(1).min(501) as libc::c_int;
            let mut descriptor = libc::pollfd {
                fd: self.file.as_raw_fd(),
                events: libc::POLLOUT,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
            if ready < 0 {
                return Err(io::Error::last_os_error());
            }
            if ready == 0 {
                return Err(timeout_error());
            }
            if descriptor.revents & libc::POLLNVAL != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "stdout descriptor is invalid",
                ));
            }
            if descriptor.revents & (libc::POLLERR | libc::POLLHUP) != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "stdout consumer disconnected",
                ));
            }
            Ok(())
        }
    }

    impl Drop for FdSink {
        fn drop(&mut self) {
            // Restore while our owned descriptor is still valid. Avoid an
            // unbounded retry loop during shutdown if signals keep arriving.
            for _ in 0..3 {
                if unsafe { libc::fcntl(self.file.as_raw_fd(), libc::F_SETFL, self.original_flags) }
                    >= 0
                    || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted
                {
                    break;
                }
            }
        }
    }

    fn timeout_error() -> io::Error {
        io::Error::new(io::ErrorKind::TimedOut, "stdout record deadline expired")
    }

    pub(super) struct RecordWriter<S> {
        sink: S,
        timeout: Duration,
        deadline: Option<Instant>,
        record_bytes: usize,
        failure: Option<(io::ErrorKind, String)>,
    }

    impl<S: Sink> RecordWriter<S> {
        pub(super) fn new(sink: S, timeout: Duration) -> Self {
            Self {
                sink,
                timeout,
                deadline: None,
                record_bytes: 0,
                failure: None,
            }
        }

        fn previous_failure(&self) -> io::Result<()> {
            if let Some((kind, message)) = &self.failure {
                return Err(io::Error::new(*kind, message.clone()));
            }
            Ok(())
        }

        fn fail<T>(&mut self, error: io::Error) -> io::Result<T> {
            self.failure = Some((error.kind(), error.to_string()));
            Err(error)
        }
    }

    impl<S: Sink> Write for RecordWriter<S> {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.previous_failure()?;
            if buffer.is_empty() {
                return Ok(0);
            }
            if buffer.len() > MAX_RECORD_BYTES - self.record_bytes {
                return self.fail(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "stdout record exceeds 65,536 bytes",
                ));
            }
            let deadline = *self
                .deadline
                .get_or_insert_with(|| self.sink.now() + self.timeout);
            loop {
                if self.sink.now() >= deadline {
                    return self.fail(timeout_error());
                }
                match self.sink.write(buffer) {
                    Ok(0) => {
                        return self.fail(io::Error::new(
                            io::ErrorKind::WriteZero,
                            "stdout accepted no bytes",
                        ));
                    }
                    Ok(written) if written <= buffer.len() => {
                        self.record_bytes += written;
                        return Ok(written);
                    }
                    Ok(_) => {
                        return self.fail(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "stdout accepted an invalid byte count",
                        ));
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        match self.sink.wait_writable(deadline) {
                            Ok(()) => {}
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                            Err(error) => return self.fail(error),
                        }
                    }
                    Err(error) => return self.fail(error),
                }
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            self.previous_failure()?;
            if self
                .deadline
                .is_some_and(|deadline| self.sink.now() >= deadline)
            {
                return self.fail(timeout_error());
            }
            self.deadline = None;
            self.record_bytes = 0;
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::collections::VecDeque;

        struct FakeSink {
            now: Instant,
            writes: VecDeque<Result<usize, io::ErrorKind>>,
            polls: VecDeque<Result<(), io::ErrorKind>>,
            output: Vec<u8>,
            deadlines: Vec<Instant>,
        }

        impl FakeSink {
            fn new() -> Self {
                Self {
                    now: Instant::now(),
                    writes: VecDeque::new(),
                    polls: VecDeque::new(),
                    output: Vec::new(),
                    deadlines: Vec::new(),
                }
            }
        }

        impl Sink for FakeSink {
            fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
                let written = self
                    .writes
                    .pop_front()
                    .unwrap_or(Ok(buffer.len()))
                    .map_err(io::Error::from)?;
                self.output.extend_from_slice(&buffer[..written]);
                Ok(written)
            }

            fn wait_writable(&mut self, deadline: Instant) -> io::Result<()> {
                self.deadlines.push(deadline);
                self.polls
                    .pop_front()
                    .unwrap_or(Ok(()))
                    .map_err(io::Error::from)
            }

            fn now(&self) -> Instant {
                self.now
            }
        }

        #[test]
        fn handles_partial_writes_and_interrupted_writes_and_polls() {
            let mut sink = FakeSink::new();
            sink.writes = VecDeque::from([
                Err(io::ErrorKind::Interrupted),
                Err(io::ErrorKind::WouldBlock),
                Err(io::ErrorKind::WouldBlock),
                Ok(2),
                Ok(2),
            ]);
            sink.polls = VecDeque::from([Err(io::ErrorKind::Interrupted), Ok(())]);
            let mut writer = RecordWriter::new(sink, Duration::from_millis(50));
            writer.write_all(b"test").unwrap();
            writer.flush().unwrap();
            assert_eq!(writer.sink.output, b"test");
            assert_eq!(writer.sink.deadlines.len(), 2);
            assert_eq!(writer.sink.deadlines[0], writer.sink.deadlines[1]);
        }

        #[test]
        fn deadline_covers_separate_writes_and_flush_without_extending() {
            let mut writer = RecordWriter::new(FakeSink::new(), Duration::from_millis(50));
            writer.write_all(b"first").unwrap();
            writer.sink.now += Duration::from_millis(49);
            writer.write_all(b"second").unwrap();
            writer.sink.now += Duration::from_millis(1);
            assert_eq!(writer.flush().unwrap_err().kind(), io::ErrorKind::TimedOut);
            assert_eq!(
                writer.write(b"third").unwrap_err().kind(),
                io::ErrorKind::TimedOut
            );
            assert_eq!(writer.sink.output, b"firstsecond");
        }

        #[test]
        fn expired_deadline_prevents_later_write() {
            let mut writer = RecordWriter::new(FakeSink::new(), Duration::from_millis(50));
            writer.write_all(b"first").unwrap();
            writer.sink.now += Duration::from_millis(50);
            assert_eq!(
                writer.write(b"second").unwrap_err().kind(),
                io::ErrorKind::TimedOut
            );
            assert_eq!(writer.sink.output, b"first");
        }

        #[test]
        fn successful_flush_resets_record_deadline_and_size_bound() {
            let mut writer = RecordWriter::new(FakeSink::new(), Duration::from_millis(50));
            writer.write_all(&vec![b'a'; MAX_RECORD_BYTES]).unwrap();
            writer.flush().unwrap();
            writer.sink.now += Duration::from_millis(100);
            writer.write_all(b"next").unwrap();
            writer.flush().unwrap();
            assert_eq!(writer.sink.output.len(), MAX_RECORD_BYTES + 4);
        }

        #[test]
        fn oversized_record_is_rejected_and_failure_is_sticky() {
            let mut writer = RecordWriter::new(FakeSink::new(), Duration::from_millis(50));
            writer.write_all(&vec![b'a'; MAX_RECORD_BYTES]).unwrap();
            assert_eq!(
                writer.write(b"\n").unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
            assert_eq!(
                writer.flush().unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
            assert_eq!(writer.sink.output.len(), MAX_RECORD_BYTES);
        }

        #[test]
        fn zero_write_fails_without_retry() {
            let mut sink = FakeSink::new();
            sink.writes.push_back(Ok(0));
            let mut writer = RecordWriter::new(sink, Duration::from_millis(50));
            assert_eq!(
                writer.write(b"record").unwrap_err().kind(),
                io::ErrorKind::WriteZero
            );
            assert_eq!(writer.flush().unwrap_err().kind(), io::ErrorKind::WriteZero);
        }

        fn pipe() -> (File, File) {
            let mut descriptors = [0; 2];
            assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
            unsafe {
                (
                    File::from_raw_fd(descriptors[0]),
                    File::from_raw_fd(descriptors[1]),
                )
            }
        }

        fn flags(file: &File) -> libc::c_int {
            let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
            assert!(result >= 0);
            result
        }

        #[test]
        fn owned_duplicate_restores_shared_flags_and_is_close_on_exec() {
            let (_reader, writer) = pipe();
            let original_flags = flags(&writer);
            {
                let sink = FdSink::duplicate(writer.as_raw_fd()).unwrap();
                assert_eq!(flags(&writer), original_flags | libc::O_NONBLOCK);
                let descriptor_flags = unsafe { libc::fcntl(sink.file.as_raw_fd(), libc::F_GETFD) };
                assert_ne!(descriptor_flags & libc::FD_CLOEXEC, 0);
            }
            assert_eq!(flags(&writer), original_flags);
        }

        #[test]
        fn regular_files_are_rejected_without_modifying_flags() {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "masterhub-live-output-{}-{unique}",
                std::process::id()
            ));
            let file = File::create_new(&path).unwrap();
            std::fs::remove_file(path).unwrap();
            let original_flags = flags(&file);
            let error = match FdSink::duplicate(file.as_raw_fd()) {
                Ok(_) => panic!("regular file was accepted"),
                Err(error) => error,
            };
            assert_eq!(error.kind(), io::ErrorKind::Unsupported);
            assert!(error.to_string().contains("actively read pipe"));
            assert_eq!(flags(&file), original_flags);
        }

        #[test]
        fn real_pipe_writes_an_entire_record() {
            use std::io::Read;

            let (mut reader, output) = pipe();
            let sink = FdSink::duplicate(output.as_raw_fd()).unwrap();
            let mut writer = RecordWriter::new(sink, Duration::from_millis(500));
            writer.write_all(b"{\"event\":").unwrap();
            writer.write_all(b"1}\n").unwrap();
            writer.flush().unwrap();
            let mut received = [0_u8; 12];
            reader.read_exact(&mut received).unwrap();
            assert_eq!(&received, b"{\"event\":1}\n");
        }

        #[test]
        fn disconnected_pipe_poll_fails_immediately() {
            let (reader, output) = pipe();
            let mut sink = FdSink::duplicate(output.as_raw_fd()).unwrap();
            drop(reader);
            let error = sink
                .wait_writable(Instant::now() + Duration::from_millis(500))
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        }

        #[test]
        fn stalled_pipe_times_out_and_restores_flags_after_failure() {
            let (_reader, output) = pipe();
            let original_flags = flags(&output);
            let mut sink = FdSink::duplicate(output.as_raw_fd()).unwrap();
            let fill = [0_u8; 4096];
            loop {
                match sink.write(&fill) {
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    result => panic!("unexpected pipe fill result: {result:?}"),
                }
            }
            let mut writer = RecordWriter::new(sink, Duration::from_millis(30));
            let start = Instant::now();
            assert_eq!(
                writer.write(b"x").unwrap_err().kind(),
                io::ErrorKind::TimedOut
            );
            let elapsed = start.elapsed();
            assert!(elapsed >= Duration::from_millis(25), "{elapsed:?}");
            assert!(elapsed < Duration::from_millis(500), "{elapsed:?}");
            drop(writer);
            // macOS adds a kernel-managed pipe status bit after writing. Check
            // the mutable flags restored by F_SETFL rather than that whole word.
            let mutable_flags = libc::O_NONBLOCK | libc::O_APPEND | libc::O_ASYNC;
            assert_eq!(
                flags(&output) & mutable_flags,
                original_flags & mutable_flags
            );
        }

        #[test]
        fn timeout_validation_enforces_live_shutdown_bound() {
            assert!(validate_timeout(Duration::ZERO).is_err());
            assert!(validate_timeout(Duration::from_millis(501)).is_err());
            assert!(validate_timeout(Duration::from_millis(500)).is_ok());
        }

        #[test]
        fn public_writer_is_send() {
            fn assert_send<T: Send>() {}
            assert_send::<super::super::TimedStdout>();
        }
    }
}
