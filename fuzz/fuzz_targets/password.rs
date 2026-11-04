//! Password hashes as `tw bcrypt verify` and `tw argon2 verify` read them:
//! the hash parsed, and checked when its costs are the smallest, so a run
//! stays quick.
#![no_main]

use toolwing_tools::password;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let (hash, pwd) = text.split_once('\n').unwrap_or((text, ""));
    if let Ok(parts) = password::bcrypt_parts(hash)
        && parts.cost == 4
    {
        assert!(bcrypt::verify(pwd, hash).is_ok());
    }
    if let Ok((spec, salt, _)) = password::argon2_decode(hash) {
        assert!(salt.len() >= 8 && spec.out_len >= 4);
        if spec.m_cost <= 64 && spec.t_cost <= 2 && spec.lanes <= 8 && spec.out_len <= 64 {
            assert!(password::argon2_verify(hash, pwd.as_bytes()).is_ok());
        }
    }
});
