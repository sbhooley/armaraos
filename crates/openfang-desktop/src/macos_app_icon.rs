//! Apply a Tahoe-specific Dock icon override.
//!
//! On macOS 26+ ("Tahoe"), the system may synthesize a layered Dock treatment
//! for legacy app icons. We only override the icon on Tahoe so the running app
//! keeps the same rounded black/starfish look as the bundled icon.

#[cfg(target_os = "macos")]
pub fn apply_tahoe_icon_image() {
    use objc::runtime::Class;
    use objc::runtime::Object;
    use objc::{msg_send, sel, sel_impl};

    #[repr(C)]
    struct NSOperatingSystemVersion {
        major_version: isize,
        minor_version: isize,
        patch_version: isize,
    }

    const PNG: &[u8] = include_bytes!("../icons/icon-tahoe-rounded.png");

    unsafe fn is_tahoe_or_newer() -> bool {
        let ns_process_info = match Class::get("NSProcessInfo") {
            Some(c) => c,
            None => return false,
        };
        let process_info: *mut Object = msg_send![ns_process_info, processInfo];
        if process_info.is_null() {
            return false;
        }
        let v: NSOperatingSystemVersion = msg_send![process_info, operatingSystemVersion];
        v.major_version >= 26
    }

    unsafe {
        if !is_tahoe_or_newer() {
            return;
        }

        let ns_data = match Class::get("NSData") {
            Some(c) => c,
            None => return,
        };
        let data: *mut Object = msg_send![ns_data, dataWithBytes: PNG.as_ptr() length: PNG.len()];
        if data.is_null() {
            return;
        }

        let ns_image = match Class::get("NSImage") {
            Some(c) => c,
            None => return,
        };
        let alloc: *mut Object = msg_send![ns_image, alloc];
        let img: *mut Object = msg_send![alloc, initWithData: data];
        if img.is_null() {
            return;
        }

        let ns_app = match Class::get("NSApplication") {
            Some(c) => c,
            None => return,
        };
        let app: *mut Object = msg_send![ns_app, sharedApplication];
        if app.is_null() {
            return;
        }
        let _: () = msg_send![app, setApplicationIconImage: img];
    }
}

#[cfg(not(target_os = "macos"))]
pub fn apply_tahoe_icon_image() {}
