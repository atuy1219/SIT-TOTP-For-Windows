use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroize;

type HmacSha256 = Hmac<Sha256>;

pub const DIGITS: u32 = 6;
pub const PERIOD_SECONDS: u64 = 30;
const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

pub fn normalize_seed(input: &str) -> Result<String, String> {
    let mut seed = input.trim().to_owned();
    if seed
        .get(..10)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("otpauth://"))
    {
        seed = extract_secret_from_otpauth(&seed)?;
    }

    let normalized: String = seed
        .chars()
        .filter(|character| !character.is_whitespace() && *character != '-')
        .collect::<String>()
        .trim_end_matches('=')
        .to_ascii_uppercase();

    if normalized.is_empty() {
        return Err("シードを入力してください".to_owned());
    }
    if !normalized
        .bytes()
        .all(|byte| BASE32_ALPHABET.contains(&byte))
    {
        return Err("シードはBase32形式で入力してください".to_owned());
    }

    let mut decoded = decode_base32(&normalized)?;
    decoded.zeroize();
    Ok(normalized)
}

fn extract_secret_from_otpauth(uri: &str) -> Result<String, String> {
    let query = uri
        .split_once('?')
        .map(|(_, query)| query)
        .ok_or_else(|| "otpauth URIにsecretがありません".to_owned())?;

    for pair in query.split('&') {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        if percent_decode(name)?.eq_ignore_ascii_case("secret") {
            let secret = percent_decode(value)?;
            if !secret.trim().is_empty() {
                return Ok(secret);
            }
        }
    }

    Err("otpauth URIにsecretがありません".to_owned())
}

fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let high = hex_value(bytes[index + 1])?;
                let low = hex_value(bytes[index + 2])?;
                decoded.push((high << 4) | low);
                index += 3;
            }
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8(decoded).map_err(|_| "otpauth URIの文字コードが不正です".to_owned())
}

fn hex_value(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("otpauth URIのパーセントエンコードが不正です".to_owned()),
    }
}

pub fn decode_base32(seed: &str) -> Result<Vec<u8>, String> {
    let mut output = Vec::with_capacity(seed.len() * 5 / 8);
    let mut buffer: u32 = 0;
    let mut bits_in_buffer = 0u32;

    for byte in seed.bytes() {
        let value = BASE32_ALPHABET
            .iter()
            .position(|candidate| *candidate == byte)
            .ok_or_else(|| "シードはBase32形式で入力してください".to_owned())?
            as u32;

        buffer = (buffer << 5) | value;
        bits_in_buffer += 5;

        while bits_in_buffer >= 8 {
            bits_in_buffer -= 8;
            output.push(((buffer >> bits_in_buffer) & 0xff) as u8);
        }

        if bits_in_buffer == 0 {
            buffer = 0;
        } else {
            buffer &= (1u32 << bits_in_buffer) - 1;
        }
    }

    if output.is_empty() {
        return Err("シードが短すぎます".to_owned());
    }
    if bits_in_buffer > 0 && buffer != 0 {
        return Err("Base32シードの末尾ビットが不正です".to_owned());
    }

    Ok(output)
}

pub fn generate(seed: &str, epoch_seconds: u64) -> Result<String, String> {
    let mut key = decode_base32(seed)?;
    let counter = epoch_seconds / PERIOD_SECONDS;

    let mut mac = HmacSha256::new_from_slice(&key)
        .map_err(|_| "TOTP鍵を初期化できません".to_owned())?;
    key.zeroize();
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();

    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let binary = ((u32::from(digest[offset]) & 0x7f) << 24)
        | (u32::from(digest[offset + 1]) << 16)
        | (u32::from(digest[offset + 2]) << 8)
        | u32::from(digest[offset + 3]);
    let code = binary % 10u32.pow(DIGITS);

    Ok(format!("{code:06}"))
}

pub fn remaining_seconds(epoch_seconds: u64) -> u64 {
    PERIOD_SECONDS - (epoch_seconds % PERIOD_SECONDS)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RFC_SHA256_SECRET: &str =
        "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZA";

    #[test]
    fn matches_rfc_6238_sha256_vector_at_59_seconds() {
        assert_eq!(generate(RFC_SHA256_SECRET, 59).unwrap(), "119246");
    }

    #[test]
    fn normalizes_spaces_hyphens_padding_and_case() {
        assert_eq!(normalize_seed("mzxw 6-ytb oi======").unwrap(), "MZXW6YTBOI");
    }

    #[test]
    fn extracts_secret_from_otpauth_uri() {
        assert_eq!(
            normalize_seed("otpauth://totp/SIT?issuer=SIT&secret=MZXW6YTBOI%3D%3D%3D%3D%3D%3D")
                .unwrap(),
            "MZXW6YTBOI"
        );
    }
}
