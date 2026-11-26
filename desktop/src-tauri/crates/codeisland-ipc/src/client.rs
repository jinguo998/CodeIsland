//! Blocking one-shot client used by `codeisland-hook`.
//!
//! Connect retries are done on the calling thread; the response read runs on a
//! helper thread so a hung app cannot block the hook past `read_timeout`.

use std::io::{self, BufRead, Read, Write};
use std::time::Duration;

use crate::protocol::{decode_line, encode_line, ClientMessage, ServerMessage};

/// Send one request and wait for one response.
///
/// `connect_timeout` bounds how long to wait for the app's pipe/socket to
/// exist and be free. `read_timeout` bounds how long to wait for the response
/// after the request has been written.
pub fn send(
    message: &ClientMessage,
    connect_timeout: Duration,
    read_timeout: Duration,
) -> io::Result<ServerMessage> {
    let line = encode_line(message).map_err(io::Error::other)?;
    let stream = platform_connect(connect_timeout)?;
    exchange(stream, &line, read_timeout)
}

fn exchange<C>(mut stream: C, line: &[u8], read_timeout: Duration) -> io::Result<ServerMessage>
where
    C: Read + Write + Send + 'static,
{
    stream.write_all(line)?;
    stream.flush()?;

    let reply = read_line_with_timeout(stream, read_timeout)?;
    decode_line(&reply).map_err(io::Error::other)
}

fn read_line_with_timeout<C>(stream: C, read_timeout: Duration) -> io::Result<Vec<u8>>
where
    C: Read + Send + 'static,
{
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stream);
        let mut line = Vec::new();
        let result = match reader.read_until(b'\n', &mut line) {
            Ok(0) => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "CodeIsland closed the connection without a response",
            )),
            Ok(_) => Ok(line),
            Err(error) => Err(error),
        };
        let _ = sender.send(result);
    });

    match receiver.recv_timeout(read_timeout) {
        Ok(result) => result,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "timed out waiting for a CodeIsland response",
        )),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            Err(io::Error::other("CodeIsland response reader stopped"))
        }
    }
}

#[cfg(windows)]
fn platform_connect(timeout: Duration) -> io::Result<std::fs::File> {
    crate::windows::connect(timeout)
}

#[cfg(unix)]
fn platform_connect(timeout: Duration) -> io::Result<std::os::unix::net::UnixStream> {
    crate::unix::connect(timeout)
}

#[cfg(not(any(windows, unix)))]
fn platform_connect(_timeout: Duration) -> io::Result<std::fs::File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "CodeIsland IPC is not implemented for this platform",
    ))
}
