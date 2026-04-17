//! macOS 26+ (Tahoe) may synthesize a layered “glass” dock icon from legacy `.icns`, adding a light
//! plate behind the artwork. Setting `-[NSApplication setApplicationIconImage:]` uses the provided
//! image directly (same class of workaround as manually re-assigning the icon in Finder → Get Info).

#[cfg(target_os = "macos")]
pub fn apply_flat_icon_image() {
    use objc::runtime::Class;
    use objc::runtime::Object;
    use objc::{msg_send, sel, sel_impl};

    const PNG: &[u8] = include_bytes!("../icons/icon.png");

    unsafe {
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
pub fn apply_flat_icon_image() {}
