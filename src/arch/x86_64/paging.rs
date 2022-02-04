use core::{ptr, slice};
use x86::{
    controlregs::{self, Cr0, Cr4},
    msr,
};
use uefi::memory::MemoryType;
use uefi::status::Result;

unsafe fn paging_allocate() -> Result<&'static mut [u64]> {
    let ptr = super::allocate_zero_pages(1)?;

    Ok(slice::from_raw_parts_mut(
        ptr as *mut u64,
        512 // page size divided by u64 size
    ))
}

pub unsafe fn paging_create(kernel_phys: u64, kernel_size: u64) -> Result<u64> {
    let uefi = std::system_table();

    let pdp_count = 6;
    let page_phys = unsafe {
        let mut ptr = 0;
        (uefi.BootServices.AllocatePages)(
            0, // AllocateAnyPages
            MemoryType::EfiRuntimeServicesData, // Reserves kernel memory
            2 + pdp_count as usize,
            &mut ptr
        )?;
        ptr as u64
    };


    // Zero PML4, PDP, and 4 PD
    ptr::write_bytes(page_phys as *mut u8, 0, (2 + pdp_count as usize) * 4096);

    let mut base = page_phys;

    // Link first user and first kernel PML4 to PDP
    ptr::write(base as *mut u64, (page_phys + 0x1000) | 1 << 1 | 1);
    ptr::write((base + 256 * 8) as *mut u64, (page_phys + 0x1000) | 1 << 1 | 1);
    // Link last PML4 to PML4 for recursive compatibility
    ptr::write((base + 511 * 8) as *mut u64, page_phys | 1 << 1 | 1);

    // Move to PDP
    base += 4096;

    // Link first six PDP to PD
    // Six so we can map some memory at 0x140000000, and a bit above
    for i in 0..pdp_count {
        ptr::write(
            (base + i * 8) as *mut u64,
            (page_phys + 0x2000 + i * 0x1000) | 1 << 1 | 1,
        );
    }

    // Move to PD
    base += 4096;

    // Link all PD's (512 per PDP, 2MB each)
    let mut entry = 1 << 7 | 1 << 1 | 1;
    for i in 0..pdp_count * 512 {
        ptr::write((base + i * 8) as *mut u64, entry);
        entry += 0x200000;
    }

    Ok(page_phys)
}

pub unsafe fn paging_enter(page_phys: u64) {
    // Enable OSXSAVE, FXSAVE/FXRSTOR, Page Global, Page Address Extension, and Page Size Extension
    let mut cr4 = controlregs::cr4();
    cr4 |= Cr4::CR4_ENABLE_OS_XSAVE
        | Cr4::CR4_ENABLE_SSE
        | Cr4::CR4_ENABLE_GLOBAL_PAGES
        | Cr4::CR4_ENABLE_PAE
        | Cr4::CR4_ENABLE_PSE;
    controlregs::cr4_write(cr4);

    // Enable Long mode and NX bit
    let mut efer = msr::rdmsr(msr::IA32_EFER);
    efer |= 1 << 11 | 1 << 8;
    msr::wrmsr(msr::IA32_EFER, efer);

    // Set new page map
    controlregs::cr3_write(page_phys);

    // Enable paging, write protect kernel, protected mode
    let mut cr0 = controlregs::cr0();
    cr0 |= Cr0::CR0_ENABLE_PAGING | Cr0::CR0_WRITE_PROTECT | Cr0::CR0_PROTECTED_MODE;
    controlregs::cr0_write(cr0);
}
