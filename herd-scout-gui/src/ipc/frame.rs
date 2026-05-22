//! Length-prefixed framing for the IPC client.
//!
//! Mirrors `herd-scout-daemon::ipc::frame`. Kept inline rather than
//! shared via the `herd-scout-ipc` crate because it depends on tokio
//! and we want the shared crate to stay tiny + dependency-free.

use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_FRAME: u32 = 8 * 1024 * 1024;

pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame size {len} exceeds cap {MAX_FRAME}"),
        ));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, payload: &[u8]) -> io::Result<()> {
    if payload.len() as u64 > MAX_FRAME as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame size {} exceeds cap {MAX_FRAME}", payload.len()),
        ));
    }
    let len = (payload.len() as u32).to_be_bytes();
    w.write_all(&len).await?;
    w.write_all(payload).await?;
    w.flush().await?;
    Ok(())
}
