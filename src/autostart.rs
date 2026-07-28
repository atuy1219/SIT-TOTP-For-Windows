use std::ptr::null_mut;

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
};

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE_NAME: &str = "SIT-TOTP-For-Windows";

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn is_enabled() -> bool {
    unsafe {
        let mut key = null_mut();
        let path = wide(RUN_KEY);
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        ) != ERROR_SUCCESS
        {
            return false;
        }

        let name = wide(VALUE_NAME);
        let result = RegQueryValueExW(
            key,
            name.as_ptr(),
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
        ) == ERROR_SUCCESS;
        RegCloseKey(key);
        result
    }
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    unsafe {
        let mut key = null_mut();
        let path = wide(RUN_KEY);
        let result = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            &mut key,
        );
        if result != ERROR_SUCCESS {
            return Err(format!("自動起動設定を開けません: Windows error {result}"));
        }

        let name = wide(VALUE_NAME);
        let result = if enabled {
            let executable = std::env::current_exe()
                .map_err(|error| format!("実行ファイルの場所を取得できません: {error}"))?;
            let command = format!("\"{}\" --background", executable.display());
            let value = wide(&command);
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * std::mem::size_of::<u16>()) as u32,
            )
        } else {
            let result = RegDeleteValueW(key, name.as_ptr());
            if result == windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND {
                ERROR_SUCCESS
            } else {
                result
            }
        };

        RegCloseKey(key);
        if result == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("自動起動設定を変更できません: Windows error {result}"))
        }
    }
}
