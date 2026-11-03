//! At-rest secret encryption: AES-256-GCM with a 12-byte random nonce
//! prefixed to the ciphertext. Used for OIDC client secrets and TOTP
//! secrets — never returned after write, never logged (§6 secrets rule).
//!
//! Known gap (documented honestly): this is a single app-level key, not
//! full envelope encryption (per-org data keys wrapped by a master key).
//! Envelope mode is the upgrade path and does not change call sites.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("ciphertext malformed or tampered")]
    Corrupt,
    #[error("encryption unavailable")]
    Backend(String),
}

const NONCE_LEN: usize = 12;

/// A fresh 12-byte nonce from the OS CSPRNG.
///
/// `aead` 0.6 removed the `AeadCore::generate_nonce` convenience this module
/// used, so the bytes are drawn from `rand` and wrapped in the nonce type
/// here. The wire format (`nonce || ciphertext`) is unchanged, so values
/// sealed before the bump still open.
fn fresh_nonce() -> [u8; NONCE_LEN] {
    let mut bytes = [0u8; NONCE_LEN];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
    bytes
}

fn cipher(key: &crate::config::SecretKey) -> Result<Aes256Gcm, CryptoError> {
    Aes256Gcm::new_from_slice(&key.0).map_err(|e| CryptoError::Backend(e.to_string()))
}

