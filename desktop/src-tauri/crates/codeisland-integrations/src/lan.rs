//! LAN sync: a tiny read-only state endpoint plus token-authenticated actions.
//!
//! The server binds `0.0.0.0:0` (ephemeral port) and is off unless the `lan`
//! feature is enabled. Pairing is a short code exchanged for an HMAC token;
//! the secret lives only in memory and rotates every process start.

use std::sync::Arc;

use codeisland_domain::{StoreSnapshot, UserAction};
use serde::{Deserialize, Serialize};

/// Read-only snapshot source supplied by the runtime.
pub type SnapshotSource = Arc<dyn Fn() -> StoreSnapshot + Send + Sync>;
/// Optional sink for approve/deny/answer actions arriving over LAN.
pub type ActionSink = Arc<dyn Fn(LanAction) + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanAction {
    pub session_id: String,
    pub event_id: String,
    pub action: UserAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairBody {
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairResult {
    pub token: String,
}

#[derive(Debug, thiserror::Error)]
pub enum LanError {
    #[error("LAN support is not compiled into this build")]
    Unsupported,
    #[error("failed to bind LAN server: {0}")]
    Bind(String),
    #[error("LAN server I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Handle to a running server.
pub struct LanServer {
    pub port: u16,
    pub pairing_code: String,
    token: String,
    shutdown: Option<tokio::sync::watch::Sender<bool>>,
    join: Option<tokio::task::JoinHandle<()>>,
}

impl LanServer {
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Stops the HTTP server. Idempotent.
    pub async fn shutdown(mut self) {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(true);
        }
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
    }
}

/// Token generation and pairing codes. Always compiled so the primitives can
/// be unit-tested without the `lan` feature.
pub struct LanCrypto {
    secret: [u8; 32],
}

impl LanCrypto {
    pub fn new(secret: [u8; 32]) -> Self {
        Self { secret }
    }

    pub fn random() -> Self {
        Self::new(crate::random_bytes::<32>())
    }

    pub fn token(&self) -> String {
        hex(&hmac_sha256(&self.secret, b"codeisland-lan-token-v1"))
    }

    pub fn verify(&self, token: &str) -> bool {
        constant_time_eq(self.token().as_bytes(), token.trim().as_bytes())
    }
}

const PAIRING_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
pub const PAIRING_CODE_LEN: usize = 6;

pub fn generate_pairing_code() -> String {
    pairing_code_from_bytes(&crate::random_bytes::<8>())
}

/// Deterministic helper (tests, fixtures): maps bytes onto the unambiguous
/// 6-character alphabet.
pub fn pairing_code_from_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(PAIRING_CODE_LEN);
    for index in 0..PAIRING_CODE_LEN {
        let byte = bytes.get(index).copied().unwrap_or(0) as usize;
        out.push(PAIRING_ALPHABET[byte % PAIRING_ALPHABET.len()] as char);
    }
    out
}

#[cfg(feature = "lan")]
mod server {
    use std::sync::Arc;

    use axum::{
        extract::State,
        http::{HeaderMap, StatusCode},
        routing::{get, post},
        Json, Router,
    };
    use serde_json::{json, Value};
    use tokio::sync::watch;
    use tower_http::cors::CorsLayer;
    use tower_http::trace::TraceLayer;

    use super::*;

    struct LanState {
        source: SnapshotSource,
        sink: Option<ActionSink>,
        crypto: LanCrypto,
        pairing_code: String,
    }

    impl LanServer {
        /// Starts the server on an ephemeral port and returns the bound port,
        /// the pairing code and the token that a client obtains via pairing.
        pub async fn start(source: SnapshotSource) -> Result<LanServer, LanError> {
            Self::start_with_actions(source, None).await
        }

        pub async fn start_with_actions(
            source: SnapshotSource,
            sink: Option<ActionSink>,
        ) -> Result<LanServer, LanError> {
            let crypto = LanCrypto::random();
            let pairing_code = generate_pairing_code();
            let token = crypto.token();
            let state = Arc::new(LanState {
                source,
                sink,
                crypto,
                pairing_code: pairing_code.clone(),
            });

            let app = Router::new()
                .route("/api/state", get(get_state))
                .route("/api/pair", post(post_pair))
                .route("/api/action", post(post_action))
                .layer(CorsLayer::permissive())
                .layer(TraceLayer::new_for_http())
                .with_state(state);

            let listener = tokio::net::TcpListener::bind("0.0.0.0:0")
                .await
                .map_err(|error| LanError::Bind(error.to_string()))?;
            let port = listener
                .local_addr()
                .map_err(|error| LanError::Bind(error.to_string()))?
                .port();

            let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
            let join = tokio::spawn(async move {
                let result = axum::serve(listener, app)
                    .with_graceful_shutdown(async move {
                        let _ = shutdown_rx.changed().await;
                    })
                    .await;
                if let Err(error) = result {
                    tracing::warn!(%error, "LAN server stopped with an error");
                }
            });

            Ok(LanServer {
                port,
                pairing_code,
                token,
                shutdown: Some(shutdown_tx),
                join: Some(join),
            })
        }
    }

    async fn get_state(State(state): State<Arc<LanState>>) -> Json<StoreSnapshot> {
        Json((state.source.as_ref())())
    }

    async fn post_pair(
        State(state): State<Arc<LanState>>,
        Json(body): Json<PairBody>,
    ) -> Result<Json<PairResult>, (StatusCode, Json<Value>)> {
        if !constant_time_eq(
            body.code.trim().to_ascii_uppercase().as_bytes(),
            state.pairing_code.as_bytes(),
        ) {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "invalid pairing code" })),
            ));
        }
        Ok(Json(PairResult {
            token: state.crypto.token(),
        }))
    }

    async fn post_action(
        State(state): State<Arc<LanState>>,
        headers: HeaderMap,
        Json(action): Json<LanAction>,
    ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        let token = bearer_token(&headers);
        match token {
            Some(token) if state.crypto.verify(token) => {}
            _ => {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    Json(json!({ "error": "invalid or missing token" })),
                ))
            }
        }
        if let Some(sink) = &state.sink {
            let sink = sink.as_ref();
            sink(action.clone());
        }
        Ok(Json(json!({ "ok": true, "event_id": action.event_id })))
    }

    fn bearer_token(headers: &HeaderMap) -> Option<&str> {
        headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .or_else(|| {
                headers
                    .get("x-codeisland-token")
                    .and_then(|value| value.to_str().ok())
            })
    }
}

