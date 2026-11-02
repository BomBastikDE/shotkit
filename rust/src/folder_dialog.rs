//! Native Windows folder picker using the modern Common Item Dialog.
//!
//! Paths are returned as Unicode `PathBuf`s. The dialog API has no legacy
//! `MAX_PATH` output buffer; the shell still applies the Windows filesystem's
//! own path and policy limits. COM is initialized as STA on the calling thread
//! and every interface/string allocated by COM is released before uninitializing.

use std::{ffi::c_void, path::PathBuf};

/// Show a folder picker. `owner` may be null; otherwise pass the owning HWND.
/// User cancellation is `Ok(None)`, while COM/dialog failures are `Err`.
pub fn choose_folder(owner: *mut c_void) -> Result<Option<PathBuf>, String> {
    #[cfg(windows)]
    {
        win32::choose_folder(owner)
    }
    #[cfg(not(windows))]
    {
        let _ = owner;
        Err("Folder selection is Windows-only".into())
    }
}

#[cfg(windows)]
mod win32 {
    use super::*;
    use std::{os::windows::ffi::OsStringExt, ptr::null_mut, slice};

    type HResult = i32;
    type Hwnd = *mut c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, coinit: u32) -> HResult;
        fn CoUninitialize();
        fn CoCreateInstance(
            clsid: *const Guid,
            outer: *mut c_void,
            context: u32,
            iid: *const Guid,
            object: *mut *mut c_void,
        ) -> HResult;
        fn CoTaskMemFree(memory: *mut c_void);
    }

    const CLSCTX_INPROC_SERVER: u32 = 1;
    const COINIT_APARTMENTTHREADED: u32 = 0x2;
    const FOS_NOCHANGEDIR: u32 = 0x8;
    const FOS_PICKFOLDERS: u32 = 0x20;
    const FOS_FORCEFILESYSTEM: u32 = 0x40;
    const FOS_PATHMUSTEXIST: u32 = 0x800;
    const SIGDN_FILESYSPATH: u32 = 0x8005_8000;
    const HRESULT_CANCELLED: HResult = 0x8007_04c7u32 as i32;

    const CLSID_FILE_OPEN_DIALOG: Guid = Guid {
        data1: 0xdc1c_5a9c,
        data2: 0xe88a,
        data3: 0x4dde,
        data4: [0xa5, 0xa1, 0x60, 0xf8, 0x2a, 0x20, 0xae, 0xf7],
    };
    const IID_I_FILE_DIALOG: Guid = Guid {
        data1: 0x42f8_5136,
        data2: 0xdb7e,
        data3: 0x439c,
        data4: [0x85, 0xf1, 0xe4, 0x07, 0x5d, 0x13, 0x5f, 0xc8],
    };

    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }

    #[repr(C)]
    struct UnknownVTable {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
    }

    struct ComPtr(*mut c_void);
    impl ComPtr {
        unsafe fn from_raw(raw: *mut c_void, description: &str) -> Result<Self, String> {
            if raw.is_null() {
                Err(format!("{description} returned a null interface"))
            } else {
                Ok(Self(raw))
            }
        }
    }
    impl Drop for ComPtr {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    let vtable = *(self.0 as *mut *const UnknownVTable);
                    ((*vtable).release)(self.0);
                }
            }
        }
    }

    #[repr(C)]
    struct FileDialogVTable {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        show: unsafe extern "system" fn(*mut c_void, Hwnd) -> HResult,
        set_file_types: usize,
        set_file_type_index: usize,
        get_file_type_index: usize,
        advise: usize,
        unadvise: usize,
        set_options: unsafe extern "system" fn(*mut c_void, u32) -> HResult,
        get_options: unsafe extern "system" fn(*mut c_void, *mut u32) -> HResult,
        set_default_folder: usize,
        set_folder: usize,
        get_folder: usize,
        get_current_selection: usize,
        set_file_name: usize,
        get_file_name: usize,
        set_title: unsafe extern "system" fn(*mut c_void, *const u16) -> HResult,
        set_ok_button_label: usize,
        set_file_name_label: usize,
        get_result: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HResult,
    }

    #[repr(C)]
    struct ShellItemVTable {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        bind_to_handler: usize,
        get_parent: usize,
        get_display_name: unsafe extern "system" fn(*mut c_void, u32, *mut *mut u16) -> HResult,
    }

    fn hresult(operation: &str, code: HResult) -> String {
        format!("{operation} failed (HRESULT 0x{:08X})", code as u32)
    }

    unsafe fn vtable<T>(object: *mut c_void) -> *const T {
        *(object as *mut *const T)
    }

    struct CoTaskString(*mut u16);
    impl Drop for CoTaskString {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    CoTaskMemFree(self.0.cast());
                }
            }
        }
    }

    pub(super) fn choose_folder(owner: Hwnd) -> Result<Option<PathBuf>, String> {
        unsafe {
            let init = CoInitializeEx(null_mut(), COINIT_APARTMENTTHREADED);
            if init < 0 {
                return Err(hresult("CoInitializeEx(COINIT_APARTMENTTHREADED)", init));
            }
            let _apartment = Apartment;

            let mut raw_dialog = null_mut();
            let created = CoCreateInstance(
                &CLSID_FILE_OPEN_DIALOG,
                null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_I_FILE_DIALOG,
                &mut raw_dialog,
            );
            if created < 0 {
                if !raw_dialog.is_null() {
                    drop(ComPtr(raw_dialog));
                }
                return Err(hresult("CoCreateInstance(FileOpenDialog)", created));
            }
            let dialog = ComPtr::from_raw(raw_dialog, "CoCreateInstance(FileOpenDialog)")?;
            let methods = &*vtable::<FileDialogVTable>(dialog.0);

            let mut options = 0;
            let got_options = (methods.get_options)(dialog.0, &mut options);
            if got_options < 0 {
                return Err(hresult("IFileDialog::GetOptions", got_options));
            }
            let options = options
                | FOS_PICKFOLDERS
                | FOS_FORCEFILESYSTEM
                | FOS_PATHMUSTEXIST
                | FOS_NOCHANGEDIR;
            let set_options = (methods.set_options)(dialog.0, options);
            if set_options < 0 {
                return Err(hresult("IFileDialog::SetOptions", set_options));
            }

            let title: Vec<u16> = "Speicherordner auswählen"
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let set_title = (methods.set_title)(dialog.0, title.as_ptr());
            if set_title < 0 {
                return Err(hresult("IFileDialog::SetTitle", set_title));
            }

            let shown = (methods.show)(dialog.0, owner);
            if shown == HRESULT_CANCELLED {
                return Ok(None);
            }
            if shown < 0 {
                return Err(hresult("IFileDialog::Show", shown));
            }

            let mut raw_item = null_mut();
            let got_result = (methods.get_result)(dialog.0, &mut raw_item);
            if got_result < 0 {
                if !raw_item.is_null() {
                    drop(ComPtr(raw_item));
                }
                return Err(hresult("IFileDialog::GetResult", got_result));
            }
            let item = ComPtr::from_raw(raw_item, "IFileDialog::GetResult")?;
            let item_methods = &*vtable::<ShellItemVTable>(item.0);

            let mut raw_path = null_mut();
            let got_path =
                (item_methods.get_display_name)(item.0, SIGDN_FILESYSPATH, &mut raw_path);
            let path = CoTaskString(raw_path);
            if got_path < 0 {
                return Err(hresult("IShellItem::GetDisplayName", got_path));
            }
            if path.0.is_null() {
                return Err("IShellItem::GetDisplayName returned a null path".into());
            }
            let mut length = 0usize;
            while *path.0.add(length) != 0 {
                length = length
                    .checked_add(1)
                    .ok_or_else(|| "Selected folder path length overflow".to_string())?;
            }
            if length == 0 {
                return Err("Selected folder path is empty".into());
            }
            let wide_path = slice::from_raw_parts(path.0, length);
            let result = PathBuf::from(std::ffi::OsString::from_wide(wide_path));
            if !result.is_absolute() {
                return Err("Shell returned a non-absolute file-system path".into());
            }
            Ok(Some(result))
        }
    }
}
