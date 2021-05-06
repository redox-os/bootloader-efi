use core::ptr;
use x86::{
    controlregs::{self, Cr0, Cr4},
    msr,
};

static PT_BASE: u64 = 0x70000;
static N_PDP: u64 = 6;

pub unsafe fn paging() {
    // Zero PML4, PDP, and 4 PD
    ptr::write_bytes(PT_BASE as *mut u8, 0, 6 * 4096);

    let mut base = PT_BASE;

    // Link first user and first kernel PML4 to PDP
    ptr::write(base as *mut u64, (PT_BASE + 0x1000) | 1 << 1 | 1);
    ptr::write((base + 256 * 8) as *mut u64, (PT_BASE + 0x1000) | 1 << 1 | 1);
    // Link last PML4 to PML4 for recursive compatibility
    ptr::write((base + 511 * 8) as *mut u64, PT_BASE | 1 << 1 | 1);

    // Move to PDP
    base += 4096;

    // Link first six PDP to PD
    // Six so we can map some memory at 0x140000000, and a bit above
    for i in 0..N_PDP {
        ptr::write(
            (base + i * 8) as *mut u64,
            (PT_BASE + 0x2000 + i * 0x1000) | 1 << 1 | 1,
        );
    }

    // Move to PD
    base += 4096;

    // Link all PD's (512 per PDP, 2MB each)
    let mut entry = 1 << 7 | 1 << 1 | 1;
    for i in 0..N_PDP * 512 {
        ptr::write((base + i * 8) as *mut u64, entry);
        entry += 0x200000;
    }

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
    controlregs::cr3_write(PT_BASE);

    // Enable paging, write protect kernel, protected mode
    let mut cr0 = controlregs::cr0();
    cr0 |= Cr0::CR0_ENABLE_PAGING | Cr0::CR0_WRITE_PROTECT | Cr0::CR0_PROTECTED_MODE;
    controlregs::cr0_write(cr0);
}
