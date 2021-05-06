use orbclient::{Color, Renderer};
use std::proto::Protocol;
use uefi::status::Result;

use crate::display::{Display, ScaledDisplay, Output};
use crate::image::{self, Image};
use crate::key::{key, Key};
use crate::text::TextDisplay;

static SPLASHBMP: &'static [u8] = include_bytes!("../../../res/splash.bmp");

pub fn inner() -> Result<()> {
    println!("Redox Bootloader WIP");
    loop {
        let key = key(true)?;
        println!("{:?}", key);
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

        /* TODO
        let mode = pretty_pipe(&splash, || {
            select_mode(&mut output)
        })?;
        (output.0.SetMode)(output.0, mode)?;
        */

        pretty_pipe(&splash, inner)?;
    } else {
        inner()?;
    }

    Ok(())
}
