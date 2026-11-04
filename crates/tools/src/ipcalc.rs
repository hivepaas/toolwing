//! `ipcalc`: what an IPv4 or IPv6 address and prefix say (network, mask,
//! broadcast, first and last hosts, how many, the IANA address space,
//! reverse DNS), a network split into smaller ones, and a range of
//! addresses as the fewest networks that cover it, as ipcalc 1.0.3 (Red
//! Hat's, Debian's ipcalc-ng) works them out. Addresses are read and written
//! as glibc's `inet_pton` and `inet_ntop` do, as ipcalc does through them.
//! Names are never looked up: toolwing stays off the network.

use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, Field, InputKind, Meta, Output, Param, ParamKind, Reference,
    Score, Tool, Value, Values, en,
};

// ---- Addresses, as glibc reads and writes them ------------------------------

/// glibc's `inet_pton(AF_INET, …)`: four decimal parts, 0 to 255, without
/// leading zeros.
pub fn parse_v4(s: &str) -> Option<u32> {
    parse_v4_bytes(s.as_bytes()).map(u32::from_be_bytes)
}

fn parse_v4_bytes(s: &[u8]) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let (mut part, mut octets, mut saw_digit) = (0usize, 0, false);
    for &ch in s {
        match ch {
            b'0'..=b'9' => {
                let new = u32::from(out[part]) * 10 + u32::from(ch - b'0');
                if saw_digit && out[part] == 0 {
                    return None;
                }
                if new > 255 {
                    return None;
                }
                out[part] = new as u8;
                if !saw_digit {
                    octets += 1;
                    if octets > 4 {
                        return None;
                    }
                    saw_digit = true;
                }
            }
            b'.' if saw_digit => {
                if octets == 4 {
                    return None;
                }
                part += 1;
                saw_digit = false;
            }
            _ => return None,
        }
    }
    (octets == 4).then_some(out)
}

/// glibc's `inet_pton(AF_INET6, …)`, with a dotted IPv4 tail where it fits.
pub fn parse_v6(s: &str) -> Option<[u8; 16]> {
    let src = s.as_bytes();
    let mut tmp = [0u8; 16];
    let mut tp = 0usize;
    let mut colonp: Option<usize> = None;
    let mut i = 0usize;
    if src.is_empty() {
        return None;
    }
    if src[0] == b':' {
        i += 1;
        if i == src.len() || src[i] != b':' {
            return None;
        }
    }
    let mut curtok = i;
    let (mut xdigits, mut val) = (0usize, 0u32);
    while i < src.len() {
        let ch = src[i];
        i += 1;
        if let Some(d) = (ch as char).to_digit(16) {
            if xdigits == 4 {
                return None;
            }
            val = (val << 4) | d;
            xdigits += 1;
            continue;
        }
        if ch == b':' {
            curtok = i;
            if xdigits == 0 {
                if colonp.is_some() {
                    return None;
                }
                colonp = Some(tp);
                continue;
            } else if i == src.len() {
                return None;
            }
            if tp + 2 > 16 {
                return None;
            }
            tmp[tp] = (val >> 8) as u8;
            tmp[tp + 1] = val as u8;
            tp += 2;
            xdigits = 0;
            val = 0;
            continue;
        }
        if ch == b'.'
            && tp + 4 <= 16
            && let Some(v4) = parse_v4_bytes(&src[curtok..])
        {
            tmp[tp..tp + 4].copy_from_slice(&v4);
            tp += 4;
            xdigits = 0;
            break;
        }
        return None;
    }
    if xdigits > 0 {
        if tp + 2 > 16 {
            return None;
        }
        tmp[tp] = (val >> 8) as u8;
        tmp[tp + 1] = val as u8;
        tp += 2;
    }
    if let Some(c) = colonp {
        if tp == 16 {
            return None;
        }
        let n = tp - c;
        tmp.copy_within(c..tp, 16 - n);
        tmp[c..16 - n].fill(0);
        tp = 16;
    }
    (tp == 16).then_some(tmp)
}

pub fn v4_text(a: u32) -> String {
    let b = a.to_be_bytes();
    format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
}

/// glibc's `inet_ntop(AF_INET6, …)`: the first longest run of two or more
/// zero groups as `::`, and a dotted tail for `::a.b.c.d` and
/// `::ffff:a.b.c.d`.
pub fn v6_text(a: &[u8; 16]) -> String {
    let words: Vec<u16> = a
        .chunks(2)
        .map(|w| u16::from(w[0]) << 8 | u16::from(w[1]))
        .collect();
    let (mut best, mut cur): ((isize, usize), (isize, usize)) = ((-1, 0), (-1, 0));
    for (i, &w) in words.iter().enumerate() {
        if w == 0 {
            if cur.0 == -1 {
                cur = (i as isize, 1);
            } else {
                cur.1 += 1;
            }
        } else if cur.0 != -1 {
            if best.0 == -1 || cur.1 > best.1 {
                best = cur;
            }
            cur.0 = -1;
        }
    }
    if cur.0 != -1 && (best.0 == -1 || cur.1 > best.1) {
        best = cur;
    }
    if best.0 != -1 && best.1 < 2 {
        best.0 = -1;
    }
    let mut out = String::new();
    let in_best = |i: usize| best.0 != -1 && i as isize >= best.0 && i < best.0 as usize + best.1;
    for (i, &w) in words.iter().enumerate() {
        if in_best(i) {
            if i as isize == best.0 {
                out.push(':');
            }
            continue;
        }
        if i != 0 {
            out.push(':');
        }
        if i == 6 && best.0 == 0 && (best.1 == 6 || (best.1 == 5 && words[5] == 0xffff)) {
            out.push_str(&format!("{}.{}.{}.{}", a[12], a[13], a[14], a[15]));
            return out;
        }
        out.push_str(&format!("{w:x}"));
    }
    if best.0 != -1 && best.0 as usize + best.1 == 8 {
        out.push(':');
    }
    out
}

