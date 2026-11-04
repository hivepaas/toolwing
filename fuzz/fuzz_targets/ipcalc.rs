//! Addresses, prefixes and ranges as `tw ipcalc` reads them: worked out,
//! split and deaggregated (the first networks of each), and every IPv6
//! address written as glibc writes it reads back as the same address.
#![no_main]

use toolwing_tools::ipcalc;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Some(a) = ipcalc::parse_v6(text) {
        assert_eq!(ipcalc::parse_v6(&ipcalc::v6_text(&a)), Some(a), "{text}");
    }
    if let Some(a) = ipcalc::parse_v4(text) {
        assert_eq!(ipcalc::parse_v4(&ipcalc::v4_text(a)), Some(a), "{text}");
    }
    let (address, rest) = text.split_once(' ').unwrap_or((text, ""));
    let (ip, prefix) = address.split_once('/').unwrap_or((address, ""));
    let mut v6 = ip.contains(':');
    let prefix = ipcalc::str_to_prefix(&mut v6, prefix, false);
    let info = if v6 {
        ipcalc::v6_info(ip, prefix)
    } else {
        ipcalc::v4_info(ip, prefix, rest.is_empty())
    };
    if let Ok(info) = info {
        if let Some(into) = ipcalc::str_to_prefix(&mut v6.clone(), rest, true)
            && let Ok(nets) = ipcalc::split(&info, into)
        {
            assert!(nets.take(64).count() > 0);
        }
    }
    if let Ok(nets) = ipcalc::deaggregate(text, text.contains(':')) {
        assert!(nets.take(64).count() > 0);
    }
});
