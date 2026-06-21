//! Byte-level content comparison: full, sampled (quick), and binary detection.

use confold_vfs::{ContentReader, SourceError};

/// Block size for streaming and sampled comparisons.
const BLOCK: usize = 64 * 1024;

/// Bytes scanned from the start of a file when sniffing binary-vs-text.
const BINARY_SNIFF: usize = 8000;

/// Heuristically detect whether content is binary (contains a NUL byte in the first [`BINARY_SNIFF`]
/// bytes). Cheap and good enough to *label* a comparison; it does not affect the equality verdict.
pub fn is_binary(reader: &dyn ContentReader) -> bool {
    let limit = (reader.len() as usize).min(BINARY_SNIFF);
    if limit == 0 {
        return false;
    }
    if let Some(slice) = reader.as_slice() {
        return memchr::memchr(0, &slice[..limit]).is_some();
    }
    let mut buf = vec![0u8; limit];
    match reader.read_at(0, &mut buf) {
        Ok(n) => memchr::memchr(0, &buf[..n]).is_some(),
        Err(_) => false,
    }
}

/// Full byte-by-byte equality. Uses a zero-copy slice compare when both readers expose one (local mmap),
/// else streams in blocks.
pub fn full_equal(a: &dyn ContentReader, b: &dyn ContentReader) -> Result<bool, SourceError> {
    if a.len() != b.len() {
        return Ok(false);
    }
    if let (Some(sa), Some(sb)) = (a.as_slice(), b.as_slice()) {
        return Ok(sa == sb);
    }
    let len = a.len();
    let mut off = 0u64;
    let mut ba = vec![0u8; BLOCK];
    let mut bb = vec![0u8; BLOCK];
    while off < len {
        let na = a.read_at(off, &mut ba)?;
        let nb = b.read_at(off, &mut bb)?;
        if na != nb || na == 0 {
            return Ok(na == nb); // short read mismatch ⇒ not equal; both 0 ⇒ equal (reached end)
        }
        if ba[..na] != bb[..na] {
            return Ok(false);
        }
        off += na as u64;
    }
    Ok(true)
}

/// Sampled equality for large files: compares the head, the tail, and three interior blocks. Equal
/// samples are taken as "equal" — faster than a full read, at the cost of missing a difference that
/// falls entirely between samples. Callers should surface this uncertainty (see the engine's detail).
/// Assumes `a.len() == b.len()`.
pub fn quick_equal(a: &dyn ContentReader, b: &dyn ContentReader) -> Result<bool, SourceError> {
    debug_assert_eq!(a.len(), b.len());
    let len = a.len();
    let block = BLOCK as u64;
    let mut offsets = vec![0u64];
    if len > block {
        offsets.push(len.saturating_sub(block));
        offsets.push(len / 4);
        offsets.push(len / 2);
        offsets.push(len * 3 / 4);
    }
    for offset in offsets {
        if !range_equal(a, b, offset)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Compare up to one [`BLOCK`] starting at `offset` on both readers.
fn range_equal(
    a: &dyn ContentReader,
    b: &dyn ContentReader,
    offset: u64,
) -> Result<bool, SourceError> {
    let mut ba = vec![0u8; BLOCK];
    let mut bb = vec![0u8; BLOCK];
    let na = a.read_at(offset, &mut ba)?;
    let nb = b.read_at(offset, &mut bb)?;
    Ok(na == nb && ba[..na] == bb[..nb])
}