/// Encrypts `plaintext` into `nonce || ciphertext`.
pub fn seal(key: &crate::config::SecretKey, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let c = cipher(key)?;
    let nonce = Nonce::from(fresh_nonce());
    let ct = c
        .encrypt(&nonce, plaintext)
        .map_err(|e| CryptoError::Backend(e.to_string()))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(nonce.as_slice());
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypts a value produced by [`seal`]. Fails closed on any tampering.
pub fn open(key: &crate::config::SecretKey, sealed: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() <= NONCE_LEN {
        return Err(CryptoError::Corrupt);
    }
    let c = cipher(key)?;
    let (nonce_bytes, ct) = sealed.split_at(NONCE_LEN);
    let nonce = Nonce::try_from(nonce_bytes).map_err(|_| CryptoError::Corrupt)?;
    c.decrypt(&nonce, ct).map_err(|_| CryptoError::Corrupt)
}

/// Encrypts `plaintext` into `nonce || ciphertext`, binding the
/// ciphertext to the given ADDITIONAL AUTHENTICATED DATA (the wave-16
/// brief: secretvault ciphertexts bind an AAD of
/// `(org_id, record_id, field_key)` so bits cannot be replayed across
/// coordinates). The wire format is identical to [`seal`] — the AAD is
/// positionally bound by the coordinate, never stored.
pub fn seal_with_aad(
    key: &crate::config::SecretKey,
    plaintext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let c = cipher(key)?;
    let nonce = Nonce::from(fresh_nonce());
    let ct = c
        .encrypt(
            &nonce,
            aes_gcm::aead::Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|e| CryptoError::Backend(e.to_string()))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(nonce.as_slice());
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypts a value produced by [`seal_with_aad`]. Fails closed on any
/// tampering or a coordinate mismatch (the AAD roundtrip).
pub fn open_with_aad(
    key: &crate::config::SecretKey,
    sealed: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() <= NONCE_LEN {
        return Err(CryptoError::Corrupt);
    }
    let c = cipher(key)?;
    let (nonce_bytes, ct) = sealed.split_at(NONCE_LEN);
    let nonce = Nonce::try_from(nonce_bytes).map_err(|_| CryptoError::Corrupt)?;
    c.decrypt(&nonce, aes_gcm::aead::Payload { msg: ct, aad })
        .map_err(|_| CryptoError::Corrupt)
}

/// Fresh identifier for a stored secret row.
pub fn new_id() -> Uuid {
    Uuid::now_v7()
}

// ---------------------------------------------------------------------------
// Chunked file framing (Wave 19 lane 1 `p19-vault-scale`): true streaming
// decryption for GET /api/secrets/records/{id}/file. New PUTs frame files
// as `[MRDV2]` envelopes so GET decrypts chunk by chunk off the store
// stream instead of buffering the whole ciphertext. Legacy single-envelope
// files (Wave 16) stay readable via the unframed path.
//
// FORMAT V2 ("MRDV2"):
//   header (25 bytes) = magic "MRDV2" (5) || nonce_prefix (8, random)
//                       || plaintext_len (u64 BE) || chunk_size (u32 BE)
//   body              = per-chunk ciphertexts, one after another.
//
// Per chunk i (0-based, nonce-derived):
//   nonce_i = nonce_prefix || u32 BE i          (unique — 2^32 chunk space)
//   aad_i   = base_aad || header || u32 BE i    (coordinate- and
//                                               order-binding, like
//                                               seal_with_aad + framing)
//   ct_i    = AES-256-GCM(plaintext chunk i)    (ct includes the 16-byte
//                                               GCM tag, aead-crates
//                                               encrypt output)
//
// Authentication properties:
// * the header is AAD of EVERY chunk — plaintext_len, chunk_size, and the
//   prefix cannot be forged or transplanted without breaking all chunks;
// * the chunk index is AAD — chunks cannot be reordered or replayed into
//   another index (extends the Wave 16 coordinate-binding rule to the
//   file frame);
// * the base_aad `(org_id, record_id)` rides every chunk, so frames stay
//   coordinate-bound exactly like the legacy single-envelope seal;
// * truncation is caught at finish(): produced plaintext must equal the
//   authenticated `plaintext_len`;
// * the 12-byte nonce is fresh per object (8 random prefix || 4-byte
//   chunk counter) — no 96-bit-limit construction, so this frame stays
//   inside the single-key nonce-safety envelope the module documents.
// ---------------------------------------------------------------------------

pub const STREAM_MAGIC: [u8; 5] = *b"MRDV2";
const STREAM_HEADER_LEN: usize = 25;
const CHUNK_PLAIN: usize = 64 * 1024;
const GCM_TAG_LEN: usize = 16;

/// AAD length of the shared route builder `file_aad()` (36-hex org +
/// 36-hex record + separator): carried here only for capacity pre-sizing,
/// the route's builder stays authoritative.
pub fn base_file_aad_len() -> usize {
    73
}

fn chunk_count_for(plain_len: u64, chunk_size: u32) -> u32 {
    if plain_len == 0 {
        1
    } else {
        u32::try_from(
            plain_len
                .saturating_sub(1)
                .saturating_div(u64::from(chunk_size))
                + 1,
        )
        .unwrap_or(u32::MAX)
    }
}

fn chunk_start(plain_len: u64, chunk_size: u32, chunk: u32) -> usize {
    let start = u64::from(chunk).saturating_mul(u64::from(chunk_size));
    usize::try_from(start.min(plain_len)).unwrap_or(usize::MAX)
}

fn chunk_nonce(prefix: &[u8; 8], chunk: u32) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[..8].copy_from_slice(prefix);
    n[8..].copy_from_slice(&chunk.to_be_bytes());
    n
}

/// Seals `plaintext` into a `[MRDV2]` framed envelope bound to
/// `base_aad` (the PUT file path's AAD of `(org_id, record_id)`).
pub fn seal_file_frame(
    key: &crate::config::SecretKey,
    plaintext: &[u8],
    base_aad: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let chunk_size = CHUNK_PLAIN as u32;
    let plain_len = plaintext.len() as u64;
    let total = chunk_count_for(plain_len, chunk_size);
    let mut header = Vec::with_capacity(STREAM_HEADER_LEN);
    header.extend_from_slice(&STREAM_MAGIC);
    let fresh = fresh_nonce();
    let mut prefix = [0u8; 8];
    prefix.copy_from_slice(&fresh[..8]);
    header.extend_from_slice(&prefix);
    header.extend_from_slice(&plain_len.to_be_bytes());
    header.extend_from_slice(&chunk_size.to_be_bytes());
    debug_assert_eq!(header.len(), STREAM_HEADER_LEN);

    let c = cipher(key)?;
    #[allow(clippy::cast_possible_truncation)]
    let mut out =
        Vec::with_capacity(STREAM_HEADER_LEN + plaintext.len() + total as usize * GCM_TAG_LEN);
    out.extend_from_slice(&header);
    let aad_cap = base_aad.len() + header.len() + 4;
    let mut aad = Vec::with_capacity(aad_cap);
    aad.extend_from_slice(base_aad);
    aad.extend_from_slice(&header);
    let base_len = aad.len();
    for chunk in 0..total {
        aad.truncate(base_len);
        aad.extend_from_slice(&chunk.to_be_bytes());
        let from = chunk_start(plain_len, chunk_size, chunk);
        let to = match chunk.checked_add(1) {
            Some(next) if next < total => chunk_start(plain_len, chunk_size, next),
            _ => plain_len as usize,
        };
        let plain = &plaintext[from..to];
        let ct = c
            .encrypt(
                &Nonce::from(chunk_nonce(&prefix, chunk)),
                aes_gcm::aead::Payload {
                    msg: plain,
                    aad: aad.as_slice(),
                },
            )
            .map_err(|e| CryptoError::Backend(e.to_string()))?;
        out.extend_from_slice(&ct);
    }
    Ok(out)
}

/// Streaming decryptor for a `[MRDV2]` file frame: feed ciphertext bytes
/// (any slicing — store chunks never align with frame chunks), buffer
/// until the next whole envelope chunk is available, then decrypt it and
/// return the plaintext emitted so far. Finish by confirming the produced
/// length equals the authenticated header length.
/// Streaming decryptor for a `[MRDV2]` file frame: feed ciphertext bytes
/// (any slicing — store chunks never align with frame chunks); each
/// `feed` call decrypts and returns the next chunk of plaintext as soon
/// as one whole envelope chunk is buffered. Finish with [`Self::finish`]
/// after the credential stream ends, confirming the produced length
/// equals the authenticated header length.
pub struct OpenStream {
    key: crate::config::SecretKey,
    base_aad: Vec<u8>,
    prefix: [u8; 8],
    plain_len: u64,
    chunk_size: u32,
    total: u32,
    next_chunk: u32,
    buf: Vec<u8>,
    produced: u64,
    header_read: bool,
}

impl OpenStream {
    #[must_use]
    pub fn new(key: crate::config::SecretKey, base_aad: Vec<u8>) -> Self {
        Self {
            key,
            base_aad,
            prefix: [0u8; 8],
            plain_len: 0,
            chunk_size: 0,
            total: 0,
            next_chunk: 0,
            buf: Vec::new(),
            produced: 0,
            header_read: false,
        }
    }

    fn open_header(&mut self) -> Result<(), CryptoError> {
        if self.buf.len() < STREAM_HEADER_LEN {
            return Ok(());
        }
        if self.buf[..5] != STREAM_MAGIC {
            return Err(CryptoError::Corrupt);
        }
        self.prefix.copy_from_slice(&self.buf[5..13]);
        self.plain_len = u64::from_be_bytes(
            self.buf[13..21]
                .try_into()
                .map_err(|_| CryptoError::Corrupt)?,
        );
        self.chunk_size = u32::from_be_bytes(
            self.buf[21..25]
                .try_into()
                .map_err(|_| CryptoError::Corrupt)?,
        );
        if self.chunk_size == 0 {
            return Err(CryptoError::Corrupt);
        }
        self.total = chunk_count_for(self.plain_len, self.chunk_size);
        self.base_aad
            .extend_from_slice(&self.buf[..STREAM_HEADER_LEN]);
        self.buf.drain(..STREAM_HEADER_LEN);
        self.header_read = true;
        Ok(())
    }

    /// `Ok(None)` — no whole chunk is buffered yet; push more ciphertext.
    pub fn feed(&mut self, ct: &[u8]) -> Result<Option<Vec<u8>>, CryptoError> {
        self.buf.extend_from_slice(ct);
        if !self.header_read {
            self.open_header()?;
            if !self.header_read {
                return Ok(None);
            }
        }
        if self.next_chunk >= self.total {
            return Ok(None);
        }
        // The LAST chunk's plaintext length comes from the header
        // (plain_len - consumed); the expected CIPHERTEXT length is
        // exactly that + the GCM tag — the final chunk is only
        // decryptable once whole, everything else is exact-sized.
        let is_last = self.next_chunk + 1 == self.total;
        let expect_plain = if is_last {
            self.plain_len
                .saturating_sub(u64::from(self.next_chunk) * u64::from(self.chunk_size))
        } else {
            u64::from(self.chunk_size)
        };
        let span = expect_plain as usize + GCM_TAG_LEN;
        if self.buf.len() < span {
            return Ok(None);
        }
        let ct_chunk: Vec<u8> = self.buf.drain(..span).collect();
        let mut aad = Vec::with_capacity(self.base_aad.len() + 4);
        aad.extend_from_slice(&self.base_aad);
        aad.extend_from_slice(&self.next_chunk.to_be_bytes());
        let plain = self
            .cipher()?
            .decrypt(
                &Nonce::from(chunk_nonce(&self.prefix, self.next_chunk)),
                aes_gcm::aead::Payload {
                    msg: ct_chunk.as_ref(),
                    aad: aad.as_slice(),
                },
            )
            .map_err(|_| CryptoError::Corrupt)?;
        if plain.len() as u64 != expect_plain {
            return Err(CryptoError::Corrupt);
        }
        self.produced += plain.len() as u64;
        self.next_chunk += 1;
        Ok(Some(plain))
    }

    fn cipher(&self) -> Result<Aes256Gcm, CryptoError> {
        Aes256Gcm::new_from_slice(&self.key.0).map_err(|e| CryptoError::Backend(e.to_string()))
    }

    #[must_use]
    pub fn produced_len(&self) -> u64 {
        self.produced
    }

    /// Steals any trailing buffered bytes: after the store stream ends,
    /// bytes beyond the authenticated length are tampering.
    pub fn finish(&mut self) -> Result<(), CryptoError> {
        if !self.header_read || self.produced != self.plain_len || !self.buf.is_empty() {
            return Err(CryptoError::Corrupt);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> crate::config::SecretKey {
        crate::config::SecretKey([7u8; 32])
    }

    #[test]
    fn seal_open_roundtrip() {
        let sealed = seal(&key(), b"top-secret-value").unwrap();
        assert_ne!(sealed, b"top-secret-value");
        assert_eq!(open(&key(), &sealed).unwrap(), b"top-secret-value");
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let mut sealed = seal(&key(), b"payload").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0xFF;
        assert!(matches!(open(&key(), &sealed), Err(CryptoError::Corrupt)));
    }

    #[test]
    fn wrong_key_is_rejected() {
        let sealed = seal(&key(), b"payload").unwrap();
        let other = crate::config::SecretKey([9u8; 32]);
        assert!(matches!(open(&other, &sealed), Err(CryptoError::Corrupt)));
    }

    #[test]
    fn file_frame_roundtrip_exact_multi_of_64k() {
        let key = key();
        let base_aad = file_aad();
        let plain = vec![b'Q'; 4 * 64 * 1024];
        let frame = seal_file_frame(&key, &plain, &base_aad).unwrap();
        let mut decryptor = OpenStream::new(key, base_aad);
        let mut produced = decryptor.feed(&frame).unwrap().unwrap();
        while let Ok(Some(chunk)) = decryptor.feed(&[]) {
            produced.extend_from_slice(&chunk);
        }
        decryptor.finish().unwrap();
        assert_eq!(produced, plain);
    }

    #[test]
    fn nonces_are_fresh_per_seal() {
        let a = seal(&key(), b"same").unwrap();
        let b = seal(&key(), b"same").unwrap();
        assert_ne!(a, b, "nonce reuse would break GCM security");
    }

    /// Byte-range swap helper for tamper tests: moves range
    /// [a..a+len) up to [b..b+len) (callers guarantee a+len <= b).
    fn swap_ranges(bytes: &mut [u8], a: usize, b: usize, len: usize) -> Result<(), ()> {
        if bytes.len() < b + len || a + len > b {
            return Err(());
        }
        let (left, right) = bytes.split_at_mut(b);
        left[a..a + len].swap_with_slice(&mut right[..len]);
        Ok(())
    }

    fn file_aad() -> Vec<u8> {
        // The route's shared builder `file_aad()` shape: two 36-char
        // hex uuids (76 bytes in total keyed by the route).
        format!("{:>036}{:>036}", "org", "record").into_bytes()
    }

    fn stream_decrypt_all(
        key: &crate::config::SecretKey,
        base_aad: Vec<u8>,
        frame: &[u8],
        feed_size: usize,
    ) -> Result<Vec<u8>, CryptoError> {
        let mut decryptor = OpenStream::new(*key, base_aad);
        let mut produced = Vec::new();
        for slice in frame.chunks(feed_size.max(1)) {
            if let Some(chunk) = decryptor.feed(slice)? {
                produced.extend_from_slice(&chunk);
            }
        }
        while let Some(chunk) = decryptor.feed(&[])? {
            produced.extend_from_slice(&chunk);
        }
        decryptor.finish()?;
        Ok(produced)
    }

    #[test]
    fn file_frame_roundtrip_multi_chunk() {
        let key = key();
        let base_aad = file_aad();
        let mut plain = Vec::new();
        for i in 0..70_000u32 {
            plain.extend_from_slice(format!("block-{i};").as_bytes());
        }
        let frame = seal_file_frame(&key, &plain, &base_aad).unwrap();
        assert_eq!(
            &frame[..5],
            &STREAM_MAGIC,
            "every frame must open with the magnet"
        );
        // Feed store-chunk style: odd slicing far off the 64KiB body
        // chunks forces the buffered path many times.
        let round = stream_decrypt_all(&key, base_aad.clone(), &frame, 1000).unwrap();
        assert_eq!(round, plain);
        // Feed again but SPREAD apart bigger than the cipher chunk.
        let bigger = stream_decrypt_all(&key, base_aad, &frame, 2_000_000).unwrap();
        assert_eq!(bigger, plain);
    }

    #[test]
    fn file_frame_tampered_chunk_is_rejected() {
        let key = key();
        let frame = seal_file_frame(&key, &[b'x'; 80_000], &file_aad()).unwrap();
        let mid = frame.len() / 2;
        let mut tampered = frame;
        tampered[mid] ^= 0x01;
        let mut decryptor = OpenStream::new(key, file_aad());
        let mut saw_corrupt = false;
        for slice in tampered.chunks(4096) {
            if decryptor.feed(slice).is_err() {
                saw_corrupt = true;
                break;
            }
        }
        assert!(saw_corrupt, "bit flip in the body must fail closed");
    }

    #[test]
    fn file_frame_truncation_is_rejected_at_finish() {
        let key = key();
        let frame = seal_file_frame(&key, &[b'y'; 10_000], &file_aad()).unwrap();
        let cut = &frame[..frame.len() - 40];
        let mut decryptor = OpenStream::new(key, file_aad());
        for slice in cut.chunks(2048) {
            while decryptor.feed(slice).unwrap().is_some() {}
        }
        assert!(matches!(decryptor.finish(), Err(CryptoError::Corrupt)));
    }

    #[test]
    fn file_frame_reordered_chunk_is_rejected() {
        let key = key();
        let frame = seal_file_frame(&key, &[b'z'; 200_000], &file_aad()).unwrap();
        // Swap the first and second whole body ciphertexts — every
        // chunk's plaintext is exactly 64KiB here, so the swap keeps
        // byte-length unnoticeable and is STILL rejected (index AAD).
        let chunk_len = 64 * 1024 + 16;
        let mut reordered = frame.clone();
        swap_ranges(&mut reordered, 25, 25 + chunk_len, chunk_len).unwrap();
        let mut decryptor = OpenStream::new(key, file_aad());
        let mut saw_corrupt = false;
        for slice in reordered.chunks(4096) {
            if decryptor.feed(slice).is_err() {
                saw_corrupt = true;
                break;
            }
        }
        assert!(saw_corrupt, "chunk swap must fail closed");
    }

    #[test]
    fn file_frame_cannot_open_under_wrong_aad() {
        let key = key();
        let frame = seal_file_frame(&key, b"secret-bytes", &file_aad()).unwrap();
        let mut decryptor = OpenStream::new(key, format!("{:>072}", "other").into_bytes());
        let mut saw_corrupt = false;
        for slice in frame.chunks(4096) {
            if decryptor.feed(slice).is_err() {
                saw_corrupt = true;
                break;
            }
        }
        assert!(saw_corrupt, "coordinate mismatch must fail closed");
    }
}

// (appended to tests module? no — see below)
