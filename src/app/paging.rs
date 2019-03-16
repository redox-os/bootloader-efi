use core::ptr;
use x86::{controlregs, msr};

static PT_BASE: u64 = 0x70000;

pub unsafe fn paging() {
    // Zero PML4, PDP, and 4 PD
    ptr::write_bytes(PT_BASE as *mut u8, 0, 6 * 4096);

    let mut base = PT_BASE;

    // Link first PML4 and second to last PML4 to PDP
    ptr::write(base as *mut u64, 0x71000 | 1 << 1 | 1);
    ptr::write((base + 510*8) as *mut u64, 0x71000 | 1 << 1 | 1);
    // Link last PML4 to PML4
    ptr::write((base + 511*8) as *mut u64, 0x70000 | 1 << 1 | 1);

    // Move to PDP
    base += 4096;

    // Link first four PDP to PD
    ptr::write(base as *mut u64, 0x72000 | 1 << 1 | 1);
    ptr::write((base + 8) as *mut u64, 0x73000 | 1 << 1 | 1);
    ptr::write((base + 16) as *mut u64, 0x74000 | 1 << 1 | 1);
    ptr::write((base + 24) as *mut u64, 0x75000 | 1 << 1 | 1);

    // Move to PD
    base += 4096;

    // Link all PD's (512 per PDP, 2MB each)
    let mut entry = 1 << 7 | 1 << 1 | 1;
    for i in 0..4*512 {
        ptr::write((base + i*8) as *mut u64, entry);
        entry += 0x200000;
    }

    // Enable OSXSAVE, FXSAVE/FXRSTOR, Page Global, Page Address Extension, and Page Size Extension
    let mut cr4 = controlregs::cr4();
    cr4 |= 1 << 18 | 1 << 9 | 1 << 7 | 1 << 5 | 1 << 4;
    controlregs::cr4_write(cr4);

    // Enable Long mode and NX bit
    let mut efer = msr::rdmsr(0xC0000080);
    efer |= 1 << 11 | 1 << 8;
    msr::wrmsr(0xC0000080, efer);

    // Set new page map
    controlregs::cr3_write(PT_BASE);

    // Enable paging, write protect kernel, protected mode
    let mut cr0 = controlregs::cr0();
    cr0 |= 1 << 31 | 1 << 16 | 1;
    controlregs::cr0_write(cr0);
}
