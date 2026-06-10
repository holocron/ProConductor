//! macOS dock icon via objc2/AppKit.

// macOS dock icon variants — 64x64 RGBA, loaded from icons/ at compile time
#[cfg(target_os = "macos")]
pub(crate) const DOCK_ICON_W: u32 = 64;
#[cfg(target_os = "macos")]
pub(crate) const DOCK_ICON_H: u32 = 64;
#[cfg(target_os = "macos")]
pub(crate) const DOCK_ICON_GREEN: &[u8] = include_bytes!("../../icons/dock_green.rgba");
#[cfg(target_os = "macos")]
pub(crate) const DOCK_ICON_AMBER: &[u8] = include_bytes!("../../icons/dock_amber.rgba");
#[cfg(target_os = "macos")]
pub(crate) const DOCK_ICON_RED:   &[u8] = include_bytes!("../../icons/dock_red.rgba");

// ══════════════════════════════════════════════════════════════════════════════
// macOS dock icon
// ══════════════════════════════════════════════════════════════════════════════

#[cfg(target_os = "macos")]
pub(crate) fn set_dock_icon(rgba: &[u8], width: u32, height: u32) {
    use objc2_app_kit::{NSApplication, NSBitmapImageRep, NSImage};
    use objc2_foundation::{NSString, NSSize, MainThreadMarker};
    use objc2::AnyThread;

    // The copy below trusts the caller's dimensions; a short buffer would read
    // out of bounds. (bytesPerRow is pinned to width*4, so no row padding.)
    if rgba.len() != (width as usize) * (height as usize) * 4 { return; }

    unsafe {
        let color_space = NSString::from_str("NSDeviceRGBColorSpace");

        // Build NSBitmapImageRep from our RGBA buffer
        let rep = NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            width as isize,
            height as isize,
            8,
            4,
            true,
            false,
            &color_space,
            (width * 4) as isize,
            32,
        );
        if let Some(rep) = rep {
            // Copy pixel data
            let dst: *mut u8 = rep.bitmapData();
            if !dst.is_null() {
                std::ptr::copy_nonoverlapping(rgba.as_ptr(), dst, rgba.len());
            }
            // Build NSImage and set as dock icon
            let size = NSSize { width: width as f64, height: height as f64 };
            let img = NSImage::initWithSize(NSImage::alloc(), size);
            img.addRepresentation(&rep);
            // SAFETY: dock icon is set from the egui paint callback, which runs on the main thread.
            let mtm = MainThreadMarker::new_unchecked();
            NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&img));
        }
    }
}