/// Every group in full: `2001:0db8:0000:…`.
pub fn v6_expanded(a: &[u8; 16]) -> String {
    a.chunks(2)
        .map(|w| format!("{:02x}{:02x}", w[0], w[1]))
        .collect::<Vec<_>>()
        .join(":")
}

// ---- Prefixes ---------------------------------------------------------------

/// C's `strtol(s, &end, 0)` that must read all of `s` and fit an `int`:
/// ipcalc's `safe_atoi`. `0x18` and `030` are both 24.
pub fn safe_atoi(s: &str) -> Option<i32> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r') {
        i += 1;
    }
    let negative = match b.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let mut radix = 10;
    if b.get(i) == Some(&b'0') {
        if matches!(b.get(i + 1), Some(b'x' | b'X'))
            && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit)
        {
            radix = 16;
            i += 2;
        } else {
            radix = 8;
        }
    }
    let digits = &s[i..];
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    let n = i64::from_str_radix(digits, radix).ok()?;
    let n = if negative { -n } else { n };
    i32::try_from(n).ok()
}

/// A netmask's prefix (`255.255.255.0` is 24); `None` for one that is not
/// a run of ones, or not an address.
pub fn mask_to_prefix(s: &str) -> Option<i32> {
    let mut m = parse_v4(s)?;
    let (mut count, mut seen_one) = (0, false);
    while m > 0 {
        if m & 1 == 1 {
            seen_one = true;
            count += 1;
        } else if seen_one {
            return None;
        }
        m >>= 1;
    }
    Some(count)
}

/// ipcalc's `str_to_prefix`: a number, or for IPv4 a netmask. With `fix`,
/// a number past 32 makes the address IPv6 (`-S 64`).
pub fn str_to_prefix(v6: &mut bool, s: &str, fix: bool) -> Option<u32> {
    let prefix = if !*v6 && s.contains('.') {
        mask_to_prefix(s)?
    } else {
        safe_atoi(s)?
    };
    if fix && prefix > 32 && !*v6 {
        *v6 = true;
    }
    if prefix < 0 || (*v6 && prefix > 128) || (!*v6 && prefix > 32) {
        return None;
    }
    Some(prefix as u32)
}

fn mask_v4(prefix: u32) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    }
}

fn mask_v6(prefix: u32) -> [u8; 16] {
    let mut m = [0u8; 16];
    let mut left = prefix as i32;
    for byte in m.iter_mut() {
        if left <= 0 {
            break;
        }
        *byte = if left >= 8 {
            0xff
        } else {
            0xffu8 << (8 - left)
        };
        left -= 8;
    }
    m
}

/// 2^n in decimal, as ipcalc's table has them; "" past 2^127.
fn pow2(n: u32) -> String {
    if n > 127 {
        String::new()
    } else {
        (1u128 << n).to_string()
    }
}

/// Hosts in an IPv4 network of `prefix`: all but the network and broadcast
/// addresses, the two of a /31 and the one of a /32. A /0 gives what the
/// reference's 32-bit shift gives on x86, 4294967295.
pub fn v4_hosts(prefix: u32) -> String {
    if prefix >= 31 {
        pow2(32 - prefix)
    } else {
        1u32.wrapping_shl(32 - prefix).wrapping_sub(2).to_string()
    }
}

pub fn v6_hosts(prefix: u32) -> String {
    pow2(128 - prefix)
}

// ---- Address spaces ---------------------------------------------------------

/// The IANA IPv4 special registry and address space, as ipcalc 1.0.3 has
/// them (updated 2020-04-06).
fn v4_space(net: u32) -> &'static str {
    let [b1, b2, b3, b4] = net.to_be_bytes();
    match (b1, b2, b3, b4) {
        (0, ..) => "This host on this network",
        (10, ..) => "Private Use",
        (100, b, ..) if b & 0xc0 == 64 => "Shared Address Space",
        (127, ..) => "Loopback",
        (169, 254, ..) => "Link Local",
        (172, b, ..) if b & 0xf0 == 16 => "Private Use",
        (192, 0, 0, 0..=7) => "IPv4 Service Continuity Prefix",
        (192, 0, 0, 8) => "IPv4 dummy address",
        (192, 0, 0, 9) => "Port Control Protocol Anycast",
        (192, 0, 0, 10) => "Traversal Using Relays around NAT Anycast",
        (192, 0, 0, 170 | 171) => "NAT64/DNS64 Discovery",
        (192, 0, 0, _) => "IETF Protocol Assignments",
        (192, 0, 2, _) => "Documentation (TEST-NET-1)",
        (198, 51, 100, _) => "Documentation (TEST-NET-2)",
        (203, 0, 113, _) => "Documentation (TEST-NET-3)",
        (192, 88, 99, _) => "6 to 4 Relay Anycast (Deprecated)",
        (192, 31, 196, _) => "AS112-v4",
        (192, 52, 193, _) => "AMT",
        (192, 168, ..) => "Private Use",
        (192, 175, 48, _) => "Direct Delegation AS112 Service",
        (255, 255, 255, 255) => "Limited Broadcast",
        (198, b, ..) if b & 0xfe == 18 => "Benchmarking",
        (224..=239, ..) => "Multicast",
        (b, ..) if b & 0xf0 == 240 => "Reserved",
        _ => "Internet",
    }
}

