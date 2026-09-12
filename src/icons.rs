use std::ffi::c_void;

use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, GetDC, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER,
    DIB_RGB_COLORS,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, HICON, ICONINFO,
};

pub struct IconSet {
    pub green: HICON,
    pub red: HICON,
    pub gray: HICON,
}

unsafe impl Send for IconSet {}
unsafe impl Sync for IconSet {}

impl IconSet {
    pub fn new() -> IconSet {
        IconSet {
            green: make_circle_icon((46, 160, 67)),
            red: make_circle_icon((217, 48, 37)),
            gray: make_circle_icon((130, 130, 130)),
        }
    }
}

impl Drop for IconSet {
    fn drop(&mut self) {
        unsafe {
            DestroyIcon(self.green);
            DestroyIcon(self.red);
            DestroyIcon(self.gray);
        }
    }
}

fn make_circle_icon(rgb: (u8, u8, u8)) -> HICON {
    const S: i32 = 32;
    unsafe {
        let hdc = GetDC(std::ptr::null_mut());
        if hdc.is_null() {
            return std::ptr::null_mut();
        }
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = S;
        bmi.bmiHeader.biHeight = -S;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = 0; // BI_RGB
        let mut bits: *mut c_void = std::ptr::null_mut();
        let hbmp = CreateDIBSection(hdc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if hbmp.is_null() || bits.is_null() {
            ReleaseDC(std::ptr::null_mut(), hdc);
            return std::ptr::null_mut();
        }
        let px = bits as *mut u32;
        let cx = 15.5f64;
        let cy = 15.5f64;
        let r = 12.0f64;
        for y in 0..S {
            for x in 0..S {
                let dx = x as f64 - cx;
                let dy = y as f64 - cy;
                let idx = (y * S + x) as usize;
                if dx * dx + dy * dy <= r * r {
                    *px.add(idx) = 0xFF000000
                        | ((rgb.2 as u32) << 16)
                        | ((rgb.1 as u32) << 8)
                        | (rgb.0 as u32);
                } else {
                    *px.add(idx) = 0;
                }
            }
        }
        let mask = CreateBitmap(S, S, 1, 1, std::ptr::null());
        let mut ii: ICONINFO = std::mem::zeroed();
        ii.fIcon = 1;
        ii.hbmColor = hbmp;
        ii.hbmMask = mask;
        let icon = CreateIconIndirect(&ii);
        DeleteObject(hbmp);
        DeleteObject(mask);
        ReleaseDC(std::ptr::null_mut(), hdc);
        icon
    }
}