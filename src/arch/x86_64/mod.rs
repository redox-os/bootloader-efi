use core::{mem, ptr};
use orbclient::{Color, Renderer};
use std::fs::find;
use std::proto::Protocol;
use uefi::status::Result;
use uefi::guid::GuidKind;

use crate::display::{Display, ScaledDisplay, Output};
use crate::image::{self, Image};
use crate::key::{key, Key};
use crate::redoxfs;
use crate::text::TextDisplay;

use self::memory_map::memory_map;
use self::paging::paging;
use self::vesa::vesa;

mod memory_map;
mod paging;
mod partitions;
mod vesa;

static KERNEL: &'static str = concat!("\\", env!("BASEDIR"), "\\kernel");
static SPLASHBMP: &'static [u8] = include_bytes!("../../../res/splash.bmp");

static PHYSICAL_OFFSET: u64 = 0xFFFF800000000000;

static KERNEL_PHYSICAL: u64 = 0x100000;
static mut KERNEL_SIZE: u64 = 0;
static mut KERNEL_ENTRY: u64 = 0;

static STACK_PHYSICAL: u64 = 0x80000;
static STACK_VIRTUAL: u64 = STACK_PHYSICAL + PHYSICAL_OFFSET;
static STACK_SIZE: u64 = 0x1F000;

static mut ENV_SIZE: u64 = 0x0;

static mut RSDPS_AREA: Option<Vec<u8>> = None;

#[repr(packed)]
pub struct KernelArgs {
    kernel_base: u64,
    kernel_size: u64,
    stack_base: u64,
    stack_size: u64,
    env_base: u64,
    env_size: u64,

    acpi_rsdps_base: u64,
    acpi_rsdps_size: u64,
}

unsafe fn exit_boot_services(key: usize) {
    let handle = std::handle();
    let uefi = std::system_table();

    let _ = (uefi.BootServices.ExitBootServices)(handle, key);
}

unsafe fn enter() -> ! {
    let args = KernelArgs {
        kernel_base: KERNEL_PHYSICAL,
        kernel_size: KERNEL_SIZE,
        stack_base: STACK_VIRTUAL,
        stack_size: STACK_SIZE,
        env_base: STACK_VIRTUAL,
        env_size: ENV_SIZE,

        acpi_rsdps_base: RSDPS_AREA.as_ref().map(Vec::as_ptr).unwrap_or(core::ptr::null()) as usize as u64 + PHYSICAL_OFFSET,
        acpi_rsdps_size: RSDPS_AREA.as_ref().map(Vec::len).unwrap_or(0) as u64,
    };

    let entry_fn: extern "sysv64" fn(args_ptr: *const KernelArgs) -> ! = mem::transmute(KERNEL_ENTRY);
    entry_fn(&args);
}

fn get_correct_block_io() -> Result<redoxfs::Disk> {
    // Get all BlockIo handles.
    let mut handles = vec! [uefi::Handle(0); 128];
    let mut size = handles.len() * mem::size_of::<uefi::Handle>();

    (std::system_table().BootServices.LocateHandle)(uefi::boot::LocateSearchType::ByProtocol, &uefi::guid::BLOCK_IO_GUID, 0, &mut size, handles.as_mut_ptr())?;

    let max_size = size / mem::size_of::<uefi::Handle>();
    let actual_size = std::cmp::min(handles.len(), max_size);

    // Return the handle that seems bootable.
    for handle in handles.into_iter().take(actual_size) {
        let block_io = redoxfs::Disk::handle_protocol(handle)?;
        if !block_io.0.Media.LogicalPartition {
            continue;
        }

        let part = partitions::PartitionProto::handle_protocol(handle)?.0;
        if part.sys == 1 {
            continue;
        }
        assert_eq!({part.rev}, partitions::PARTITION_INFO_PROTOCOL_REVISION);
        if part.ty == partitions::PartitionProtoDataTy::Gpt as u32 {
            let gpt = unsafe { part.info.gpt };
            assert_ne!(gpt.part_ty_guid, partitions::ESP_GUID, "detected esp partition again");
            if gpt.part_ty_guid == partitions::REDOX_FS_GUID || gpt.part_ty_guid == partitions::LINUX_FS_GUID {
                return Ok(block_io);
            }
        } else if part.ty == partitions::PartitionProtoDataTy::Mbr as u32 {
            let mbr = unsafe { part.info.mbr };
            if mbr.ty == 0x83 {
                return Ok(block_io);
            }
        } else {
            continue;
        }
    }
    panic!("Couldn't find handle for partition");
}

