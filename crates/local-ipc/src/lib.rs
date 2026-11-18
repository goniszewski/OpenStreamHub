//! The local connection between OpenStreamHub and the processes beside it: the plugin host and the ingress listen
//! on one each, and plugins, `osh` and agent hooks connect to them.
//!
//! An endpoint is named by a path in a directory only this user can open. On Unix it is a socket file at that path,
//! and every call here is the standard library's Unix socket call, unchanged. Other platforms are not supported yet:
//! [`Listener::bind`] and [`Stream::connect`] fail with [`io::ErrorKind::Unsupported`]. A Windows named pipe needs
//! read and write timeouts and a shutdown that wakes blocked threads, which every user here relies on, so it waits
//! until it can be tried on Windows.

use std::{
    io::{self, Read, Write},
    path::Path,
    time::Duration,
};

#[cfg(unix)]
use std::{
    fs,
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
};

/// Removes the endpoint at `path`, as before binding over a stale one or when the listener stops. A missing one is
/// [`io::ErrorKind::NotFound`], as for a file.
pub fn remove(path: impl AsRef<Path>) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::remove_file(path.as_ref())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(io::ErrorKind::NotFound.into())
    }
}

/// Lets only this user connect to the endpoint at `path` (the socket file becomes 0600).
pub fn protect(path: impl AsRef<Path>) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::set_permissions(path.as_ref(), fs::Permissions::from_mode(0o600))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(unsupported())
    }
}

#[cfg(not(unix))]
fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "local connections are not supported on this platform yet",
    )
}

/// Never constructed: what a listener or stream holds where there are none.
#[cfg(not(unix))]
#[derive(Debug)]
enum Never {}

/// A bound endpoint that accepts connections.
#[derive(Debug)]
pub struct Listener {
    #[cfg(unix)]
    inner: UnixListener,
    #[cfg(not(unix))]
    inner: Never,
}

impl Listener {
    /// Binds a new endpoint at `path`. It fails when one is already there: [`remove`] a stale one first, and
    /// [`protect`] the new one before anyone may connect.
    pub fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
        #[cfg(unix)]
        {
            UnixListener::bind(path.as_ref()).map(|inner| Self { inner })
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err(unsupported())
        }
    }

    /// Waits for the next connection.
    pub fn accept(&self) -> io::Result<Stream> {
        #[cfg(unix)]
        {
            self.inner.accept().map(|(inner, _)| Stream { inner })
        }
        #[cfg(not(unix))]
        {
            match self.inner {}
        }
    }

    /// Every connection as it comes, without end.
    pub fn incoming(&self) -> impl Iterator<Item = io::Result<Stream>> + '_ {
        std::iter::repeat_with(|| self.accept())
    }
}

/// One connection, read and written as bytes. `&Stream` reads and writes too, so one thread can read while another
/// writes through a [`Stream::try_clone`].
#[derive(Debug)]
pub struct Stream {
    #[cfg(unix)]
    inner: UnixStream,
    #[cfg(not(unix))]
    inner: Never,
}

impl Stream {
    /// Connects to the endpoint at `path`.
    pub fn connect(path: impl AsRef<Path>) -> io::Result<Self> {
        #[cfg(unix)]
        {
            UnixStream::connect(path.as_ref()).map(|inner| Self { inner })
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err(unsupported())
        }
    }

    /// Two streams connected to each other, for tests.
    pub fn pair() -> io::Result<(Self, Self)> {
        #[cfg(unix)]
        {
            UnixStream::pair().map(|(a, b)| (Self { inner: a }, Self { inner: b }))
        }
        #[cfg(not(unix))]
        {
            Err(unsupported())
        }
    }

    /// How long a read may wait (`None`: for ever); a read that waits longer fails with
    /// [`io::ErrorKind::WouldBlock`] or [`io::ErrorKind::TimedOut`]. Shared with every clone.
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        #[cfg(unix)]
        {
            self.inner.set_read_timeout(timeout)
        }
        #[cfg(not(unix))]
        {
            let _ = timeout;
            match self.inner {}
        }
    }

    /// How long a write may wait (`None`: for ever), as [`Stream::set_read_timeout`].
    pub fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        #[cfg(unix)]
        {
            self.inner.set_write_timeout(timeout)
        }
        #[cfg(not(unix))]
        {
            let _ = timeout;
            match self.inner {}
        }
    }

    /// Another handle on the same connection.
    pub fn try_clone(&self) -> io::Result<Self> {
        #[cfg(unix)]
        {
            self.inner.try_clone().map(|inner| Self { inner })
        }
        #[cfg(not(unix))]
        {
            match self.inner {}
        }
    }

    /// Ends the connection both ways, for every clone: blocked reads return the end and later writes fail, here and
    /// on the other side.
    pub fn shutdown(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            self.inner.shutdown(std::net::Shutdown::Both)
        }
        #[cfg(not(unix))]
        {
            match self.inner {}
        }
    }
}

impl Read for &Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        #[cfg(unix)]
        {
            (&self.inner).read(buf)
        }
        #[cfg(not(unix))]
        {
            let _ = buf;
            match self.inner {}
        }
    }
}

impl Write for &Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        #[cfg(unix)]
        {
            (&self.inner).write(buf)
        }
        #[cfg(not(unix))]
        {
            let _ = buf;
            match self.inner {}
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        {
            (&self.inner).flush()
        }
        #[cfg(not(unix))]
        {
            match self.inner {}
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&*self).read(buf)
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&*self).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        (&*self).flush()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        os::unix::fs::PermissionsExt,
        thread,
    };

    fn dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("local-ipc-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_line_goes_each_way_through_a_protected_endpoint() {
        let dir = dir("line");
        let path = dir.join("s");
        let listener = Listener::bind(&path).unwrap();
        protect(&path).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let server = thread::spawn(move || {
            let stream = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            (&stream).write_all(line.to_uppercase().as_bytes()).unwrap();
        });
        let stream = Stream::connect(&path).unwrap();
        (&stream).write_all(b"ping\n").unwrap();
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();
        assert_eq!(line, "PING\n");
        server.join().unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn binding_over_an_endpoint_fails_until_it_is_removed() {
        let dir = dir("stale");
        let path = dir.join("s");
        drop(Listener::bind(&path).unwrap());
        assert!(
            Listener::bind(&path).is_err(),
            "the stale socket file is still there"
        );
        remove(&path).unwrap();
        assert_eq!(remove(&path).unwrap_err().kind(), io::ErrorKind::NotFound);
        Listener::bind(&path).unwrap();
        assert_eq!(
            Stream::connect(dir.join("none")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_read_times_out_and_shutdown_ends_a_clone_blocked_in_a_read() {
        let (ours, theirs) = Stream::pair().unwrap();
        ours.set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let error = (&ours).read(&mut [0; 1]).unwrap_err();
        assert!(
            matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ),
            "{error}"
        );
        ours.set_read_timeout(None).unwrap();
        let blocked = ours.try_clone().unwrap();
        let reader = thread::spawn(move || (&blocked).read(&mut [0; 1]).unwrap());
        thread::sleep(Duration::from_millis(50));
        ours.shutdown().unwrap();
        assert_eq!(reader.join().unwrap(), 0, "the blocked read sees the end");
        assert_eq!(
            (&theirs).read(&mut [0; 1]).unwrap(),
            0,
            "so does the other side"
        );
        assert!((&ours).write_all(b"x").is_err());
    }
}
