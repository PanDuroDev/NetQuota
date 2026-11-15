//! Extracts an executable's icon through the Windows shell and encodes it as a
//! PNG so the UI can cache it. No external image service is involved.

use std::ffi::c_void;

use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};

pub fn icon_png(path: &str) -> Option<Vec<u8>> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let mut info = SHFILEINFOW::default();
        let ok = SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        );
        if ok == 0 || info.hIcon.is_invalid() {
            return None;
        }
        let png = render(info.hIcon);
        let _ = DestroyIcon(info.hIcon);
        png
    }
}

unsafe fn render(icon: windows::Win32::UI::WindowsAndMessaging::HICON) -> Option<Vec<u8>> {
    let mut icon_info = ICONINFO::default();
    GetIconInfo(icon, &mut icon_info).ok()?;
    let color = icon_info.hbmColor;

    let mut bitmap = BITMAP::default();
    if GetObjectW(
        HGDIOBJ(color.0),
        std::mem::size_of::<BITMAP>() as i32,
        Some(&mut bitmap as *mut BITMAP as *mut c_void),
    ) == 0
    {
        let _ = DeleteObject(HGDIOBJ(color.0));
        let _ = DeleteObject(HGDIOBJ(icon_info.hbmMask.0));
        return None;
    }

    let width = bitmap.bmWidth.clamp(1, 256);
    let height = bitmap.bmHeight.clamp(1, 256);
    let mut header = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // Negative height requests a top-down buffer.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let hdc = GetDC(None);
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let lines = GetDIBits(
        hdc,
        color,
        0,
        height as u32,
        Some(pixels.as_mut_ptr() as *mut c_void),
        &mut header,
        DIB_RGB_COLORS,
    );
    ReleaseDC(None, hdc);
    let _ = DeleteObject(HGDIOBJ(color.0));
    let _ = DeleteObject(HGDIOBJ(icon_info.hbmMask.0));
    if lines == 0 {
        return None;
    }

    let has_alpha = pixels.chunks_exact(4).any(|p| p[3] != 0);
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2); // BGRA -> RGBA
        if !has_alpha && (pixel[0] | pixel[1] | pixel[2]) != 0 {
            pixel[3] = 255;
        }
    }

    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&pixels).ok()?;
    }
    Some(png)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_a_png_for_the_current_executable() {
        let exe = std::env::current_exe().expect("test binary path");
        let png = icon_png(&exe.to_string_lossy()).expect("test binary has an icon");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn missing_path_returns_none() {
        assert!(icon_png("C:\\definitely\\not\\here.exe").is_none());
    }
}