/// The class, as ipcalc has it: 239 is in E, an off-by-one it keeps.
fn v4_class(net: u32) -> &'static str {
    match net >> 24 {
        0..=127 => "Class A",
        128..=191 => "Class B",
        192..=223 => "Class C",
        224..=238 => "Class D",
        _ => "Class E",
    }
}

/// The prefix of an address's class, for `--class-prefix`.
fn v4_class_prefix(ip: u32) -> u32 {
    match ip >> 24 {
        0..=127 => 8,
        128..=191 => 16,
        _ => 24,
    }
}

/// The IANA IPv6 special registry and address space, as ipcalc 1.0.3 has
/// them (updated 2019-09-13).
fn v6_space(net: &[u8; 16], prefix: u32) -> &'static str {
    let word1 = u16::from(net[0]) << 8 | u16::from(net[1]);
    let word2 = u16::from(net[2]) << 8 | u16::from(net[3]);
    let is = |bytes: &[u8]| net.starts_with(bytes);
    let mut loopback = [0u8; 16];
    loopback[15] = 1;
    let mut pcp = [0x20, 0x01, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    if prefix == 128 && *net == loopback {
        return "Loopback Address";
    }
    if prefix == 128 && *net == [0u8; 16] {
        return "Unspecified Address";
    }
    if prefix >= 96 && is(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff]) {
        return "IPv4-mapped Address";
    }
    if prefix >= 96 && is(&[0, 0x64, 0xff, 0x9b, 0, 0, 0, 0, 0, 0, 0, 0]) {
        return "IPv4-IPv6 Translat.";
    }
    if prefix >= 48 && is(&[0, 0x64, 0xff, 0x9b, 0, 1]) {
        return "IPv4-IPv6 Translat.";
    }
    if prefix >= 96 && is(&[0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]) {
        return "Discard-Only Address Block";
    }
    if prefix >= 32 && word1 == 0x2001 && word2 == 0 {
        return "TEREDO";
    }
    if prefix == 128 && *net == pcp {
        return "Port Control Protocol Anycast";
    }
    pcp[15] = 2;
    if prefix == 128 && *net == pcp {
        return "Traversal Using Relays around NAT Anycast";
    }
    if prefix >= 48 && is(&[0x20, 0x01, 0, 2, 0, 0]) {
        return "Benchmarking";
    }
    if prefix >= 32 && word1 == 0x2001 && word2 == 0x3 {
        return "AMT";
    }
    if prefix >= 48 && is(&[0x20, 0x01, 0, 4, 0x01, 0x12]) {
        return "AS112-v6";
    }
    if prefix >= 28 && word1 == 0x2001 && word2 & 0xfff0 == 0x10 {
        return "Deprecated (previously ORCHID)";
    }
    if prefix >= 28 && word1 == 0x2001 && word2 & 0xfff0 == 0x20 {
        return "ORCHIDv2";
    }
    if prefix >= 23 && word1 == 0x2001 && word2 & 0xff00 <= 0x100 {
        return "IETF Protocol Assignments";
    }
    if prefix >= 32 && word1 == 0x2001 && word2 == 0xdb8 {
        return "Documentation";
    }
    if word1 == 0x2002 {
        return "6to4";
    }
    if prefix >= 48 && is(&[0x26, 0x20, 0, 0x4f, 0x80, 0]) {
        return "Direct Delegation AS112 Service";
    }
    if word1 & 0xe000 == 0x2000 {
        return "Global Unicast";
    }
    if net[0] & 0xfe == 0xfc {
        return "Unique Local Unicast";
    }
    if word1 & 0xffc0 == 0xfe80 {
        return "Link-Scoped Unicast";
    }
    if net[0] == 0xff {
        return "Multicast";
    }
    "Reserved"
}

// ---- Reverse DNS ------------------------------------------------------------

/// The reverse zone of an IPv4 network, a range of the octet it splits
/// past /8, /16, /24 (draft-ietf-dnsop-rfc2317bis); none under /8.
fn v4_reverse(net: u32, prefix: u32, broadcast: u32) -> Option<String> {
    let [b1, b2, b3, b4] = net.to_be_bytes();
    let range = |shift: u32| ((net >> shift) & 0xff, (broadcast >> shift) & 0xff);
    Some(match prefix {
        32 => format!("{b4}.{b3}.{b2}.{b1}.in-addr.arpa."),
        24 => format!("{b3}.{b2}.{b1}.in-addr.arpa."),
        16 => format!("{b2}.{b1}.in-addr.arpa."),
        8 => format!("{b1}.in-addr.arpa."),
        25.. => {
            let (min, max) = range(0);
            format!("{min}-{max}.{b3}.{b2}.{b1}.in-addr.arpa.")
        }
        17.. => {
            let (min, max) = range(8);
            format!("{min}-{max}.{b2}.{b1}.in-addr.arpa.")
        }
        9.. => {
            let (min, max) = range(16);
            format!("{min}-{max}.{b1}.in-addr.arpa.")
        }
        _ => return None,
    })
}

