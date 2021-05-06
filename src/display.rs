use core::cell::Cell;
use core::ops::Try;
use core::ptr;
use orbclient::{Color, Mode, Renderer};
use std::proto::Protocol;
use uefi::graphics::{GraphicsOutput, GraphicsBltOp, GraphicsBltPixel};
use uefi::guid::{Guid, GRAPHICS_OUTPUT_PROTOCOL_GUID};

pub struct Output(pub &'static mut GraphicsOutput);

impl Protocol<GraphicsOutput> for Output {
    fn guid() -> Guid {
        GRAPHICS_OUTPUT_PROTOCOL_GUID
    }

    fn new(inner: &'static mut GraphicsOutput) -> Self {
        Output(inner)
    }
}

pub struct Display {
    output: Output,
    w: u32,
    h: u32,
    data: Box<[Color]>,
    mode: Cell<Mode>,
}

impl Display {
    pub fn new(output: Output) -> Self {
        let w = output.0.Mode.Info.HorizontalResolution;
        let h = output.0.Mode.Info.VerticalResolution;
        Self {
            output: output,
            w: w,
            h: h,
            data: vec![Color::rgb(0, 0, 0); w as usize * h as usize].into_boxed_slice(),
            mode: Cell::new(Mode::Blend),
        }
    }

    pub fn blit(&mut self, x: i32, y: i32, w: u32, h: u32) -> bool {
        let status = (self.output.0.Blt)(
            self.output.0,
            self.data.as_mut_ptr() as *mut GraphicsBltPixel,
            GraphicsBltOp::BufferToVideo,
            x as usize,
            y as usize,
            x as usize,
            y as usize,
            w as usize,
            h as usize,
            0
        );
        status.into_result().is_ok()
    }

    pub fn scroll(&mut self, rows: usize, color: Color) {
        let width = self.w as usize;
        let height = self.h as usize;
        if rows > 0 && rows < height {
            let off1 = rows * width;
            let off2 = height * width - off1;
            unsafe {
                let data_ptr = self.data.as_mut_ptr() as *mut u32;
                ptr::copy(data_ptr.offset(off1 as isize), data_ptr, off2 as usize);
                for i in off2..off2 + off1 {
                    *data_ptr.add(i) = color.data;
                }
            }
        }
    }
}

impl Renderer for Display {
    fn width(&self) -> u32 {
        self.w
    }

    fn height(&self) -> u32 {
        self.h
    }

    fn data(&self) -> &[Color] {
        &self.data
    }

    fn data_mut(&mut self) -> &mut [Color] {
        &mut self.data
    }

    fn sync(&mut self) -> bool {
        let w = self.width();
        let h = self.height();
        self.blit(0, 0, w, h)
    }

    fn mode(&self) -> &Cell<Mode> {
        &self.mode
    }
}

pub struct ScaledDisplay<'a> {
    display: &'a mut Display,
    scale: u32,
}

impl<'a> ScaledDisplay<'a> {
    pub fn new(display: &'a mut Display) -> Self {
        let scale = if display.height() > 1440 {
            2
        } else {
            1
        };

        Self {
            display,
            scale,
        }
    }

    pub fn scale(&self) -> u32 {
        self.scale
    }

    pub fn scroll(&mut self, rows: usize, color: Color) {
        let scale = self.scale as usize;
        self.display.scroll(rows * scale, color);
    }

    pub fn blit(&mut self, x: i32, y: i32, w: u32, h: u32) -> bool {
        let scale = self.scale;
        self.display.blit(
            x * scale as i32,
            y * scale as i32,
            w * scale,
            h * scale
        )
    }
}

impl<'a> Renderer for ScaledDisplay<'a> {
    fn width(&self) -> u32 {
        self.display.width()/self.scale
    }

    fn height(&self) -> u32 {
        self.display.height()/self.scale
    }

    fn data(&self) -> &[Color] {
        unreachable!()
    }

    fn data_mut(&mut self) -> &mut [Color] {
        unreachable!()
    }

    fn sync(&mut self) -> bool {
        self.display.sync()
    }

    fn pixel(&mut self, x: i32, y: i32, color: Color) {
        self.rect(x, y, 1, 1, color);
    }

    fn rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) {
        let scale = self.scale;
        self.display.rect(
            x * scale as i32,
            y * scale as i32,
            w * scale,
            h * scale,
            color
        );
    }

    fn set(&mut self, color: Color) {
        let w = self.width();
        let h = self.height();
        self.rect(0, 0, w, h, color);
    }

    fn mode(&self) -> &Cell<Mode> {
        self.display.mode()
    }
}
