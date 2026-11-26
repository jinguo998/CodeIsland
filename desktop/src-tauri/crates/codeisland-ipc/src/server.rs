//! Connection accept loop and per-connection NDJSON handling.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use crate::endpoint::endpoint_name;
use crate::protocol::{decode_line, encode_line, ClientMessage, ServerMessage, PROTOCOL_VERSION};

#[cfg(unix)]
use tokio::net::UnixListener;
#[cfg(windows)]
use tokio::net::windows::named_pipe::ServerOptions;

/// A request handler. The app supplies this to [`serve`]; the returned future
/// resolves to the response to write back, or `None` to close silently.
pub type Handler = Arc<
    dyn Fn(ClientMessage) -> Pin<Box<dyn Future<Output = Option<ServerMessage>> + Send>>
        + Send
        + Sync,
>;

/// How long a connection may sit idle while reading a request or writing a
/// response before it is dropped. Handlers are not subject to this limit: a
/// blocking permission wait is bounded by the runtime's own decision timeout.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Accepts connections forever. One NDJSON request and one NDJSON response per
/// connection.
pub async fn serve(handler: Handler) -> io::Result<()> {
    #[cfg(windows)]
    {
        serve_windows(handler).await
    }

    #[cfg(unix)]
    {
        serve_unix(handler).await
    }

    #[cfg(not(any(windows, unix)))]
    {
        let _ = handler;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "CodeIsland IPC is not implemented for this platform",
        ))
    }
}

#[cfg(windows)]
async fn serve_windows(handler: Handler) -> io::Result<()> {
    let path = endpoint_name();

    loop {
        // The SECURITY_DESCRIPTOR only has to outlive CreateNamedPipeW, and
        // SECURITY_ATTRIBUTES holds raw pointers, so build it inside a scope
        // that ends before the next `.await` (keeps this future `Send`).
        let server = {
            let mut security = crate::windows::PipeSecurity::for_current_user()?;
            let options = ServerOptions::new();
            crate::windows::create_pipe_server(&options, &path, &mut security)?
        };

        server.connect().await?;

        let handler = Arc::clone(&handler);
        tokio::spawn(async move {
            if let Err(error) = handle_connection(server, handler).await {
                tracing::debug!(%error, "codeisland ipc connection ended");
            }
        });
    }
}

#[cfg(unix)]
async fn serve_unix(handler: Handler) -> io::Result<()> {
    let path = endpoint_name();

    // A socket file left behind by a crashed process would make bind fail.
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let listener = UnixListener::bind(&path)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }

    loop {
        let (stream, _address) = listener.accept().await?;
        let handler = Arc::clone(&handler);
        tokio::spawn(async move {
            if let Err(error) = handle_connection(stream, handler).await {
                tracing::debug!(%error, "codeisland ipc connection ended");
            }
        });
    }
}

async fn handle_connection<S>(stream: S, handler: Handler) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut line = Vec::new();

    let read = tokio::time::timeout(IDLE_TIMEOUT, reader.read_until(b'\n', &mut line))
        .await
        .map_err(|_| idle_timeout())??;
    if read == 0 {
        return Ok(());
    }

    let response = match decode_line::<ClientMessage>(&line) {
        Ok(message) => handler(message).await,
        Err(error) => Some(ServerMessage::Error {
            v: PROTOCOL_VERSION,
            id: String::new(),
            error: format!("invalid message: {error}"),
        }),
    };

    if let Some(response) = response {
        let bytes = encode_line(&response).map_err(io::Error::other)?;
        tokio::time::timeout(IDLE_TIMEOUT, write_half.write_all(&bytes))
            .await
            .map_err(|_| idle_timeout())??;
        tokio::time::timeout(IDLE_TIMEOUT, write_half.flush())
            .await
            .map_err(|_| idle_timeout())??;
    }

    Ok(())
}

fn idle_timeout() -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        "codeisland ipc connection idle timeout",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeisland_domain::{AgentEvent, AgentEventType, AgentKind};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    fn echo_handler() -> Handler {
        Arc::new(|message| {
            Box::pin(async move {
                match message {
                    ClientMessage::Heartbeat { v, id } => Some(ServerMessage::Ack {
                        v,
                        id: "server-1".to_string(),
                        ack_for: id,
                    }),
                    _ => None,
                }
            })
        })
    }

    #[tokio::test]
    async fn handles_one_request_and_response() {
        let (server_side, client_side) = tokio::io::duplex(8 * 1024);
        let task = tokio::spawn(handle_connection(server_side, echo_handler()));

        let mut client = client_side;
        let request = ClientMessage::Heartbeat {
            v: PROTOCOL_VERSION,
            id: "req-1".to_string(),
        };
        client
            .write_all(&encode_line(&request).expect("encode"))
            .await
            .expect("write");

        let mut reader = BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line).await.expect("read");
        match decode_line::<ServerMessage>(line.as_bytes()).expect("decode") {
            ServerMessage::Ack { v, id, ack_for } => {
                assert_eq!(v, PROTOCOL_VERSION);
                assert_eq!(id, "server-1");
                assert_eq!(ack_for, "req-1");
            }
            other => panic!("unexpected response: {other:?}"),
        }

        task.await.expect("join").expect("handler");
    }

    #[tokio::test]
    async fn malformed_request_gets_an_error_frame() {
        let (server_side, client_side) = tokio::io::duplex(8 * 1024);
        let task = tokio::spawn(handle_connection(server_side, echo_handler()));

        let mut client = client_side;
        client.write_all(b"this is not json\n").await.expect("write");

        let mut reader = BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line).await.expect("read");
        match decode_line::<ServerMessage>(line.as_bytes()).expect("decode") {
            ServerMessage::Error { error, .. } => {
                assert!(error.contains("invalid message"), "error: {error}");
            }
            other => panic!("unexpected response: {other:?}"),
        }

        task.await.expect("join").expect("handler");
    }

    #[tokio::test]
    async fn none_response_closes_without_writing() {
        let (server_side, client_side) = tokio::io::duplex(8 * 1024);
        let task = tokio::spawn(handle_connection(server_side, echo_handler()));

        let mut client = client_side;
        let request = ClientMessage::Event {
            v: PROTOCOL_VERSION,
            id: "req-1".to_string(),
            source: "claude".to_string(),
            event: AgentEvent::new("session-1", AgentKind::Claude, AgentEventType::ToolStart),
            waits: false,
            timeout_ms: None,
        };
        client
            .write_all(&encode_line(&request).expect("encode"))
            .await
            .expect("write");

        let mut reader = BufReader::new(client);
        let mut line = String::new();
        let read = reader.read_line(&mut line).await.expect("read");
        assert_eq!(read, 0);
        assert!(line.is_empty());

        task.await.expect("join").expect("handler");
    }
}
