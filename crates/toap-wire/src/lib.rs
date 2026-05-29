//! Transport framing for TOAP V1: each message is prefixed with a 4-byte big-endian length.
//!
//! TCP is a stream; the length prefix tells the receiver where one message ends. (Phase 1 of the
//! blueprint's transport layer.) These helpers are generic over any tokio async read/write half.

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Maximum frame length we accept (guards against a hostile/oversized length prefix).
pub const MAX_FRAME: usize = 1 << 20; // 1 MiB

/// Write one length-prefixed frame.
pub async fn write_frame<W>(w: &mut W, bytes: &[u8]) -> std::io::Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let len = bytes.len() as u32;
    w.write_all(&len.to_be_bytes()).await?;
    w.write_all(bytes).await?;
    w.flush().await?;
    Ok(())
}

/// Read one length-prefixed frame. Returns `Ok(None)` on a clean EOF before any bytes.
pub async fn read_frame<R>(r: &mut R) -> std::io::Result<Option<Vec<u8>>>
where
    R: AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame exceeds MAX_FRAME",
        ));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(Some(buf))
}
