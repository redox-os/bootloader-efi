#![no_std]
#![no_main]
#![cfg_attr(target_arch = "aarch64", feature(asm))]
#![cfg_attr(target_arch = "x86_64", feature(llvm_asm))]
#![feature(core_intrinsics)]
#![feature(control_flow_enum)]
#![feature(prelude_import)]
#![feature(try_trait_v2)]
#![feature(untagged_unions)]

#[macro_use]
extern crate uefi_std as std;

use core::ops::Try;
use core::ptr;
use uefi::reset::ResetType;
use uefi::status::{Result, Status};

mod arch;
mod display;
pub mod image;
mod key;
pub mod null;
mod redoxfs;
pub mod text;

fn set_max_mode(output: &uefi::text::TextOutput) -> Result<()> {
    let mut max_i = None;
    let mut max_w = 0;
    let mut max_h = 0;

    for i in 0..output.Mode.MaxMode as usize {
        let mut w = 0;
        let mut h = 0;
        if (output.QueryMode)(output, i, &mut w, &mut h).branch().is_continue() {
            if w >= max_w && h >= max_h {
                max_i = Some(i);
                max_w = w;
                max_h = h;
            }
        }
    }

    if let Some(i) = max_i {
        (output.SetMode)(output, i)?;
    }

    Ok(())
}

#[no_mangle]
pub extern "C" fn main() -> Status {
    let uefi = std::system_table();

    let _ = (uefi.BootServices.SetWatchdogTimer)(0, 0, 0, ptr::null());

    if let Err(err) = set_max_mode(uefi.ConsoleOut) {
        println!("Failed to set max mode: {:?}", err);
    }

    if let Err(err) = arch::main() {
        println!("App error: {:?}", err);
        let _ = key::key(true);
    }

    (uefi.RuntimeServices.ResetSystem)(ResetType::Cold, Status(0), 0, ptr::null());
}