#[cfg(not(feature = "lan"))]
mod server {
    use super::*;

    impl LanServer {
        pub async fn start(_source: SnapshotSource) -> Result<LanServer, LanError> {
            Err(LanError::Unsupported)
        }

        pub async fn start_with_actions(
            _source: SnapshotSource,
            _sink: Option<ActionSink>,
        ) -> Result<LanServer, LanError> {
            Err(LanError::Unsupported)
        }
    }
}

// ---------------------------------------------------------------------------
// The LAN token's wire format predates the Apple companion transport.
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    codeisland_swift::call(c"lan.digest", serde_json::json!({"key": key, "message": message}))
}

#[cfg(test)]
fn sha256(message: &[u8]) -> [u8; 32] {
    codeisland_swift::call(c"lan.digest", serde_json::json!({"message": message}))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        difference |= *a ^ *b;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hmac_sha256_rfc4231_case_2() {
        // RFC 4231 test case 2: key "Jefe", data "what do ya want for nothing?"
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn token_verifies_and_rejects_wrong_value() {
        let crypto = LanCrypto::new([7u8; 32]);
        let token = crypto.token();
        assert_eq!(token.len(), 64);
        assert!(crypto.verify(&token));
        assert!(crypto.verify(&format!("  {token}  ")));
        assert!(!crypto.verify("deadbeef"));
        assert!(!LanCrypto::new([8u8; 32]).verify(&token));
    }

    #[test]
    fn pairing_code_is_six_unambiguous_chars() {
        let code = generate_pairing_code();
        assert_eq!(code.len(), PAIRING_CODE_LEN);
        assert!(code
            .chars()
            .all(|ch| PAIRING_ALPHABET.contains(&(ch as u8))));
        // No 0/O/1/I lookalikes.
        assert!(!code.chars().any(|ch| "01OI".contains(ch)));
        assert_eq!(pairing_code_from_bytes(&[0, 1, 2, 3, 4, 5]), "ABCDEF");
    }

    #[test]
    fn constant_time_eq_matches_semantics() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
