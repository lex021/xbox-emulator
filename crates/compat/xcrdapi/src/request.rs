use crate::abi::*;
use crate::bcpack;
use miniz_oxide::inflate::core::{decompress, inflate_flags, DecompressorOxide};
use miniz_oxide::inflate::TINFLStatus;
use std::fs::File;
use std::os::windows::fs::FileExt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct XRequest {
    pub options: u64,
    pub destination: u64,
    pub ext10: u16,
    pub destination_size: u32,
    pub source: u64,
    pub source_offset: u64,
    pub ext28: u16,
    pub source_size: u32,
    pub field34: u32,
    pub cancellation_tag: u64,
    pub name: u64,
}

impl XRequest {
    pub fn decode(b: &[u8; XREQUEST_SIZE]) -> XRequest {
        let q = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
        let d = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let w = |o: usize| u16::from_le_bytes(b[o..o + 2].try_into().unwrap());
        let options = q(0x00);
        let memory = options & OPT_SOURCE_MEMORY != 0;
        XRequest {
            options,
            destination: q(0x08),
            ext10: if options & OPT_EXT10_VALID != 0 { w(0x10) } else { 0 },
            destination_size: d(0x18),
            source: q(0x20),
            source_offset: if memory { 0 } else { q(0x28) },
            ext28: if memory && options & OPT_EXT28_VALID != 0 { w(0x28) } else { 0 },
            source_size: d(0x30),
            field34: d(0x34),
            cancellation_tag: q(0x38),
            name: q(0x40),
        }
    }

    pub unsafe fn read(p: *const u8) -> XRequest {
        let mut b = [0u8; XREQUEST_SIZE];
        unsafe { core::ptr::copy_nonoverlapping(p, b.as_mut_ptr(), XREQUEST_SIZE) };
        XRequest::decode(&b)
    }

    pub fn compression(&self) -> u8 {
        (self.options & (OPT_ZLIB | (OPT_BCPACK_MASK << OPT_BCPACK_SHIFT))) as u8
    }

    pub fn zlib(&self) -> bool {
        self.options & OPT_ZLIB != 0
    }

    pub fn bcpack_mode(&self) -> u8 {
        ((self.options >> OPT_BCPACK_SHIFT) & OPT_BCPACK_MASK) as u8
    }

    pub fn swizzle_mode(&self) -> u16 {
        ((self.options >> OPT_SWIZZLE_SHIFT) & OPT_SWIZZLE_MASK) as u16
    }