struct Invalid;

fn validate_rsdp(address: usize, v2: bool) -> core::result::Result<usize, Invalid> {
    #[repr(packed)]
    #[derive(Clone, Copy, Debug)]
    struct Rsdp {
        signature: [u8; 8], // b"RSD PTR "
        chksum: u8,
        oem_id: [u8; 6],
        revision: u8,
        rsdt_addr: u32,
        // the following fields are only available for ACPI 2.0, and are reserved otherwise
        length: u32,
        xsdt_addr: u64,
        extended_chksum: u8,
        _rsvd: [u8; 3],
    }
    // paging is not enabled at this stage; we can just read the physical address here.
    let rsdp_bytes = unsafe { core::slice::from_raw_parts(address as *const u8, core::mem::size_of::<Rsdp>()) };
    let rsdp = unsafe { (rsdp_bytes.as_ptr() as *const Rsdp).as_ref::<'static>().unwrap() };

    println!("RSDP: {:?}", rsdp);

    if rsdp.signature != *b"RSD PTR " {
        return Err(Invalid);
    }
    let mut base_sum = 0u8;
    for base_byte in &rsdp_bytes[..20] {
        base_sum = base_sum.wrapping_add(*base_byte);
    }
    if base_sum != 0 {
        return Err(Invalid);
    }

    if rsdp.revision == 2 {
        let mut extended_sum = 0u8;
        for byte in rsdp_bytes {
            extended_sum = extended_sum.wrapping_add(*byte);
        }

        if extended_sum != 0 {
            return Err(Invalid);
        }
    }

    let length = if rsdp.revision == 2 { rsdp.length as usize } else { core::mem::size_of::<Rsdp>() };

    Ok(length)
}

fn find_acpi_table_pointers() -> Result<()> {
    let rsdps_area = unsafe {
        RSDPS_AREA = Some(Vec::new());
        RSDPS_AREA.as_mut().unwrap()
    };

    let cfg_tables = std::system_table().config_tables();

    for (address, v2) in cfg_tables.iter().find_map(|cfg_table| if cfg_table.VendorGuid.kind() == GuidKind::Acpi { Some((cfg_table.VendorTable, false)) } else if cfg_table.VendorGuid.kind() == GuidKind::Acpi2 { Some((cfg_table.VendorTable, true)) } else { None }) {
        match validate_rsdp(address, v2) {
            Ok(length) => {
                let align = 8;

                rsdps_area.extend(&u32::to_ne_bytes(length as u32));
                rsdps_area.extend(unsafe { core::slice::from_raw_parts(address as *const u8, length) });
                rsdps_area.resize(((rsdps_area.len() + (align - 1)) / align) * align, 0u8);
            }
            Err(_) => println!("Found RSDP that wasn't valid at {:p}", address as *const u8),
        }
    }
    Ok(())
}

fn redoxfs() -> Result<redoxfs::FileSystem> {
    // TODO: Scan multiple partitions for a kernel.
    redoxfs::FileSystem::open(get_correct_block_io()?)
}

const MB: usize = 1024 * 1024;

fn inner() -> Result<()> {
    {
        println!("Loading Kernel...");
        let (kernel, mut env): (Vec<u8>, String) = if let Ok((_i, mut kernel_file)) = find(KERNEL) {
            let info = kernel_file.info()?;
            let len = info.FileSize;
            let mut kernel = Vec::with_capacity(len as usize);
            let mut buf = vec![0; 4 * MB];
            loop {
                let percent = kernel.len() as u64 * 100 / len;
                print!("\r{}% - {} MB", percent, kernel.len() / MB);

                let count = kernel_file.read(&mut buf)?;
                if count == 0 {
                    break;
                }

                kernel.extend(&buf[.. count]);
            }
            println!("");

            (kernel, String::new())
        } else {
            let mut fs = redoxfs()?;

            let root = fs.header.1.root;
            let node = fs.find_node("kernel", root)?;

            let len = fs.node_len(node.0)?;
            let mut kernel = Vec::with_capacity(len as usize);
            let mut buf = vec![0; 4 * MB];
            loop {
                let percent = kernel.len() as u64 * 100 / len;
                print!("\r{}% - {} MB", percent, kernel.len() / MB);

                let count = fs.read_node(node.0, kernel.len() as u64, &mut buf)?;
                if count == 0 {
                    break;
                }

                kernel.extend(&buf[.. count]);
            }
            println!();

            let mut env = format!("REDOXFS_BLOCK={:016x}\n", fs.block);

            env.push_str("REDOXFS_UUID=");
            for i in 0..fs.header.1.uuid.len() {
                if i == 4 || i == 6 || i == 8 || i == 10 {
                    env.push('-');
                }

                env.push_str(&format!("{:>02x}", fs.header.1.uuid[i]));
            }

            (kernel, env)
        };

        println!("Copying Kernel...");
        unsafe {
            KERNEL_SIZE = kernel.len() as u64;
            println!("Size: {}", KERNEL_SIZE);
            KERNEL_ENTRY = *(kernel.as_ptr().offset(0x18) as *const u64);
            println!("Entry: {:X}", KERNEL_ENTRY);
            ptr::copy(kernel.as_ptr(), KERNEL_PHYSICAL as *mut u8, kernel.len());
        }

        println!("Copying Environment...");
        unsafe {
            ENV_SIZE = env.len() as u64;
            println!("Size: {}", ENV_SIZE);
            println!("Data: {}", env);
            ptr::copy(env.as_ptr(), STACK_PHYSICAL as *mut u8, env.len());
        }

        println!("Parsing and writing ACPI RSDP structures.");
        find_acpi_table_pointers();

        println!("Done!");
    }

    unsafe {
        vesa();
    }

    unsafe {
        let key = memory_map();
        exit_boot_services(key);
    }

    unsafe {
        llvm_asm!("cli" : : : "memory" : "intel", "volatile");
        paging();
    }

    unsafe {
        llvm_asm!("mov rsp, $0" : : "r"(STACK_VIRTUAL + STACK_SIZE) : "memory" : "intel", "volatile");
        enter();
    }
}

fn select_mode(output: &mut Output) -> Result<u32> {
    loop {
        for i in 0..output.0.Mode.MaxMode {
            let mut mode_ptr = ::core::ptr::null_mut();
            let mut mode_size = 0;
            (output.0.QueryMode)(output.0, i, &mut mode_size, &mut mode_ptr)?;

            let mode = unsafe { &mut *mode_ptr };
            let w = mode.HorizontalResolution;
            let h = mode.VerticalResolution;

            print!("\r{}x{}: Is this OK? (y)es/(n)o", w, h);

            if key(true)? == Key::Character('y') {
                println!("");

                return Ok(i);
            }
        }
    }
}

fn pretty_pipe<T, F: FnMut() -> Result<T>>(splash: &Image, f: F) -> Result<T> {
    let mut display = Display::new(Output::one()?);

    let mut display = ScaledDisplay::new(&mut display);

    {
        let bg = Color::rgb(0x4a, 0xa3, 0xfd);

        display.set(bg);

        {
            let x = (display.width() as i32 - splash.width() as i32)/2;
            let y = 16;
            splash.draw(&mut display, x, y);
        }

        {
            let prompt = format!(
                "Redox Bootloader {} {}",
                env!("CARGO_PKG_VERSION"),
                env!("TARGET").split('-').next().unwrap_or("")
            );
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

        let mut text = TextDisplay::new(display);
        text.off_x = off_x;
        text.off_y = off_y;
        text.cols = cols;
        text.rows = rows;
        text.pipe(f)
    }
}

pub fn main() -> Result<()> {
    if let Ok(mut output) = Output::one() {
        let mut splash = Image::new(0, 0);
        {
            println!("Loading Splash...");
            if let Ok(image) = image::bmp::parse(&SPLASHBMP) {
                splash = image;
            }
            println!(" Done");
        }

        let mode = pretty_pipe(&splash, || {
            select_mode(&mut output)
        })?;
        (output.0.SetMode)(output.0, mode)?;

        pretty_pipe(&splash, inner)?;
    } else {
        inner()?;
    }

    Ok(())
}
