//! AEAD-encrypted framing for iroh ACP connections.
//!
//! Wraps an `AsyncRead` + `AsyncWrite` (the iroh QUIC streams) with
//! ChaCha20-Poly1305 encryption using the session key from OPAQUE pairing.
//!
//! Frame format: `[4-byte BE length] [12-byte nonce] [ciphertext + tag]`

use std::io;
use std::pin::Pin;
use std::task::{ready, Context, Poll};

use chacha20poly1305::{aead::AeadInPlace, ChaCha20Poly1305, KeyInit, Nonce};
use tokio::io::{AsyncBufRead, AsyncRead, AsyncWrite, ReadBuf};

const TAG_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const HEADER_LEN: usize = 4;
const MAX_FRAME: usize = 1_048_576;

// ---------------------------------------------------------------------------
// Key derivation
// ---------------------------------------------------------------------------

pub fn key_from_session(session_key: &[u8]) -> [u8; 32] {
    let mut key = [0u8; 32];
    let len = session_key.len().min(32);
    key[..len].copy_from_slice(&session_key[..len]);
    key
}

// ---------------------------------------------------------------------------
// Nonce
// ---------------------------------------------------------------------------

fn make_nonce(counter: u64) -> Nonce {
    let mut n = Nonce::default();
    n[..8].copy_from_slice(&counter.to_be_bytes());
    n
}

// ---------------------------------------------------------------------------
// AEAD Reader
// ---------------------------------------------------------------------------

pin_project_lite::pin_project! {
    pub struct AeadReader<R> {
        #[pin]
        inner: R,
        cipher: ChaCha20Poly1305,
        counter: u64,
        plaintext: Vec<u8>,
        read_pos: usize,
        read_buf: Vec<u8>,
    }
}

impl<R> AeadReader<R> {
    pub fn new(inner: R, key: &[u8; 32]) -> Self {
        Self {
            inner,
            cipher: ChaCha20Poly1305::new_from_slice(key).expect("32-byte key"),
            counter: 0,
            plaintext: Vec::new(),
            read_pos: 0,
            read_buf: vec![0u8; MAX_FRAME],
        }
    }
}

macro_rules! refill_decrypted {
    ($inner:expr, $cipher:expr, $counter:expr, $plaintext:expr, $read_pos:expr, $read_buf:expr, $cx:expr) => {{
        if *$read_pos >= $plaintext.len() {
            $plaintext.clear();
            *$read_pos = 0;

            let mut header = [0u8; HEADER_LEN];
            let mut hdr_buf = ReadBuf::new(&mut header);
            ready!($inner.as_mut().poll_read($cx, &mut hdr_buf))?;
            if hdr_buf.filled().len() != HEADER_LEN {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "eof before AEAD header",
                )));
            }

            let frame_len = u32::from_be_bytes(header) as usize;
            if frame_len > MAX_FRAME {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("AEAD frame too large: {frame_len}"),
                )));
            }
            let total = NONCE_LEN + frame_len;

            let buf = &mut $read_buf[..total];
            let mut frm_buf = ReadBuf::new(buf);
            ready!($inner.as_mut().poll_read($cx, &mut frm_buf))?;
            if frm_buf.filled().len() != total {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "short AEAD frame",
                )));
            }

            let nonce = make_nonce(*$counter);
            *$counter += 1;

            let wire_nonce = &$read_buf[..NONCE_LEN];
            if wire_nonce != nonce.as_slice() {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "AEAD nonce mismatch",
                )));
            }

            let mut decrypted = $read_buf[NONCE_LEN..total].to_vec();
            $cipher
                .decrypt_in_place(&nonce, b"", &mut decrypted)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "AEAD decrypt failed"))?;

            *$plaintext = decrypted;
        }
        Poll::<io::Result<()>>::Ready(Ok(()))
    }};
}

impl<R: AsyncRead> AsyncRead for AeadReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let mut this = self.as_mut().project();

        ready!(refill_decrypted!(
            this.inner,
            this.cipher,
            this.counter,
            this.plaintext,
            this.read_pos,
            this.read_buf,
            cx
        ))?;

        let available = this.plaintext.len() - *this.read_pos;
        let to_copy = available.min(buf.remaining());
        buf.put_slice(&this.plaintext[*this.read_pos..*this.read_pos + to_copy]);
        *this.read_pos += to_copy;

        Poll::Ready(Ok(()))
    }
}

impl<R: AsyncRead> AsyncBufRead for AeadReader<R> {
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<&[u8]>> {
        let mut this = self.project();

        ready!(refill_decrypted!(
            this.inner,
            this.cipher,
            this.counter,
            this.plaintext,
            this.read_pos,
            this.read_buf,
            cx
        ))?;

        Poll::Ready(Ok(&this.plaintext[*this.read_pos..]))
    }

