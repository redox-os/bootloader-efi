use core::{mem, ptr};
use orbclient::{Color, Renderer};
use uefi::memory::{MemoryDescriptor, MemoryType};
use uefi::status::Result;
use x86::controlregs;

use display::{Display, Output};
use fs::load;
use image::{self, Image};
use io::wait_key;
use proto::Protocol;
use text::TextDisplay;

static KERNEL: &'static str = concat!("\\", env!("BASEDIR"), "\\res\\kernel");
static SPLASHBMP: &'static str = concat!("\\", env!("BASEDIR"), "\\res\\splash.bmp");

static MM_BASE: u64 = 0x500;
static MM_SIZE: u64 = 0x4B00;
static VBE_BASE: u64 = 0x5200;
static PT_BASE: u64 = 0x70000;
static KERNEL_BASE: u64 = 0x100000;
static mut KERNEL_SIZE: u64 = 0;
static mut KERNEL_ENTRY: u64 = 0;
static STACK_BASE: u64 = 0xFFFFFF0000080000;
static STACK_SIZE: u64 = 0x1F000;

/// The info of the VBE mode
#[derive(Copy, Clone, Default, Debug)]
#[repr(packed)]
pub struct VBEModeInfo {
    attributes: u16,
    win_a: u8,
    win_b: u8,
    granularity: u16,
    winsize: u16,
    segment_a: u16,
    segment_b: u16,
    winfuncptr: u32,
    bytesperscanline: u16,
    pub xresolution: u16,
    pub yresolution: u16,
    xcharsize: u8,
    ycharsize: u8,
    numberofplanes: u8,
    bitsperpixel: u8,
    numberofbanks: u8,
    memorymodel: u8,
    banksize: u8,
    numberofimagepages: u8,
    unused: u8,
    redmasksize: u8,
    redfieldposition: u8,
    greenmasksize: u8,
    greenfieldposition: u8,
    bluemasksize: u8,
    bluefieldposition: u8,
    rsvdmasksize: u8,
    rsvdfieldposition: u8,
    directcolormodeinfo: u8,
    pub physbaseptr: u32,
    offscreenmemoryoffset: u32,
    offscreenmemsize: u16,
}

/// Memory does not exist
pub const MEMORY_AREA_NULL: u32 = 0;

/// Memory is free to use
pub const MEMORY_AREA_FREE: u32 = 1;

/// Memory is reserved
pub const MEMORY_AREA_RESERVED: u32 = 2;

/// Memory is used by ACPI, and can be reclaimed
pub const MEMORY_AREA_ACPI: u32 = 3;

/// A memory map area
#[derive(Copy, Clone, Debug, Default)]
#[repr(packed)]
pub struct MemoryArea {
    pub base_addr: u64,
    pub length: u64,
    pub _type: u32,
    pub acpi: u32
}

#[repr(packed)]
pub struct KernelArgs {
    kernel_base: u64,
    kernel_size: u64,
    stack_base: u64,
    stack_size: u64,
    env_base: u64,
    env_size: u64,
}

unsafe fn vesa() {
    let mut mode_info = VBEModeInfo::default();

    if let Ok(output) = Output::one() {
        mode_info.xresolution = output.0.Mode.Info.HorizontalResolution as u16;
        mode_info.yresolution = output.0.Mode.Info.VerticalResolution as u16;
        mode_info.physbaseptr = output.0.Mode.FrameBufferBase as u32;
    }

    ptr::write(VBE_BASE as *mut VBEModeInfo, mode_info);
}

unsafe fn memory_map() -> usize {
    let uefi = unsafe { &mut *::UEFI };

    ptr::write_bytes(MM_BASE as *mut u8, 0, MM_SIZE as usize);

    let mut map: [u8; 65536] = [0; 65536];
    let mut map_size = map.len();
    let mut map_key = 0;
    let mut descriptor_size = 0;
    let mut descriptor_version = 0;
    let _ = (uefi.BootServices.GetMemoryMap)(
        &mut map_size,
        map.as_mut_ptr() as *mut MemoryDescriptor,
        &mut map_key,
        &mut descriptor_size,
        &mut descriptor_version
    );

    if descriptor_size >= mem::size_of::<MemoryDescriptor>() {
        for i in 0..map_size/descriptor_size {
            let descriptor_ptr = map.as_ptr().offset((i * descriptor_size) as isize);
            let descriptor = & *(descriptor_ptr as *const MemoryDescriptor);
            let descriptor_type: MemoryType = mem::transmute(descriptor.Type);

            let bios_type = match descriptor_type {
                MemoryType::EfiLoaderCode |
                MemoryType::EfiLoaderData |
                MemoryType::EfiBootServicesCode |
                MemoryType::EfiBootServicesData |
                MemoryType::EfiConventionalMemory => {
                    MEMORY_AREA_FREE
                },
                _ => {
                    MEMORY_AREA_RESERVED
                }
            };

            let bios_area = MemoryArea {
                base_addr: descriptor.PhysicalStart.0,
                length: descriptor.NumberOfPages * 4096,
                _type: bios_type,
                acpi: 0,
            };

            println!("{}: {:?}", i, bios_area);

            ptr::write((MM_BASE as *mut MemoryArea).offset(i as isize), bios_area);
        }
    } else {
        println!("Unknown memory descriptor size: {}", descriptor_size);
    }

    map_key
}