/// The `ip6.arpa.` zone of an IPv6 network, one label a nibble; none for a
/// prefix that is not a whole nibble.
fn v6_reverse(net: &[u8; 16], prefix: u32) -> Option<String> {
    if !prefix.is_multiple_of(4) {
        return None;
    }
    let hex = |n: u8| char::from_digit(u32::from(n), 16).unwrap_or('0');
    let max = (prefix / 8) as usize;
    let mut s = String::new();
    if prefix % 8 == 4 {
        s.push(hex(net[((prefix + 4) / 8 - 1) as usize] >> 4));
        s.push('.');
    }
    for i in 0..max {
        let b = net[max - 1 - i];
        s.push(hex(b & 0xf));
        s.push('.');
        s.push(hex(b >> 4));
        s.push('.');
    }
    s.push_str("ip6.arpa.");
    Some(s)
}

// ---- What an address says ---------------------------------------------------

/// What ipcalc works out for an address and prefix (its `ip_info_st`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Info {
    pub v6: bool,
    pub ip: String,
    pub expanded_ip: Option<String>,
    pub network: String,
    pub expanded_network: Option<String>,
    pub netmask: String,
    pub prefix: u32,
    /// IPv4 only.
    pub broadcast: Option<String>,
    pub reverse_dns: Option<String>,
    pub hostmin: String,
    pub hostmax: String,
    pub space: &'static str,
    /// IPv4 only.
    pub class: Option<&'static str>,
    /// How many addresses there are to use: a number, or "" for all of IPv6.
    pub hosts: String,
}

/// An IPv4 address's info at `prefix`, or the prefix of its class, or /32.
/// The error is the reference's message.
pub fn v4_info(ip: &str, prefix: Option<u32>, class_prefix: bool) -> Result<Info, String> {
    let addr = parse_v4(ip).ok_or_else(|| format!("bad IPv4 address: {ip}"))?;
    let prefix = match prefix {
        Some(p) => p,
        None if class_prefix => v4_class_prefix(addr),
        None => 32,
    };
    if prefix > 32 {
        return Err(format!("bad IPv4 prefix {prefix}"));
    }
    let mask = mask_v4(prefix);
    let network = addr & mask;
    let broadcast = if prefix == 31 {
        u32::MAX
    } else {
        network | !mask
    };
    let (hostmin, hostmax) = if prefix < 32 {
        let min = if prefix <= 30 { network | 1 } else { network };
        let max = network | !mask;
        let max = if prefix <= 30 {
            max.wrapping_sub(1)
        } else {
            max
        };
        (min, max)
    } else {
        (network, network)
    };
    Ok(Info {
        v6: false,
        ip: v4_text(addr),
        expanded_ip: None,
        network: v4_text(network),
        expanded_network: None,
        netmask: v4_text(mask),
        prefix,
        broadcast: Some(v4_text(broadcast)),
        reverse_dns: v4_reverse(network, prefix, broadcast),
        hostmin: v4_text(hostmin),
        hostmax: v4_text(hostmax),
        space: v4_space(network),
        class: Some(v4_class(network)),
        hosts: v4_hosts(prefix),
    })
}

/// An IPv6 address's info at `prefix`, /128 when none is given.
pub fn v6_info(ip: &str, prefix: Option<u32>) -> Result<Info, String> {
    let addr = parse_v6(ip).ok_or_else(|| format!("bad IPv6 address: {ip}"))?;
    let prefix = prefix.unwrap_or(128);
    if prefix > 128 {
        return Err(format!("bad IPv6 prefix: {prefix}"));
    }
    let mask = mask_v6(prefix);
    let mut network = [0u8; 16];
    let mut last = [0u8; 16];
    for i in 0..16 {
        network[i] = addr[i] & mask[i];
        last[i] = network[i] | !mask[i];
    }
    let (hostmin, hostmax) = if prefix < 128 {
        (v6_text(&network), v6_text(&last))
    } else {
        (v6_text(&network), v6_text(&network))
    };
    Ok(Info {
        v6: true,
        ip: v6_text(&addr),
        expanded_ip: Some(v6_expanded(&addr)),
        network: v6_text(&network),
        expanded_network: Some(v6_expanded(&network)),
        netmask: v6_text(&mask),
        prefix,
        broadcast: None,
        reverse_dns: v6_reverse(&network, prefix),
        hostmin,
        hostmax,
        space: v6_space(&network, prefix),
        class: None,
        hosts: v6_hosts(prefix),
    })
}

// ---- Splitting a network ----------------------------------------------------

/// The networks of `/split` in `info`'s network, as ipcalc lists them,
/// one at a time: there may be four billion.
pub enum Split {
    V4 {
        start: u32,
        end: u32,
        diff: u32,
        broadcast: u32,
        prefix: u32,
        done: bool,
    },
    V6 {
        start: [u8; 16],
        end: [u8; 16],
        sdiff: [u8; 16],
        ediff: [u8; 16],
        last: [u8; 16],
        prefix: u32,
        done: bool,
    },
}

fn v6_add(a: &mut [u8; 16], b: &[u8; 16]) {
    let mut carry = 0u16;
    for i in (0..16).rev() {
        let sum = u16::from(a[i]) + u16::from(b[i]) + carry;
        a[i] = sum as u8;
        carry = sum >> 8;
    }
}

