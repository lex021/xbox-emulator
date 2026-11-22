pub mod bc1;
pub mod rans;
pub mod tables;
#[cfg(test)]
pub mod encode;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod xcrd_tests;

use crate::abi::{E_DSTORAGE_DECOMPRESSION_ERROR, E_DSTORAGE_INVALID_DESTINATION_SIZE};
use std::sync::OnceLock;

pub const MODE_NONE: u8 = 0;
pub const MODE_BC1: u8 = 1;
pub const MODE_BC7: u8 = 7;
pub const BC1_BLOCK_BYTES: usize = 8;

pub fn mode_name(mode: u8) -> &'static str {
    match mode {
        0 => "NONE",
        1 => "BC1",
        2 => "BC2",
        3 => "BC3",
        4 => "BC4",
        5 => "BC5",
        6 => "BC6H",
        7 => "BC7",
        _ => "invalid",
    }
}

pub fn decodable(mode: u8) -> bool {
    mode == MODE_BC1
}

fn tables() -> &'static (bc1::Window, bc1::QuadTable) {
    static T: OnceLock<(bc1::Window, bc1::QuadTable)> = OnceLock::new();
    T.get_or_init(|| (bc1::Window::default_table(), bc1::QuadTable::default_table()))
}

pub fn decode_bc1_with(src: &[u8], dst: &mut [u8], window: &bc1::Window, quads: &bc1::QuadTable) -> Result<usize, i32> {
    if dst.is_empty() || dst.len() % BC1_BLOCK_BYTES != 0 {
        return Err(E_DSTORAGE_INVALID_DESTINATION_SIZE);
    }
    let blocks = dst.len() / BC1_BLOCK_BYTES;
    let out = match bc1::decode(src, blocks, window, quads) {
        Ok(o) => o,
        Err(e) => {
            compat_common::record_raw("xcrdapi", "BCPack", &format!("BC1 chunk of {} bytes for {blocks} blocks does not decode: {e}", src.len()));
            return Err(E_DSTORAGE_DECOMPRESSION_ERROR);
        }
    };
    if out.unfitted_blocks != 0 {
        compat_common::record_raw(
            "xcrdapi",
            "BCPack",
            &format!("BC1 chunk of {blocks} blocks: {} blocks reach an unfitted preset window slot (0 or 4); request failed instead of emitting guessed endpoints", out.unfitted_blocks),
        );
        return Err(E_DSTORAGE_DECOMPRESSION_ERROR);
    }
    dst.copy_from_slice(&out.blocks);
    Ok(dst.len())
}

pub fn decode(mode: u8, src: &[u8], dst: &mut [u8]) -> Result<usize, i32> {
    match mode {
        MODE_BC1 => {
            let (w, q) = tables();
            decode_bc1_with(src, dst, w, q)
        }
        m => {
            compat_common::record_raw("xcrdapi", "BCPack", &format!("BCPack mode {m} ({}) has no public block mapping; request failed", mode_name(m)));
            Err(E_DSTORAGE_DECOMPRESSION_ERROR)
        }
    }
}
