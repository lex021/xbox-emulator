#![allow(non_snake_case, clippy::missing_safety_doc)]

pub mod sys;
pub mod imports;
pub mod com;
pub mod messaging;
pub mod titles;
pub mod system;
pub mod process;
pub mod debugaction;
pub mod entry;
pub mod monitor;
pub mod xmemi;
pub mod titlememory;
pub mod processdelta;
pub mod offline;
pub mod conndiag;
pub mod trace;
pub mod netstate;

#[cfg(test)]
mod tests;

pub type OperationFn = unsafe extern "system" fn(usize, usize, usize) -> sys::HRESULT;

pub use titles::{is_title_process, register_title_process, unregister_title_process};
