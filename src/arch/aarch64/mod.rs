use uefi::status::Result;

use crate::key;

pub fn main() -> Result<()> {
    println!("Redox Bootloader WIP");
    loop {
        let key = key::key(true)?;
        println!("{:?}", key);
    }
    Ok(())
}
