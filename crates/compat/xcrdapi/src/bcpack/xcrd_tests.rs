use super::bc1::{self, QuadTable, Window};
use super::encode;
use crate::abi::*;
use crate::request::{execute_with_intermediate, Source};

fn all_fitted() -> (Window, QuadTable) {
    let mut w = Window::default_table();
    for s in 0..256 {
        if !w.fitted[s] {
            w.slots[s] = [1, 2, 3, 4, 5, 6];
            w.fitted[s] = true;
        }
    }
    (w, QuadTable::default_table())
}

fn without_prestart(seed: u64, blocks: usize) -> Vec<u8> {
    for s in seed.. {
        let (stream, syms) = encode::synthetic_bc1(s, blocks, 300);
        let pre = (0..blocks).any(|i| {
            let d = bc1::back_distance(syms[0][i]);
            d > i
        });
        if !pre {
            return stream;
        }
    }
    unreachable!()
}

#[test]
fn bc1_request_matches_the_upstream_block_assembly() {
    let blocks = 512;
    let (stream, _) = encode::synthetic_bc1(11, blocks, 400);
    let (w, q) = all_fitted();
    let want = bc1::decode(&stream, blocks, &w, &q).unwrap().blocks;
    let mut dst = vec![0u8; blocks * 8];
    assert_eq!(super::decode_bc1_with(&stream, &mut dst, &w, &q), Ok(blocks * 8));
    assert_eq!(dst, want);
}

#[test]
fn chunks_reaching_an_unfitted_window_slot_fail_instead_of_guessing() {
    let blocks = 256;
    let (stream, syms) = encode::synthetic_bc1(5, blocks, 300);
    let w = Window::default_table();
    let q = QuadTable::default_table();
    let tainted = bc1::decode(&stream, blocks, &w, &q).unwrap().unfitted_blocks;
    let mut dst = vec![0xAAu8; blocks * 8];
    let hr = super::decode_bc1_with(&stream, &mut dst, &w, &q);
    if tainted > 0 {
        assert_eq!(hr, Err(E_DSTORAGE_DECOMPRESSION_ERROR));
        assert!(dst.iter().all(|&b| b == 0xAA), "nothing written on failure");
    } else {
        assert_eq!(hr, Ok(blocks * 8));
    }
    assert!(syms[0].len() == blocks);
    let mut none = Window::default_table();
    none.fitted = [false; 256];
    let (s2, _) = encode::synthetic_bc1(5, blocks, 300);
    let pre = bc1::decode(&s2, blocks, &none, &q).unwrap();
    if pre.prestart_refs > 0 {
        assert_eq!(super::decode_bc1_with(&s2, &mut dst, &none, &q), Err(E_DSTORAGE_DECOMPRESSION_ERROR));
    }
}

#[test]
fn bc1_and_zlib_plus_bc1_requests_run_through_execute() {
    let blocks = 1024;
    let stream = without_prestart(21, blocks);
    let (w, q) = (Window::default_table(), QuadTable::default_table());
    let want = bc1::decode(&stream, blocks, &w, &q).unwrap();
    assert_eq!(want.unfitted_blocks, 0);
    let mut dst = vec![0u8; blocks * 8];
    let comp = (bcpack_opt(1)) as u8;
    let out = execute_with_intermediate(comp, 0, Source::Memory(&stream), stream.len() as u32, &mut dst).unwrap();
    assert_eq!(out.written, blocks * 8);
    assert_eq!(dst, want.blocks);
    let z = miniz_oxide::deflate::compress_to_vec_zlib(&stream, 6);
    let mut dst2 = vec![0u8; blocks * 8];
    let comp = (OPT_ZLIB | bcpack_opt(1)) as u8;
    let out = execute_with_intermediate(comp, stream.len() as u32, Source::Memory(&z), z.len() as u32, &mut dst2).unwrap();
    assert_eq!(out.written, blocks * 8);
    assert_eq!(dst2, want.blocks, "DEFLATE then BCPack, as the docs order them");
}

#[test]
fn bcpack_modes_without_a_public_mapping_fail() {
    let mut dst = vec![0u8; 64];
    for m in 2u64..=7 {
        let comp = bcpack_opt(m) as u8;
        assert_eq!(execute_with_intermediate(comp, 0, Source::Memory(&[0u8; 16]), 16, &mut dst), Err(E_DSTORAGE_DECOMPRESSION_ERROR), "mode {m}");
    }
    assert_eq!(super::mode_name(6), "BC6H");
    assert!(super::decodable(1) && !super::decodable(7));
}

#[test]
fn bcpack_destination_must_be_whole_blocks() {
    let (w, q) = all_fitted();
    let mut dst = vec![0u8; 12];
    assert_eq!(super::decode_bc1_with(&[0u8; 16], &mut dst, &w, &q), Err(E_DSTORAGE_INVALID_DESTINATION_SIZE));
    let mut dst = vec![0u8; 64];
    assert_eq!(super::decode_bc1_with(&[0xffu8; 3], &mut dst, &w, &q), Err(E_DSTORAGE_DECOMPRESSION_ERROR));
}

fn bcpack_opt(mode: u64) -> u64 {
    mode << OPT_BCPACK_SHIFT
}
