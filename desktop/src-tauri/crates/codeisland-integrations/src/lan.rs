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
// SHA-256 / HMAC-SHA256 (RFC 2104), dependency-free
// ---------------------------------------------------------------------------

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
    0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
    0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
    0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
    0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
    0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
    0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
    0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
    0xc67178f2,
];

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hash: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let mut message = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in message.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (index, word) in w.iter_mut().take(16).enumerate() {
            let start = index * 4;
            *word = u32::from_be_bytes([
                chunk[start],
                chunk[start + 1],
                chunk[start + 2],
                chunk[start + 3],
            ]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }

        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h) = (
            hash[0], hash[1], hash[2], hash[3], hash[4], hash[5], hash[6], hash[7],
        );
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        hash[0] = hash[0].wrapping_add(a);
        hash[1] = hash[1].wrapping_add(b);
        hash[2] = hash[2].wrapping_add(c);
        hash[3] = hash[3].wrapping_add(d);
        hash[4] = hash[4].wrapping_add(e);
        hash[5] = hash[5].wrapping_add(f);
        hash[6] = hash[6].wrapping_add(g);
        hash[7] = hash[7].wrapping_add(h);
    }

    let mut out = [0u8; 32];
    for (index, word) in hash.iter().enumerate() {
        out[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&sha256(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36u8; 64];
    let mut outer_pad = [0x5cu8; 64];
    for index in 0..64 {
        inner_pad[index] ^= block[index];
        outer_pad[index] ^= block[index];
    }

    let mut inner = Vec::with_capacity(64 + message.len());
    inner.extend_from_slice(&inner_pad);
    inner.extend_from_slice(message);
    let inner_hash = sha256(&inner);

    let mut outer = Vec::with_capacity(96);
    outer.extend_from_slice(&outer_pad);
    outer.extend_from_slice(&inner_hash);
    sha256(&outer)
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