/// The error is the reference's message, for a split into larger networks.
pub fn split(info: &Info, split: u32) -> Result<Split, String> {
    let refuse = || {
        format!(
            "Cannot subnet to /{split} with this base network, use a prefix > /{}",
            info.prefix
        )
    };
    if !info.v6 {
        if split > 32 {
            return Err(format!("bad IPv4 prefix: {split}"));
        }
        let splitmask = mask_v4(split);
        if splitmask < mask_v4(info.prefix) {
            return Err(refuse());
        }
        let net = parse_v4(&info.network).unwrap_or(0);
        let diff = 0xffff_ffffu32.wrapping_sub(splitmask).wrapping_add(1);
        let broadcast = match info.prefix {
            31 => net.wrapping_add(1),
            _ => info.broadcast.as_deref().and_then(parse_v4).unwrap_or(0),
        };
        return Ok(Split::V4 {
            start: net,
            end: net.wrapping_add(diff).wrapping_sub(1),
            diff,
            broadcast,
            prefix: split,
            done: false,
        });
    }
    if split > 128 {
        return Err(format!("bad IPv6 prefix: {split}"));
    }
    let net = parse_v6(&info.network).unwrap_or_default();
    let last = parse_v6(&info.hostmax).unwrap_or_default();
    let netmask = mask_v6(info.prefix);
    let splitmask = mask_v6(split);
    if netmask > splitmask {
        return Err(refuse());
    }
    let mut sdiff = [0u8; 16];
    let mut end = [0u8; 16];
    for i in 0..16 {
        sdiff[i] = 0xff - splitmask[i];
        end[i] = net[i].wrapping_add(sdiff[i]);
    }
    let ediff = sdiff;
    let mut one = [0u8; 16];
    one[15] = 1;
    v6_add(&mut sdiff, &one);
    Ok(Split::V6 {
        start: net,
        end,
        sdiff,
        ediff,
        last,
        prefix: split,
        done: false,
    })
}

impl Iterator for Split {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        match self {
            Split::V4 {
                start,
                end,
                diff,
                broadcast,
                prefix,
                done,
            } => {
                if *done {
                    return None;
                }
                let line = format!("{}/{prefix}", v4_text(*start));
                *start = start.wrapping_add(*diff);
                if *end == u32::MAX || *end >= *broadcast {
                    *done = true;
                } else {
                    *end = end.wrapping_add(*diff);
                }
                Some(line)
            }
            Split::V6 {
                start,
                end,
                sdiff,
                ediff,
                last,
                prefix,
                done,
            } => {
                if *done {
                    return None;
                }
                let line = format!("{}/{prefix}", v6_text(start));
                v6_add(start, sdiff);
                // The reference's test: it stops when every pair of bytes
                // of the end has an 0xff, or the end reaches the last address.
                if !(0..16)
                    .step_by(2)
                    .any(|k| end[k] != 0xff && end[k + 1] != 0xff)
                {
                    *done = true;
                }
                if *end >= *last {
                    *done = true;
                }
                *end = *start;
                v6_add(end, ediff);
                Some(line)
            }
        }
    }
}

// ---- Deaggregating a range --------------------------------------------------

/// The fewest networks that cover a range, from the first address, one at
/// a time. The reference runs on forever past the last address (it wraps
/// to the first), and aborts on all of IPv6; this ends there.
pub struct Deaggregate {
    base: u128,
    end: u128,
    bits: u32,
    done: bool,
}

/// A range `A-B` as `-d` takes it: split at `-`, white space trimmed. The
/// error is the reference's message.
pub fn deaggregate(range: &str, v6: bool) -> Result<Deaggregate, String> {
    let bad = || format!("bad deaggregation string: {range}");
    let mut parts = range.split('-').filter(|p| !p.is_empty());
    let first = parts.next().ok_or_else(bad)?.trim_matches(is_c_space);
    let second = parts.next().ok_or_else(bad)?.trim_matches(is_c_space);
    let (base, end, bits) = if v6 {
        let a = parse_v6(first).ok_or_else(|| format!("bad IPv6 address: {first}"))?;
        let b = parse_v6(second).ok_or_else(|| format!("bad IPv6 address: {second}"))?;
        let (a, b) = (u128::from_be_bytes(a), u128::from_be_bytes(b));
        if a > b {
            return Err("bad IPv6 range".to_owned());
        }
        (a, b, 128)
    } else {
        let a = parse_v4(first).ok_or_else(|| format!("bad IPv4 address: {first}"))?;
        let b = parse_v4(second).ok_or_else(|| format!("bad IPv4 address: {second}"))?;
        if a > b {
            return Err("bad range".to_owned());
        }
        (u128::from(a), u128::from(b), 32)
    };
    Ok(Deaggregate {
        base,
        end,
        bits,
        done: false,
    })
}

fn is_c_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r')
}

impl Iterator for Deaggregate {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        if self.done || self.base > self.end {
            return None;
        }
        // The largest aligned block at base that ends at or before end.
        let mut step = 0;
        while step < self.bits && self.base & (1u128 << step) == 0 {
            let low = u128::MAX >> (127 - step);
            let low = if self.bits == 32 {
                low & u128::from(u32::MAX)
            } else {
                low
            };
            let top = self.base | low;
            if top > self.end {
                break;
            }
            step += 1;
        }
        let prefix = self.bits - step;
        let line = if self.bits == 32 {
            format!("{}/{prefix}", v4_text(self.base as u32))
        } else {
            format!("{}/{prefix}", v6_text(&self.base.to_be_bytes()))
        };
        let size = 1u128.checked_shl(step).unwrap_or(0);
        let top = if self.bits == 32 {
            u128::from(u32::MAX)
        } else {
            u128::MAX
        };
        match self.base.checked_add(size) {
            Some(next) if size != 0 && next <= top => self.base = next,
            _ => self.done = true,
        }
        Some(line)
    }
}

