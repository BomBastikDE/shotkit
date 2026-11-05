//! Per-user Windows startup entry for this Rust executable.
//!
//! The value name is deliberately distinct from the C# application's `ShotKit`
//! value. The command is built from the executable running in this process.

#[cfg(windows)]
use std::ffi::OsStr;

#[cfg(windows)]
const VALUE_NAME: &str = "ShotKitRust";

/// Returns whether the Rust executable currently running is registered to start
/// at user logon. A missing key/value, or a value for another executable,
/// returns `Ok(false)`.
#[cfg(windows)]
pub fn get() -> Result<bool, String> {
    win32::get()
}

/// Adds or removes this running executable's per-user startup registration.
/// Removing it only deletes the `ShotKitRust` value.
#[cfg(windows)]
pub fn set(enabled: bool) -> Result<(), String> {
    win32::set(enabled)
}

#[cfg(not(windows))]
pub fn get() -> Result<bool, String> {
    Err("Windows autostart is only available on Windows".into())
}

#[cfg(not(windows))]
pub fn set(_enabled: bool) -> Result<(), String> {
    Err("Windows autostart is only available on Windows".into())
}

fn quote_windows_argument(argument: &[u16]) -> Vec<u16> {
    let mut quoted = Vec::with_capacity(argument.len() + 2);
    quoted.push(b'"' as u16);

    let mut slashes = 0usize;
    for &unit in argument {
        if unit == b'\\' as u16 {
            slashes += 1;
            continue;
        }

        if unit == b'"' as u16 {
            quoted.extend(std::iter::repeat(b'\\' as u16).take(slashes * 2 + 1));
            quoted.push(unit);
        } else {
            quoted.extend(std::iter::repeat(b'\\' as u16).take(slashes));
            quoted.push(unit);
        }
        slashes = 0;
    }

    // Backslashes immediately before the closing quote must be doubled so
    // CommandLineToArgvW keeps them as part of the argument.
    quoted.extend(std::iter::repeat(b'\\' as u16).take(slashes * 2));
    quoted.push(b'"' as u16);
    quoted
}

#[cfg(windows)]
mod win32 {
    use super::{quote_windows_argument, OsStr, VALUE_NAME};
    use std::{ffi::c_void, mem::size_of, os::windows::ffi::OsStrExt, ptr};

    type HKey = *mut c_void;
    type LStatus = i32;

