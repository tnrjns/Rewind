//! Putting clip files on the clipboard so they can be pasted into Discord.
//!
//! "Copy" has to mean the *file list*, not a text path: chat apps that accept a dragged
//! file ask the clipboard for `CF_HDROP` on Windows and an `NSURL` array on macOS. Text
//! would paste as a path string, which is not the same thing.

use std::path::{Path, PathBuf};

/// Copy the files themselves onto the clipboard.
pub fn copy_files(paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("Nothing to copy".to_string());
    }
    let absolute = absolute_paths(paths)?;
    #[cfg(target_os = "windows")]
    {
        windows::copy(&absolute)
    }
    #[cfg(target_os = "macos")]
    {
        macos::copy(&absolute)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = absolute;
        Err("Copying files is not supported on this platform".to_string())
    }
}

/// Clipboard consumers want absolute paths; a bare file name means nothing to them.
fn absolute_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let cwd = if paths.iter().any(|p| !p.is_absolute()) {
        Some(std::env::current_dir().map_err(|e| e.to_string())?)
    } else {
        None
    };
    Ok(paths
        .iter()
        .map(|p| match (&cwd, p.is_absolute()) {
            (Some(dir), false) => dir.join(p),
            _ => p.clone(),
        })
        .collect())
}

/// The file name of each path, for the message shown after copying.
pub fn names(paths: &[PathBuf]) -> String {
    match paths {
        [] => String::new(),
        [one] => one
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        many => format!("{} files", many.len()),
    }
}

/// `true` when the path exists, so a stale selection cannot poison the clipboard.
pub fn existing(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter(|p| Path::new(p).exists())
        .cloned()
        .collect()
}

#[cfg(target_os = "windows")]
mod windows {
    use std::path::PathBuf;

    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::System::Ole::CF_HDROP;

    /// `DROPFILES`, the header `CF_HDROP` puts in front of a double-NUL-terminated list.
    ///
    /// `f_wide` must be non-zero: it says the paths are UTF-16, which is the only form
    /// that carries non-ASCII names such as an accented clip title.
    #[repr(C)]
    struct DropFiles {
        p_files: u32,
        pt_x: i32,
        pt_y: i32,
        f_nc: i32,
        f_wide: i32,
    }

    pub(super) fn copy(paths: &[PathBuf]) -> Result<(), String> {
        let mut list: Vec<u16> = Vec::new();
        for path in paths {
            list.extend(path.to_string_lossy().encode_utf16());
            list.push(0);
        }
        list.push(0);

        let header = std::mem::size_of::<DropFiles>();
        let bytes = header + list.len() * 2;

        // SAFETY: the clipboard is opened, filled and closed on this thread, and every
        // pointer written comes from a buffer that outlives the call.
        unsafe {
            OpenClipboard(None).map_err(|e| e.to_string())?;
            let result = (|| -> Result<(), String> {
                EmptyClipboard().map_err(|e| e.to_string())?;
                let handle = GlobalAlloc(GMEM_MOVEABLE, bytes).map_err(|e| e.to_string())?;
                let target = GlobalLock(handle) as *mut u8;
                if target.is_null() {
                    let _ = GlobalFree(Some(handle));
                    return Err("Could not allocate the clipboard buffer".to_string());
                }
                let drop = DropFiles {
                    p_files: header as u32,
                    pt_x: 0,
                    pt_y: 0,
                    f_nc: 0,
                    f_wide: 1,
                };
                std::ptr::copy_nonoverlapping(
                    &drop as *const DropFiles as *const u8,
                    target,
                    header,
                );
                std::ptr::copy_nonoverlapping(
                    list.as_ptr() as *const u8,
                    target.add(header),
                    list.len() * 2,
                );
                // Unlocking to zero still succeeds as far as we care; the handle is
                // owned by the clipboard from here on.
                let _ = GlobalUnlock(handle);
                match SetClipboardData(CF_HDROP.0 as u32, Some(HANDLE(handle.0))) {
                    // On success Windows owns the memory, so it must not be freed.
                    Ok(_) => Ok(()),
                    Err(e) => {
                        let _ = GlobalFree(Some(handle));
                        Err(e.to_string())
                    }
                }
            })();
            let _ = CloseClipboard();
            result
        }
    }

    // HGLOBAL and HANDLE are both pointer-sized handles; the cast above goes through
    // their inner value rather than transmuting the wrapper types.
    const _: fn() = || {
        let _: fn(HGLOBAL) -> HANDLE = |h| HANDLE(h.0);
    };
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{c_void, CString};
    use std::os::raw::c_char;
    use std::path::PathBuf;

    type Id = *mut c_void;

    // Frameworks have to be *loaded* before their classes exist: a process that links
    // only libobjc has the bare runtime, and `objc_getClass` answers nil for every
    // Foundation and AppKit class. Dyld runs the initialisers at startup, which is what
    // registers NSString, NSURL (Foundation) and NSPasteboard (AppKit).
    #[link(name = "Foundation", kind = "framework")]
    extern "C" {}
    #[link(name = "AppKit", kind = "framework")]
    extern "C" {}