/// `-r`: a random private network of `prefix`: in fd00::/8, or 10/8, or
/// for /16 and longer 172.16/12 or 192.168/16 too, as the reference picks.
pub fn random_network(prefix: u32, v6: bool) -> Option<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).ok()?;
    if v6 {
        bytes[0] = 0xfd;
        return Some(format!("{}/{prefix}", v6_text(&bytes)));
    }
    let pick = bytes[15] % 4;
    let mut b = [bytes[0], bytes[1], bytes[2], bytes[3]];
    if prefix >= 16 && pick < 2 {
        if pick == 1 {
            b = [192, 168, b[2], b[3]];
        } else {
            b = [172, 16 | (bytes[14] & 0x0f), b[2], b[3]];
        }
    } else {
        b[0] = 10;
    }
    Some(format!("{}.{}.{}.{}/{prefix}", b[0], b[1], b[2], b[3]))
}

// ---- The app's tools --------------------------------------------------------

static REFERENCE: Option<Reference> = Some(Reference {
    name: "ipcalc",
    version: "1.0.3",
    compat: Compat::DropIn,
});

const fn meta(
    action: &'static str,
    id: &'static str,
    title: &'static str,
    summary: &'static str,
) -> Meta {
    Meta {
        id,
        group: "ipcalc",
        action,
        action_flag: None,
        title,
        summary,
        category: Category::Network,
        keywords: &[
            "ip",
            "cidr",
            "subnet",
            "netmask",
            "network",
            "ipv4",
            "ipv6",
            "prefix",
            "broadcast",
            "ipcalc",
            "range",
        ],
        input: InputKind::Text { trim_newline: true },
        streams: false,
        reference: REFERENCE,
    }
}

static INFO_META: Meta = meta(
    "info",
    "ipcalc.info",
    en!("Network info"),
    en!("An address and prefix's network, netmask, broadcast, hosts and address space"),
);
static SPLIT_META: Meta = meta(
    "split",
    "ipcalc.split",
    en!("Split a network"),
    en!("A network as the smaller networks of a longer prefix"),
);
static DEAGGREGATE_META: Meta = meta(
    "deaggregate",
    "ipcalc.deaggregate",
    en!("Range to networks"),
    en!("A range of addresses as the fewest networks that cover it"),
);

static INFO_PARAMS: &[Param] = &[Param {
    key: "class_prefix",
    label: en!("Class prefix"),
    help: en!("With no prefix, take the IPv4 address class's (/8, /16, /24), not /32"),
    kind: ParamKind::Flag,
    default: Value::Bool(false),
    cli: Cli {
        long: "class-prefix",
        short: None,
    },
}];

static SPLIT_PARAMS: &[Param] = &[Param {
    key: "prefix",
    label: en!("Into"),
    help: en!("The prefix (or IPv4 netmask) of the networks to split into"),
    kind: ParamKind::Text,
    default: Value::Text(String::new()),
    cli: Cli {
        long: "split",
        short: Some('S'),
    },
}];

pub struct NetInfo;
pub struct NetSplit;
pub struct NetDeaggregate;

/// The address, its prefix if given, and whether it is IPv6, as ipcalc
/// takes them: `A/P`, `A/MASK`, or `A MASK` (IPv4).
fn address(input: &str) -> Result<(String, Option<u32>, bool), Error> {
    let input = input.trim();
    if input.is_empty() {
        return Err(Error::usage(
            "give an address: 192.168.1.10/24, 2001:db8::1/64",
        ));
    }
    let mut words = input.split_whitespace();
    let first = words.next().unwrap_or_default();
    let second = words.next();
    let mut v6 = first.contains(':');
    let (ip, prefix_text) = match (first.split_once('/'), second) {
        (_, Some(p)) if v6 => {
            return Err(Error::input(format!("unexpected argument: {p}")));
        }
        (None, p) => (first, p),
        (Some((ip, p)), None) => (ip, Some(p)),
        // As the reference: the second word is the prefix, and the first,
        // slash and all, the address.
        (Some(_), Some(p)) => (first, Some(p)),
    };
    let prefix = match prefix_text {
        Some(p) => Some(str_to_prefix(&mut v6, p, false).ok_or_else(|| {
            Error::input(format!(
                "bad {} prefix: {p}",
                if v6 { "IPv6" } else { "IPv4" }
            ))
        })?),
        None => None,
    };
    Ok((ip.to_owned(), prefix, v6))
}

fn info_of(input: &str, class_prefix: bool) -> Result<Info, Error> {
    let (ip, prefix, v6) = address(input)?;
    if v6 {
        v6_info(&ip, prefix)
    } else {
        v4_info(&ip, prefix, class_prefix)
    }
    .map_err(Error::input)
}

/// The most networks the app lists; `tw ipcalc` lists them all.
const APP_LINES: usize = 1 << 16;

