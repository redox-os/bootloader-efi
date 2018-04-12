use core::{mem, ptr};
use orbclient::{Color, Renderer};
use uefi::status::Result;

use display::{Display, Output};
use fs::load;
use image::{self, Image};
use proto::Protocol;
use text::TextDisplay;

use self::memory_map::memory_map;
use self::paging::paging;
use self::vesa::vesa;

mod memory_map;
mod paging;
mod vesa;

static KERNEL: &'static str = concat!("\\", env!("BASEDIR"), "\\res\\kernel");
static SPLASHBMP: &'static str = concat!("\\", env!("BASEDIR"), "\\res\\splash.bmp");

static KERNEL_BASE: u64 = 0x100000;
static mut KERNEL_SIZE: u64 = 0;
static mut KERNEL_ENTRY: u64 = 0;
static STACK_BASE: u64 = 0xFFFFFF0000080000;
static STACK_SIZE: u64 = 0x1F000;

#[repr(packed)]
pub struct KernelArgs {
    kernel_base: u64,
    kernel_size: u64,
    stack_base: u64,
    stack_size: u64,
    env_base: u64,
    env_size: u64,
}

unsafe fn exit_boot_services(key: usize) {
    let handle = ::HANDLE;
    let uefi = &mut *::UEFI;

    let _ = (uefi.BootServices.ExitBootServices)(handle, key);
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
        asm!("cli" : : : "memory" : "intel", "volatile");
        paging();
    }

    unsafe {
        asm!("mov rsp, $0" : : "r"(STACK_BASE + STACK_SIZE) : "memory" : "intel", "volatile");
        enter();
    }
}

pub fn main() -> Result<()> {
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
