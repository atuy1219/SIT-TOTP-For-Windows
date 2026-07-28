use std::fs;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::slice;

use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN,
};
use windows_sys::Win32::System::Memory::LocalFree;
use zeroize::Zeroize;

fn secret_path() -> Result<PathBuf, String> {
    let base = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| "LOCALAPPDATAを取得できません".to_owned())?;
    let directory = PathBuf::from(base).join("SIT-TOTP");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("設定フォルダーを作成できません: {error}"))?;
    Ok(directory.join("seed.dat"))
}

pub fn save(seed: &str) -> Result<(), String> {
    let mut plaintext = seed.as_bytes().to_vec();
    let input = CRYPT_INTEGER_BLOB {
        cbData: plaintext.len() as u32,
        pbData: plaintext.as_mut_ptr(),
    };
    let mut output: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };

    let success = unsafe {
        CryptProtectData(
            &input,
            null(),
            null(),
            null_mut(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    plaintext.zeroize();

    if success == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }

    let encrypted = unsafe { slice::from_raw_parts(output.pbData, output.cbData as usize) };
    let result = fs::write(secret_path()?, encrypted)
        .map_err(|error| format!("暗号化したシードを保存できません: {error}"));
    unsafe {
        LocalFree(output.pbData.cast());
    }
    result
}

pub fn load() -> Result<Option<String>, String> {
    let path = secret_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let mut encrypted = fs::read(&path)
        .map_err(|error| format!("保存済みシードを読み込めません: {error}"))?;
    if encrypted.is_empty() {
        return Ok(None);
    }

    let input = CRYPT_INTEGER_BLOB {
        cbData: encrypted.len() as u32,
        pbData: encrypted.as_mut_ptr(),
    };
    let mut output: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };

    let success = unsafe {
        CryptUnprotectData(
            &input,
            null_mut(),
            null(),
            null_mut(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    encrypted.zeroize();

    if success == 0 {
        return Err(format!(
            "保存済みシードを復号できません: {}",
            std::io::Error::last_os_error()
        ));
    }

    let bytes = unsafe { slice::from_raw_parts(output.pbData, output.cbData as usize) };
    let result = String::from_utf8(bytes.to_vec())
        .map(Some)
        .map_err(|_| "保存済みシードの形式が不正です".to_owned());
    unsafe {
        LocalFree(output.pbData.cast());
    }
    result
}

pub fn delete() -> Result<(), String> {
    let path = secret_path()?;
    if path.exists() {
        fs::remove_file(path).map_err(|error| format!("シードを削除できません: {error}"))?;
    }
    Ok(())
}