    fn consume(self: Pin<&mut Self>, amt: usize) {
        let this = self.project();
        *this.read_pos += amt;
    }
}

// ---------------------------------------------------------------------------
// AEAD Writer
// ---------------------------------------------------------------------------

pin_project_lite::pin_project! {
    pub struct AeadWriter<W> {
        #[pin]
        inner: W,
        cipher: ChaCha20Poly1305,
        counter: u64,
        buffer: Vec<u8>,
    }
}

impl<W> AeadWriter<W> {
    pub fn new(inner: W, key: &[u8; 32]) -> Self {
        Self {
            inner,
            cipher: ChaCha20Poly1305::new_from_slice(key).expect("32-byte key"),
            counter: 0,
            buffer: Vec::new(),
        }
    }
}

impl<W: AsyncWrite> AsyncWrite for AeadWriter<W> {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        let this = self.project();
        if buf.len() > MAX_FRAME - TAG_LEN {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("write exceeds max AEAD frame: {}", buf.len()),
            )));
        }
        this.buffer.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        let mut this = self.project();

        if this.buffer.is_empty() {
            return Poll::Ready(Ok(()));
        }

        let plaintext: Vec<u8> = this.buffer.drain(..).collect();
        let nonce = make_nonce(*this.counter);
        *this.counter += 1;

        let mut encrypted = plaintext;
        let tag = this
            .cipher
            .encrypt_in_place_detached(&nonce, b"", &mut encrypted)
            .map_err(|_| io::Error::other("AEAD encrypt failed"))?;
        encrypted.extend_from_slice(tag.as_slice());

        let frame_len = (encrypted.len() as u32).to_be_bytes();
        let n = ready!(this.inner.as_mut().poll_write(cx, &frame_len))?;
        if n != HEADER_LEN {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short header write",
            )));
        }
        let n = ready!(this.inner.as_mut().poll_write(cx, nonce.as_slice()))?;
        if n != NONCE_LEN {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short nonce write",
            )));
        }
        let n = ready!(this.inner.as_mut().poll_write(cx, &encrypted))?;
        if n != encrypted.len() {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short payload write",
            )));
        }
        this.inner.poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        self.project().inner.poll_shutdown(cx)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    #[tokio::test]
    async fn test_aead_roundtrip() {
        let key = [0xABu8; 32];

        let (cr, sw) = tokio::io::duplex(8192);
        let (sr, cw) = tokio::io::duplex(8192);

        let server = tokio::spawn(async move {
            let mut reader = BufReader::new(AeadReader::new(sr, &key));
            let mut writer = AeadWriter::new(sw, &key);

            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            assert_eq!(line, "hello encrypted world\n");

            writer.write_all(b"ack\n").await.unwrap();
            writer.flush().await.unwrap();
        });

        let mut reader = BufReader::new(AeadReader::new(cr, &key));
        let mut writer = AeadWriter::new(cw, &key);

        writer.write_all(b"hello encrypted world\n").await.unwrap();
        writer.flush().await.unwrap();

        let mut response = String::new();
        reader.read_line(&mut response).await.unwrap();
        assert_eq!(response, "ack\n");

        server.await.unwrap();
    }

    #[tokio::test]
    async fn test_aead_corrupt_frame_rejected() {
        let key = [0xABu8; 32];
        let (cr, mut cw) = tokio::io::duplex(4096);

        let junk = b"\x00\x00\x00\x04\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00nope";
        cw.write_all(junk).await.unwrap();

        let mut reader = BufReader::new(AeadReader::new(cr, &key));
        let mut line = String::new();
        let result = reader.read_line(&mut line).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_aead_multiframe() {
        let key = [0xABu8; 32];

        let (cr, sw) = tokio::io::duplex(8192);
        let (sr, cw) = tokio::io::duplex(8192);

        let server = tokio::spawn(async move {
            let mut reader = BufReader::new(AeadReader::new(sr, &key));
            let mut line1 = String::new();
            reader.read_line(&mut line1).await.unwrap();
            assert_eq!(line1, "one\n");
            let mut line2 = String::new();
            reader.read_line(&mut line2).await.unwrap();
            assert_eq!(line2, "two\n");
        });

        let mut writer = AeadWriter::new(cw, &key);
        writer.write_all(b"one\n").await.unwrap();
        writer.flush().await.unwrap();
        writer.write_all(b"two\n").await.unwrap();
        writer.flush().await.unwrap();

        server.await.unwrap();
        drop(cr); // silence unused warning
    }
}