fn lines(ctx: &Ctx, all: impl Iterator<Item = String>) -> Result<Output, Error> {
    let mut out = Vec::new();
    for line in all {
        if ctx.is_interactive() && out.len() == APP_LINES {
            return Err(Error::usage(format!(
                "that is more than {APP_LINES} networks; the app lists that many at most, tw ipcalc all of them"
            )));
        }
        if out.len() % 4096 == 0 {
            ctx.check()?;
        }
        out.push(line);
    }
    Ok(Output::Lines(out))
}

impl Tool for NetInfo {
    fn meta(&self) -> &'static Meta {
        &INFO_META
    }

    fn params(&self) -> &'static [Param] {
        INFO_PARAMS
    }

    /// What `ipcalc --all-info` prints, as named values.
    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let input = String::from_utf8_lossy(input);
        let info = info_of(&input, values.flag(INFO_PARAMS, "class_prefix"))?;
        let mut f = Vec::new();
        if let Some(full) = &info.expanded_ip {
            f.push(Field::new("full_address", "Full address", full));
        }
        f.push(Field::new("address", "Address", &info.ip));
        if let Some(full) = &info.expanded_network {
            f.push(Field::new(
                "full_network",
                "Full network",
                format!("{full}/{}", info.prefix),
            ));
        }
        f.push(Field::new(
            "network",
            "Network",
            format!("{}/{}", info.network, info.prefix),
        ));
        f.push(Field::new("netmask", "Netmask", &info.netmask));
        f.push(Field::new("prefix", "Prefix", info.prefix.to_string()));
        if let Some(b) = &info.broadcast {
            f.push(Field::new("broadcast", "Broadcast", b));
        }
        if let Some(r) = &info.reverse_dns {
            f.push(Field::new("reverse_dns", "Reverse DNS", r));
        }
        f.push(Field::new("address_space", "Address space", info.space));
        if let Some(c) = info.class {
            f.push(Field::new("address_class", "Address class", c));
        }
        f.push(Field::new("min_address", "First host", &info.hostmin));
        f.push(Field::new("max_address", "Last host", &info.hostmax));
        f.push(Field::new("addresses", "Hosts", &info.hosts));
        Ok(Output::Fields(f))
    }

    fn detect(&self, sample: &[u8]) -> Option<Score> {
        let s = std::str::from_utf8(sample).ok()?.trim();
        let (ip, p) = s.split_once('/')?;
        let prefix: u32 = p.parse().ok()?;
        let ok =
            (parse_v4(ip).is_some() && prefix <= 32) || (parse_v6(ip).is_some() && prefix <= 128);
        ok.then_some(Score(80))
    }

    /// `ipcalc --all-info ADDRESS`.
    fn command_args(&self, values: &Values) -> Option<Vec<String>> {
        let mut args = vec!["ipcalc".to_owned(), "--all-info".to_owned()];
        if values.flag(INFO_PARAMS, "class_prefix") {
            args.push("--class-prefix".to_owned());
        }
        Some(args)
    }

    fn input_in_args(&self) -> bool {
        true
    }
}

impl Tool for NetSplit {
    fn meta(&self) -> &'static Meta {
        &SPLIT_META
    }

    fn params(&self) -> &'static [Param] {
        SPLIT_PARAMS
    }

    fn run(&self, ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let input = String::from_utf8_lossy(input);
        let into = values.text(SPLIT_PARAMS, "prefix");
        if into.trim().is_empty() {
            return Err(Error::usage(
                "give the prefix to split into: 26, or 255.255.255.192",
            ));
        }
        let info = info_of(&input, false)?;
        let mut v6 = info.v6;
        let p = str_to_prefix(&mut v6, into.trim(), true).ok_or_else(|| {
            Error::usage(format!(
                "bad {} prefix: {into}",
                if v6 { "IPv6" } else { "IPv4" }
            ))
        })?;
        if v6 != info.v6 {
            return Err(Error::usage(format!("an IPv4 network has no /{p}")));
        }
        lines(ctx, split(&info, p).map_err(Error::usage)?)
    }

    /// `ipcalc -S PREFIX ADDRESS`.
    fn command_args(&self, values: &Values) -> Option<Vec<String>> {
        Some(vec![
            "ipcalc".to_owned(),
            "-S".to_owned(),
            values.text(SPLIT_PARAMS, "prefix"),
        ])
    }

    fn input_in_args(&self) -> bool {
        true
    }
}

