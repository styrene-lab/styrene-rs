#![no_std]
#![forbid(unsafe_code)]

// Target-side std is intentionally required. No dependency/build-script error
// can stand in for the expected rustc E0463 diagnostic from this exact source.
extern crate std;

pub fn needs_target_std() -> std::vec::Vec<u8> {
    std::vec::Vec::new()
}