    #[link(name = "objc")]
    extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
    }

    fn class(name: &str) -> Id {
        let c = CString::new(name).expect("class name has no NUL");
        // SAFETY: NUL-terminated class name.
        unsafe { objc_getClass(c.as_ptr()) }
    }

    fn selector(name: &str) -> *mut c_void {
        let c = CString::new(name).expect("selector has no NUL");
        // SAFETY: NUL-terminated selector name.
        unsafe { sel_registerName(c.as_ptr()) }
    }

    fn string(text: &str) -> Id {
        let c = match CString::new(text) {
            Ok(c) => c,
            Err(_) => return std::ptr::null_mut(),
        };
        // SAFETY: `c` is a NUL-terminated C string that outlives the call.
        unsafe {
            send_one(
                class("NSString"),
                selector("stringWithUTF8String:"),
                c.as_ptr() as Id,
            )
        }
    }

    unsafe fn send_none(receiver: Id, sel: *mut c_void) -> Id {
        let f: extern "C" fn(Id, *mut c_void) -> Id =
            std::mem::transmute(objc_msgSend as *const ());
        f(receiver, sel)
    }

    unsafe fn send_one(receiver: Id, sel: *mut c_void, arg: Id) -> Id {
        let f: extern "C" fn(Id, *mut c_void, Id) -> Id =
            std::mem::transmute(objc_msgSend as *const ());
        f(receiver, sel, arg)
    }

    unsafe fn send_two(receiver: Id, sel: *mut c_void, first: Id, second: Id) {
        let f: extern "C" fn(Id, *mut c_void, Id, Id) =
            std::mem::transmute(objc_msgSend as *const ());
        f(receiver, sel, first, second);
    }

    unsafe fn send_isize(receiver: Id, sel: *mut c_void) -> isize {
        let f: extern "C" fn(Id, *mut c_void) -> isize =
            std::mem::transmute(objc_msgSend as *const ());
        f(receiver, sel)
    }

    unsafe fn send_bool_one(receiver: Id, sel: *mut c_void, arg: Id) -> bool {
        let f: extern "C" fn(Id, *mut c_void, Id) -> i8 =
            std::mem::transmute(objc_msgSend as *const ());
        f(receiver, sel, arg) != 0
    }

    pub(super) fn copy(paths: &[PathBuf]) -> Result<(), String> {
        // SAFETY: every message below is sent with the argument types its selector
        // declares, to objects created here. The pasteboard is handed *copies* of the
        // data: a `writeObjects:` argument made of live NSURLs only promises them, and
        // the data disappears the moment the autorelease pool drains.
        unsafe {
            let pool = send_none(class("NSAutoreleasePool"), selector("new"));
            let pasteboard = send_none(class("NSPasteboard"), selector("generalPasteboard"));
            if pasteboard.is_null() {
                return Err("Could not reach the clipboard".to_string());
            }

            let items = send_none(class("NSMutableArray"), selector("array"));
            let file_url = string("public.file-url");

            for path in paths {
                let name = string(&path.to_string_lossy());
                if name.is_null() {
                    return Err("A file name contains a NUL".to_string());
                }
                let url = send_one(class("NSURL"), selector("fileURLWithPath:"), name);
                if url.is_null() {
                    return Err("Could not build a file URL".to_string());
                }
                let absolute = send_none(url, selector("absoluteString"));

                let item = send_none(class("NSPasteboardItem"), selector("new"));
                if item.is_null() {
                    return Err("Could not create a clipboard item".to_string());
                }
                send_two(item, selector("setString:forType:"), absolute, file_url);
                send_one(items, selector("addObject:"), item);
                // The array retains it and the pasteboard copies its data.
                let _ = send_none(item, selector("release"));
            }
            // One item per file, each advertising `public.file-url` — what Finder,
            // Discord and Chrome read. The deprecated `NSFilenamesPboardType` list is
            // deliberately not set: modern macOS logs an "invalid UTI" warning for it.

            let _ = send_isize(pasteboard, selector("clearContents"));
            let ok = send_bool_one(pasteboard, selector("writeObjects:"), items);
            let _ = send_none(pool, selector("drain"));
            if ok {
                Ok(())
            } else {
                Err("The clipboard refused the file list".to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_selection_is_refused_before_the_os_is_touched() {
        assert!(copy_files(&[]).is_err());
    }

    #[test]
    fn names_summarise_one_file_or_many() {
        assert_eq!(names(&[]), "");
        assert_eq!(names(&[PathBuf::from("/clips/one.mp4")]), "one.mp4");
        assert_eq!(
            names(&[PathBuf::from("/a.mp4"), PathBuf::from("/b.mp4")]),
            "2 files"
        );
    }
}