impl Tool for NetDeaggregate {
    fn meta(&self) -> &'static Meta {
        &DEAGGREGATE_META
    }

    fn run(&self, ctx: &Ctx, input: &[u8], _values: &Values) -> Result<Output, Error> {
        let input = String::from_utf8_lossy(input);
        let range = input.trim();
        if range.is_empty() {
            return Err(Error::usage("give a range: 10.0.0.5-10.0.0.70"));
        }
        lines(
            ctx,
            deaggregate(range, range.contains(':')).map_err(Error::input)?,
        )
    }

    /// `ipcalc -d RANGE`.
    fn command_args(&self, _values: &Values) -> Option<Vec<String>> {
        Some(vec!["ipcalc".to_owned(), "-d".to_owned()])
    }

    fn input_in_args(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_read_and_write_as_glibc_does() {
        assert_eq!(parse_v4("192.168.1.1"), Some(0xc0a8_0101));
        for bad in [
            "01.2.3.4",
            "1.2.3",
            "1.2.3.4.",
            "256.1.1.1",
            "1..2.3",
            "",
            " 1.2.3.4",
        ] {
            assert_eq!(parse_v4(bad), None, "{bad}");
        }
        for (text, shown) in [
            ("::", "::"),
            ("::1", "::1"),
            ("::2", "::2"),
            ("::1.2.3.4", "::1.2.3.4"),
            ("::ffff:1.2.3.4", "::ffff:1.2.3.4"),
            ("2001:db8:0:0:1:0:0:1", "2001:db8::1:0:0:1"),
            ("2001:0db8:0000:0000:0000:0000:0000:0001", "2001:db8::1"),
            ("1:0:0:1:0:0:0:1", "1:0:0:1::1"),
            ("fe80::", "fe80::"),
            ("1:2:3:4:5:6:7:8", "1:2:3:4:5:6:7:8"),
            ("1::", "1::"),
        ] {
            let a = parse_v6(text).unwrap_or_else(|| panic!("{text}"));
            assert_eq!(v6_text(&a), shown, "{text}");
        }
        for bad in [
            ":",
            ":::",
            "1:::2",
            "1::2::3",
            "12345::",
            "1:2:3:4:5:6:7:8:9",
            "::g",
            "1:",
            "1:2:3:4:5:6:7::8",
        ] {
            assert_eq!(parse_v6(bad), None, "{bad}");
        }
    }

    #[test]
    fn prefixes_read_as_safe_atoi_reads_them() {
        assert_eq!(safe_atoi("24"), Some(24));
        assert_eq!(safe_atoi("0x18"), Some(24));
        assert_eq!(safe_atoi("030"), Some(24));
        assert_eq!(safe_atoi("08"), None);
        assert_eq!(safe_atoi("-1"), Some(-1));
        assert_eq!(safe_atoi(" 7"), Some(7));
        assert_eq!(safe_atoi("7 "), None);
        assert_eq!(safe_atoi("0x"), None);
        assert_eq!(safe_atoi("99999999999"), None);
        let mut v6 = false;
        assert_eq!(str_to_prefix(&mut v6, "255.255.255.0", false), Some(24));
        assert_eq!(str_to_prefix(&mut v6, "255.0.255.0", false), None);
        assert_eq!(str_to_prefix(&mut v6, "64", true), Some(64));
        assert!(v6);
    }

    #[test]
    fn info_is_the_references() {
        let i = v4_info("192.168.1.77", Some(26), false).unwrap_or_default();
        assert_eq!(
            (
                i.network.as_str(),
                i.netmask.as_str(),
                i.broadcast.as_deref()
            ),
            ("192.168.1.64", "255.255.255.192", Some("192.168.1.127"))
        );
        assert_eq!(
            (i.hostmin.as_str(), i.hostmax.as_str(), i.hosts.as_str()),
            ("192.168.1.65", "192.168.1.126", "62")
        );
        assert_eq!(
            i.reverse_dns.as_deref(),
            Some("64-127.1.168.192.in-addr.arpa.")
        );
        assert_eq!(v4_hosts(0), "4294967295");
        let i = v6_info("2001:db8::1", Some(64)).unwrap_or_default();
        assert_eq!(i.hostmax, "2001:db8::ffff:ffff:ffff:ffff");
        assert_eq!(i.space, "Documentation");
        assert_eq!(i.hosts, "18446744073709551616");
        assert_eq!(v6_hosts(0), "");
    }

    #[test]
    fn splits_and_ranges_list_as_the_reference_does() {
        let i = v4_info("192.168.1.0", Some(26), false).unwrap_or_default();
        let nets: Vec<String> = split(&i, 28).map(Iterator::collect).unwrap_or_default();
        assert_eq!(
            nets,
            [
                "192.168.1.0/28",
                "192.168.1.16/28",
                "192.168.1.32/28",
                "192.168.1.48/28"
            ]
        );
        let i = v6_info("2001:db8::", Some(64)).unwrap_or_default();
        let nets: Vec<String> = split(&i, 66).map(Iterator::collect).unwrap_or_default();
        assert_eq!(
            nets,
            [
                "2001:db8::/66",
                "2001:db8:0:0:4000::/66",
                "2001:db8:0:0:8000::/66",
                "2001:db8:0:0:c000::/66"
            ]
        );
        let d: Vec<String> = deaggregate("10.0.0.5-10.0.0.70", false)
            .map(Iterator::collect)
            .unwrap_or_default();
        assert_eq!(
            d,
            [
                "10.0.0.5/32",
                "10.0.0.6/31",
                "10.0.0.8/29",
                "10.0.0.16/28",
                "10.0.0.32/27",
                "10.0.0.64/30",
                "10.0.0.68/31",
                "10.0.0.70/32"
            ]
        );
        // A split past 32 bits of an IPv4 network is refused, not a panic.
        let i = v4_info("2.2.1.3", None, true).unwrap_or_default();
        assert!(split(&i, 40).is_err());
        // The whole space, which the reference never finishes.
        let all: Vec<String> = deaggregate("0.0.0.0-255.255.255.255", false)
            .map(Iterator::collect)
            .unwrap_or_default();
        assert_eq!(all, ["0.0.0.0/0"]);
        let top: Vec<String> = deaggregate("255.255.255.0-255.255.255.255", false)
            .map(Iterator::collect)
            .unwrap_or_default();
        assert_eq!(top, ["255.255.255.0/24"]);
        let all6: Vec<String> = deaggregate("::-ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", true)
            .map(Iterator::collect)
            .unwrap_or_default();
        assert_eq!(all6, ["::/0"]);
    }
}