unsafe fn exit_boot_services(key: usize) {
    let handle = ::HANDLE;
    let uefi = &mut *::UEFI;

    let _ = (uefi.BootServices.ExitBootServices)(handle, key);
}

unsafe fn paging() {
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

    // Enable FXSAVE/FXRSTOR, Page Global, Page Address Extension, and Page Size Extension
    let mut cr4 = controlregs::cr4();
    cr4 |= 1 << 9 | 1 << 7 | 1 << 5 | 1 << 4;
    controlregs::cr4_write(cr4);

    // Set new page map
    controlregs::cr3_write(PT_BASE);
}

unsafe fn enter() -> ! {
    let args = KernelArgs {
        kernel_base: KERNEL_BASE,
        kernel_size: KERNEL_SIZE,
        stack_base: STACK_BASE,
        stack_size: STACK_SIZE,
        env_base: STACK_BASE + STACK_SIZE,
        env_size: 0,
    };

    let entry_fn: extern "C" fn(args_ptr: *const KernelArgs) -> ! = mem::transmute(KERNEL_ENTRY);
    entry_fn(&args);
}

fn inner() -> Result<()> {
    {
        println!("Loading Kernel...");
        unsafe {
            let data = load(KERNEL)?;
            KERNEL_SIZE = data.len() as u64;
            println!("  Size: {}", KERNEL_SIZE);
            KERNEL_ENTRY = *(data.as_ptr().offset(0x18) as *const u64);
            println!("  Entry: {:X}", KERNEL_ENTRY);
            ptr::copy(data.as_ptr(), KERNEL_BASE as *mut u8, data.len());
        }
        println!("  Done");
    }

    unsafe {
        vesa();
    }

    unsafe {
        let key = memory_map();
        exit_boot_services(key);
    }

    unsafe {
        paging();
    }

    unsafe {
        asm!("mov rsp, $0" : : "r"(STACK_BASE + STACK_SIZE) : "memory" : "intel", "volatile");
        enter();
    }
}

pub fn main() -> Result<()> {
    let uefi = unsafe { &mut *::UEFI };

    let mut display = {
        let output = Output::one()?;

        let mut max_i = 0;
        let mut max_w = 0;
        let mut max_h = 0;

        for i in 0..output.0.Mode.MaxMode {
            let mut mode_ptr = ::core::ptr::null_mut();
            let mut mode_size = 0;
            (output.0.QueryMode)(output.0, i, &mut mode_size, &mut mode_ptr)?;

            let mode = unsafe { &mut *mode_ptr };
            let w = mode.HorizontalResolution;
            let h = mode.VerticalResolution;

            println!("{}: {}x{}", i, w, h);

            if w >= max_w && w <= 1024 && h >= max_h && h <= 768 {
                max_i = i;
                max_w = w;
                max_h = h;
            }
        }

        let _ = (output.0.SetMode)(output.0, max_i);

        Display::new(output)
    };

    let mut splash = Image::new(0, 0);
    {
        println!("Loading Splash...");
        if let Ok(data) = load(SPLASHBMP) {
            if let Ok(image) = image::bmp::parse(&data) {
                splash = image;
            }
        }
        println!(" Done");
    }

    {
        let bg = Color::rgb(0x5f, 0xaf, 0xff);

        display.set(bg);

        {
            let x = (display.width() as i32 - splash.width() as i32)/2;
            let y = 16;
            splash.draw(&mut display, x, y);
        }

        {
            let prompt = concat!("Redox Bootloader ", env!("CARGO_PKG_VERSION"));
            let mut x = (display.width() as i32 - prompt.len() as i32 * 8)/2;
            let y = display.height() as i32 - 32;
            for c in prompt.chars() {
                display.char(x, y, c, Color::rgb(0xff, 0xff, 0xff));
                x += 8;
            }
        }

        display.sync();
    }

    {
        let cols = 80;
        let off_x = (display.width() as i32 - cols as i32 * 8)/2;
        let off_y = 16 + splash.height() as i32 + 16;
        let rows = (display.height() as i32 - 64 - off_y - 1) as usize/16;
        display.rect(off_x, off_y, cols as u32 * 8, rows as u32 * 16, Color::rgb(0, 0, 0));
        display.sync();

        let mut text = TextDisplay::new(&mut display);
        text.off_x = off_x;
        text.off_y = off_y;
        text.cols = cols;
        text.rows = rows;
        text.pipe(inner)?;
    }

    Ok(())
}