    const ERROR_SUCCESS: LStatus = 0;
    const ERROR_FILE_NOT_FOUND: LStatus = 2;
    const ERROR_PATH_NOT_FOUND: LStatus = 3;
    const ERROR_INVALID_DATA: LStatus = 13;
    const REG_SZ: u32 = 1;
    const KEY_QUERY_VALUE: u32 = 0x0001;
    const KEY_SET_VALUE: u32 = 0x0002;
    const KEY_WOW64_64KEY: u32 = 0x0100;
    const RUN_KEY: &[u16] = &[
        b'S' as u16,
        b'o' as u16,
        b'f' as u16,
        b't' as u16,
        b'w' as u16,
        b'a' as u16,
        b'r' as u16,
        b'e' as u16,
        b'\\' as u16,
        b'M' as u16,
        b'i' as u16,
        b'c' as u16,
        b'r' as u16,
        b'o' as u16,
        b's' as u16,
        b'o' as u16,
        b'f' as u16,
        b't' as u16,
        b'\\' as u16,
        b'W' as u16,
        b'i' as u16,
        b'n' as u16,
        b'd' as u16,
        b'o' as u16,
        b'w' as u16,
        b's' as u16,
        b'\\' as u16,
        b'C' as u16,
        b'u' as u16,
        b'r' as u16,
        b'r' as u16,
        b'e' as u16,
        b'n' as u16,
        b't' as u16,
        b'V' as u16,
        b'e' as u16,
        b'r' as u16,
        b's' as u16,
        b'i' as u16,
        b'o' as u16,
        b'n' as u16,
        b'\\' as u16,
        b'R' as u16,
        b'u' as u16,
        b'n' as u16,
        0,
    ];

    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(
            key: HKey,
            sub_key: *const u16,
            options: u32,
            desired_access: u32,
            result: *mut HKey,
        ) -> LStatus;
        fn RegCreateKeyExW(
            key: HKey,
            sub_key: *const u16,
            reserved: u32,
            class: *mut u16,
            options: u32,
            desired_access: u32,
            security_attributes: *mut c_void,
            result: *mut HKey,
            disposition: *mut u32,
        ) -> LStatus;
        fn RegQueryValueExW(
            key: HKey,
            value_name: *const u16,
            reserved: *mut u32,
            value_type: *mut u32,
            data: *mut u8,
            data_size: *mut u32,
        ) -> LStatus;
        fn RegSetValueExW(
            key: HKey,
            value_name: *const u16,
            reserved: u32,
            value_type: u32,
            data: *const u8,
            data_size: u32,
        ) -> LStatus;
        fn RegDeleteValueW(key: HKey, value_name: *const u16) -> LStatus;
        fn RegCloseKey(key: HKey) -> LStatus;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleFileNameW(module: HKey, file_name: *mut u16, size: u32) -> u32;
    }

    struct Key(HKey);

    impl Drop for Key {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    RegCloseKey(self.0);
                }
            }
        }
    }

    fn hkey_current_user() -> HKey {
        // HKEY_CURRENT_USER is the sign-extended predefined pseudo-handle.
        (0x8000_0001u32 as i32 as isize) as HKey
    }

    fn status(operation: &str, code: LStatus) -> String {
        format!(
            "{operation} failed (Windows error {code}: {})",
            std::io::Error::from_raw_os_error(code)
        )
    }

    fn value_name() -> Vec<u16> {
        OsStr::new(VALUE_NAME)
            .encode_wide()
            .chain(Some(0))
            .collect()
    }

    fn current_exe_command() -> Result<Vec<u16>, String> {
        let mut buffer = vec![0u16; 512];
        loop {
            let length = unsafe {
                GetModuleFileNameW(ptr::null_mut(), buffer.as_mut_ptr(), buffer.len() as u32)
            } as usize;
            if length == 0 {
                return Err(format!(
                    "GetModuleFileNameW failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            if length < buffer.len() {
                buffer.truncate(length);
                return Ok(quote_windows_argument(&buffer));
            }
            if buffer.len() >= 1_048_576 {
                return Err("Executable path exceeds the supported length".into());
            }
            buffer.resize(buffer.len() * 2, 0);
        }
    }

    fn open_key(path: &[u16], access: u32) -> Result<Key, LStatus> {
        let mut key: HKey = ptr::null_mut();
        let result = unsafe {
            RegOpenKeyExW(
                hkey_current_user(),
                path.as_ptr(),
                0,
                access | KEY_WOW64_64KEY,
                &mut key,
            )
        };
        if result == ERROR_SUCCESS {
            Ok(Key(key))
        } else {
            Err(result)
        }
    }

    pub(super) fn get() -> Result<bool, String> {
        get_at(RUN_KEY)
    }

    fn get_at(path: &[u16]) -> Result<bool, String> {
        let key = match open_key(path, KEY_QUERY_VALUE) {
            Ok(key) => key,
            Err(ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) => return Ok(false),
            Err(code) => return Err(status("RegOpenKeyExW", code)),
        };

        let name = value_name();
        let mut value_type = 0u32;
        let mut byte_count = 0u32;
        let queried = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                ptr::null_mut(),
                &mut value_type,
                ptr::null_mut(),
                &mut byte_count,
            )
        };
        if queried == ERROR_FILE_NOT_FOUND {
            return Ok(false);
        }
        if queried != ERROR_SUCCESS {
            return Err(status("RegQueryValueExW(size)", queried));
        }
        if value_type != REG_SZ || byte_count as usize % size_of::<u16>() != 0 {
            return Err(status(
                "Invalid ShotKitRust registry value",
                ERROR_INVALID_DATA,
            ));
        }
        if byte_count as usize > 1_048_576 {
            return Err("ShotKitRust registry value is unexpectedly large".into());
        }

        let mut bytes = vec![0u8; byte_count as usize];
        let read = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                ptr::null_mut(),
                &mut value_type,
                bytes.as_mut_ptr(),
                &mut byte_count,
            )
        };
        if read != ERROR_SUCCESS {
            return Err(status("RegQueryValueExW(data)", read));
        }
        if value_type != REG_SZ || byte_count as usize % size_of::<u16>() != 0 {
            return Err(status(
                "Invalid ShotKitRust registry value",
                ERROR_INVALID_DATA,
            ));
        }
        bytes.truncate(byte_count as usize);
        let mut stored: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]))
            .collect();
        if stored.pop() != Some(0) || stored.contains(&0) {
            return Err(status(
                "Invalid ShotKitRust registry string",
                ERROR_INVALID_DATA,
            ));
        }

        Ok(stored == current_exe_command()?)
    }

    pub(super) fn set(enabled: bool) -> Result<(), String> {
        set_at(RUN_KEY, enabled)
    }

    fn set_at(path: &[u16], enabled: bool) -> Result<(), String> {
        if !enabled {
            let key = match open_key(path, KEY_SET_VALUE) {
                Ok(key) => key,
                Err(ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) => return Ok(()),
                Err(code) => return Err(status("RegOpenKeyExW", code)),
            };
            let name = value_name();
            let deleted = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
            return match deleted {
                ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
                code => Err(status("RegDeleteValueW", code)),
            };
        }

        let command = current_exe_command()?;
        let mut data: Vec<u16> = command;
        data.push(0);
        let byte_count = data
            .len()
            .checked_mul(size_of::<u16>())
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| "Executable command is too long for the registry".to_string())?;

        let mut raw_key: HKey = ptr::null_mut();
        let mut disposition = 0u32;
        let created = unsafe {
            RegCreateKeyExW(
                hkey_current_user(),
                path.as_ptr(),
                0,
                ptr::null_mut(),
                0,
                KEY_SET_VALUE | KEY_WOW64_64KEY,
                ptr::null_mut(),
                &mut raw_key,
                &mut disposition,
            )
        };
        if created != ERROR_SUCCESS {
            if !raw_key.is_null() {
                drop(Key(raw_key));
            }
            return Err(status("RegCreateKeyExW", created));
        }
        let key = Key(raw_key);
        let name = value_name();
        let written = unsafe {
            RegSetValueExW(
                key.0,
                name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr().cast(),
                byte_count,
            )
        };
        if written != ERROR_SUCCESS {
            return Err(status("RegSetValueExW", written));
        }
        Ok(())
    }
    #[cfg(test)]
    mod registry_tests {
        use super::*;

        #[repr(C)]
        struct Guid {
            data1: u32,
            data2: u16,
            data3: u16,
            data4: [u8; 8],
        }

        #[link(name = "ole32")]
        extern "system" {
            fn CoCreateGuid(guid: *mut Guid) -> i32;
        }

        // Owns only the value in a newly created GUID key. Never deletes the key:
        // the volatile key disappears when the short-lived runner's hive unloads.
        struct TestValue(Key);
        impl Drop for TestValue {
            fn drop(&mut self) {
                unsafe { RegDeleteValueW(self.0 .0, value_name().as_ptr()) };
            }
        }

        #[test]
        fn registry_roundtrip_in_hosted_runner() {
            // Environment variables are an operational guard, not an attestation.
            if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true") {
                eprintln!("Skipping registry test outside GitHub Actions");
                return;
            }
            assert_eq!(
                std::env::var("RUNNER_ENVIRONMENT").as_deref(),
                Ok("github-hosted"),
                "registry test requires the ephemeral GitHub-hosted runner"
            );

            let mut guid = Guid {
                data1: 0,
                data2: 0,
                data3: 0,
                data4: [0; 8],
            };
            assert_eq!(unsafe { CoCreateGuid(&mut guid) }, 0, "CoCreateGuid failed");
            let path: Vec<u16> = format!(
                "Software\\ShotKitRustTest-{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                guid.data1, guid.data2, guid.data3, guid.data4[0], guid.data4[1],
                guid.data4[2], guid.data4[3], guid.data4[4], guid.data4[5], guid.data4[6], guid.data4[7]
            ).encode_utf16().chain(Some(0)).collect();
            let mut raw = ptr::null_mut();
            let mut disposition = 0;
            let code = unsafe {
                RegCreateKeyExW(
                    hkey_current_user(),
                    path.as_ptr(),
                    0,
                    ptr::null_mut(),
                    1,
                    KEY_QUERY_VALUE | KEY_SET_VALUE | KEY_WOW64_64KEY,
                    ptr::null_mut(),
                    &mut raw,
                    &mut disposition,
                )
            };
            assert_eq!(code, ERROR_SUCCESS, "{}", status("create test key", code));
            let key = Key(raw);
            // A collision is never read, modified or cleaned up.
            assert_eq!(disposition, 1, "test key collision");
            assert!(!get_at(&path).unwrap());
            let owned = TestValue(key);
            set_at(&path, true).unwrap();
            assert!(get_at(&path).unwrap());

            let mut kind = 0;
            let mut size = 0;
            let name = value_name();
            assert_eq!(
                unsafe {
                    RegQueryValueExW(
                        owned.0 .0,
                        name.as_ptr(),
                        ptr::null_mut(),
                        &mut kind,
                        ptr::null_mut(),
                        &mut size,
                    )
                },
                ERROR_SUCCESS
            );
            assert_eq!(kind, REG_SZ);
            let mut data = vec![0u8; size as usize];
            assert_eq!(
                unsafe {
                    RegQueryValueExW(
                        owned.0 .0,
                        name.as_ptr(),
                        ptr::null_mut(),
                        &mut kind,
                        data.as_mut_ptr(),
                        &mut size,
                    )
                },
                ERROR_SUCCESS
            );
            let mut expected = current_exe_command().unwrap();
            assert_eq!(expected.first(), Some(&(b'"' as u16)));
            assert_eq!(expected.last(), Some(&(b'"' as u16)));
            expected.push(0);
            let actual: Vec<u16> = data
                .chunks_exact(2)
                .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]))
                .collect();
            assert_eq!(actual, expected);

            // The product reader rejects a value of the wrong type.
            let invalid = 1u32;
            assert_eq!(
                unsafe {
                    RegSetValueExW(
                        owned.0 .0,
                        name.as_ptr(),
                        0,
                        4,
                        (&invalid as *const u32).cast(),
                        size_of::<u32>() as u32,
                    )
                },
                ERROR_SUCCESS
            );
            assert!(get_at(&path).is_err());
            set_at(&path, false).unwrap();
            assert!(!get_at(&path).unwrap());
            set_at(&path, false).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::quote_windows_argument;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    fn text(units: &[u16]) -> String {
        String::from_utf16(units).unwrap()
    }

    #[test]
    fn quotes_plain_path_and_unicode() {
        assert_eq!(
            text(&quote_windows_argument(&wide(
                r"C:\Program Files\ShotKit\ShotKit.exe"
            ))),
            r#""C:\Program Files\ShotKit\ShotKit.exe""#
        );
        assert_eq!(
            text(&quote_windows_argument(&wide(
                r"C:\Apps\Änderung\ShotKit.exe"
            ))),
            r#""C:\Apps\Änderung\ShotKit.exe""#
        );
    }

    #[test]
    fn escapes_quotes_and_trailing_backslashes() {
        assert_eq!(
            text(&quote_windows_argument(&wide("a\\\"b"))),
            r#""a\\\"b""#
        );
        assert_eq!(
            text(&quote_windows_argument(&wide("path\\"))),
            r#""path\\""#
        );
        assert_eq!(text(&quote_windows_argument(&[])), "\"\"");
    }
}