    pub fn memory_source(&self) -> bool {
        self.options & OPT_SOURCE_MEMORY != 0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Rejection {
    pub hr: i32,
    pub why: String,
}

fn reject(hr: i32, why: String) -> Result<(), Rejection> {
    Err(Rejection { hr, why })
}

pub fn validate(r: &XRequest, queue_source: u32) -> Result<(), Rejection> {
    let unknown = r.options & !OPT_KNOWN_MASK;
    if unknown != 0 {
        return reject(
            E_DSTORAGE_RESERVED_FIELDS,
            format!(
                "options {:#x} carry console-only bits {unknown:#x} (zlib={} bcpack={} swizzle={:#x} ext10={:#x} ext28={:#x} intermediate={:#x} dst={:#x} src={:#x}) whose semantics are not implemented",
                r.options,
                r.zlib() as u8,
                bcpack::mode_name(r.bcpack_mode()),
                r.swizzle_mode(),
                r.ext10,
                r.ext28,
                r.field34,
                r.destination_size,
                r.source_size
            ),
        );
    }
    let mem = r.memory_source();
    if mem != (queue_source == QUEUE_SOURCE_MEMORY) {
        return reject(
            E_DSTORAGE_INVALID_SOURCE_TYPE,
            format!("{} source request on a {} queue", if mem { "memory" } else { "file" }, if queue_source == QUEUE_SOURCE_MEMORY { "memory" } else { "file" }),
        );
    }
    if r.destination == 0 {
        return reject(E_DSTORAGE_ACCESS_VIOLATION, "null destination buffer".into());
    }
    if mem && r.source == 0 {
        return reject(E_DSTORAGE_ACCESS_VIOLATION, "null memory source".into());
    }
    let mode = r.bcpack_mode();
    if mode > bcpack::MODE_BC7 {
        return reject(E_DSTORAGE_DECOMPRESSION_ERROR, format!("options {:#x}: BCPack mode {mode} is not a DSTORAGE_BCPACK_MODE value", r.options));
    }
    if mode != bcpack::MODE_NONE && !bcpack::decodable(mode) {
        return reject(
            E_DSTORAGE_DECOMPRESSION_ERROR,
            format!(
                "options {:#x}: BCPack {} has no public block mapping (zlib={} intermediate={:#x} dst={:#x} src={:#x}); request rejected",
                r.options,
                bcpack::mode_name(mode),
                r.zlib() as u8,
                r.field34,
                r.destination_size,
                r.source_size
            ),
        );
    }
    match (r.zlib(), mode != bcpack::MODE_NONE) {
        (false, false) => {
            if r.destination_size != r.source_size {
                return reject(
                    E_DSTORAGE_INVALID_DESTINATION_SIZE,
                    format!("uncompressed request with destination size {:#x} != source size {:#x}", r.destination_size, r.source_size),
                );
            }
        }
        (true, false) => {
            if r.destination_size <= r.source_size {
                return reject(
                    E_DSTORAGE_INVALID_DESTINATION_SIZE,
                    format!("zlib request with destination size {:#x} not larger than source size {:#x}", r.destination_size, r.source_size),
                );
            }
        }
        (zlib, true) => {
            if r.destination_size == 0 || r.destination_size as usize % bcpack::BC1_BLOCK_BYTES != 0 {
                return reject(
                    E_DSTORAGE_INVALID_DESTINATION_SIZE,
                    format!("BCPack {} destination size {:#x} is not a whole number of blocks", bcpack::mode_name(mode), r.destination_size),
                );
            }
            if zlib && r.field34 == 0 {
                return reject(E_DSTORAGE_INVALID_INTERMEDIATE_SIZE, "zlib + BCPack request without an intermediate size".into());
            }
            if !zlib && r.field34 != 0 {
                return reject(E_DSTORAGE_INVALID_INTERMEDIATE_SIZE, format!("intermediate size {:#x} on a BCPack request without zlib", r.field34));
            }
            return Ok(());
        }
    }
    if r.field34 != 0 && r.field34 != r.destination_size {
        return reject(
            E_DSTORAGE_INVALID_INTERMEDIATE_SIZE,
            format!("field+0x34 = {:#x} (intermediate size) is only used by zlib + BCPack requests", r.field34),
        );
    }
    Ok(())
}

fn zlib_header_ok(src: &[u8]) -> bool {
    src.len() >= 2 && src[0] & 0x0f == 8 && (src[0] >> 4) <= 7 && ((src[0] as u16) << 8 | src[1] as u16) % 31 == 0 && src[1] & 0x20 == 0
}

pub fn inflate_into(src: &[u8], dst: &mut [u8]) -> Result<usize, i32> {
    if !zlib_header_ok(src) {
        return Err(E_DSTORAGE_ZLIB_BAD_HEADER);
    }
    let mut st = Box::new(DecompressorOxide::new());
    let flags = inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER
        | inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF
        | inflate_flags::TINFL_FLAG_COMPUTE_ADLER32;
    let (status, _consumed, written) = decompress(&mut st, src, dst, 0, flags);
    match status {
        TINFLStatus::Done => Ok(written),
        TINFLStatus::HasMoreOutput => Err(E_DSTORAGE_DECOMPRESSION_ERROR),
        TINFLStatus::Adler32Mismatch => Err(E_DSTORAGE_ZLIB_PARITY_FAIL),
        TINFLStatus::NeedsMoreInput | TINFLStatus::FailedCannotMakeProgress | TINFLStatus::Failed => Err(E_DSTORAGE_ZLIB_BAD_DATA),
        TINFLStatus::BadParam => Err(E_DSTORAGE_DECOMPRESSION_ERROR),
    }
}

pub fn read_at(file: &File, offset: u64, buf: &mut [u8]) -> Result<(), i32> {
    let mut done = 0usize;
    while done < buf.len() {
        match file.seek_read(&mut buf[done..], offset + done as u64) {
            Ok(0) => {
                let len = file.metadata().map(|m| m.len()).unwrap_or(0);
                return Err(if offset.saturating_add(buf.len() as u64) > len { E_DSTORAGE_END_OF_FILE } else { E_DSTORAGE_PASSTHROUGH_ERROR });
            }
            Ok(n) => done += n,
            Err(e) => {
                if e.raw_os_error() == Some(38) {
                    return Err(E_DSTORAGE_END_OF_FILE);
                }
                return Err(e.raw_os_error().map(|c| hresult_from_win32(c as u32)).unwrap_or(E_DSTORAGE_PASSTHROUGH_ERROR));
            }
        }
    }
    Ok(())
}

pub enum Source<'a> {
    File(&'a File, u64),
    Memory(&'a [u8]),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Outcome {
    pub written: usize,
}

fn source_bytes(src: Source<'_>, source_size: u32) -> Result<Vec<u8>, i32> {
    match src {
        Source::File(f, off) => {
            let mut b = vec![0u8; source_size as usize];
            crate::stats::timed(crate::stats::READ, b.len(), || read_at(f, off, &mut b))?;
            Ok(b)
        }
        Source::Memory(m) => {
            if m.len() < source_size as usize {
                return Err(E_DSTORAGE_ACCESS_VIOLATION);
            }
            Ok(m[..source_size as usize].to_vec())
        }
    }
}

pub fn execute_with_intermediate(compression: u8, intermediate: u32, src: Source<'_>, source_size: u32, dst: &mut [u8]) -> Result<Outcome, i32> {
    let mode = ((compression as u64 >> OPT_BCPACK_SHIFT) & OPT_BCPACK_MASK) as u8;
    if mode == bcpack::MODE_NONE {
        return execute(compression, src, source_size, dst);
    }
    let packed = source_bytes(src, source_size)?;
    let chunk = if compression as u64 & OPT_ZLIB != 0 {
        let mut mid = vec![0u8; intermediate as usize];
        let n = crate::stats::timed(crate::stats::INFLATE, mid.len(), || inflate_into(&packed, &mut mid))?;
        mid.truncate(n);
        mid
    } else {
        packed
    };
    let written = bcpack::decode(mode, &chunk, dst)?;
    Ok(Outcome { written })
}

pub fn execute(compression: u8, src: Source<'_>, source_size: u32, dst: &mut [u8]) -> Result<Outcome, i32> {
    match compression {
        COMPRESSION_NONE => {
            if dst.len() != source_size as usize {
                return Err(E_DSTORAGE_INVALID_DESTINATION_SIZE);
            }
            match src {
                Source::File(f, off) => crate::stats::timed(crate::stats::READ, dst.len(), || read_at(f, off, dst))?,
                Source::Memory(m) => {
                    if m.len() < dst.len() {
                        return Err(E_DSTORAGE_ACCESS_VIOLATION);
                    }
                    let n = dst.len();
                    crate::stats::timed(crate::stats::COPY, n, || dst.copy_from_slice(&m[..n]));
                }
            }
            Ok(Outcome { written: dst.len() })
        }
        COMPRESSION_ZLIB => {
            let written = match src {
                Source::File(f, off) => {
                    let mut cbuf = vec![0u8; source_size as usize];
                    crate::stats::timed(crate::stats::READ, cbuf.len(), || read_at(f, off, &mut cbuf))?;
                    let n = dst.len();
                    crate::stats::timed(crate::stats::INFLATE, n, || inflate_into(&cbuf, dst))?
                }
                Source::Memory(m) => {
                    let n = dst.len();
                    crate::stats::timed(crate::stats::INFLATE, n, || inflate_into(&m[..(source_size as usize).min(m.len())], dst))?
                }
            };
            Ok(Outcome { written })
        }
        _ => Err(E_DSTORAGE_DECOMPRESSION_ERROR),
    }
}
