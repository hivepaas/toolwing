//! Hashes: `md5sum`, `sha1sum`, `sha256sum`, `sha512sum` and `b2sum`, as GNU
//! coreutils 9.7 writes and checks them (drop-in: tools.md), and BLAKE3 for
//! the app's page.
//!
//! The lines follow GNU's rules, checked against it: `HEX  NAME` (`HEX *NAME`
//! in binary mode) or `TAG (NAME) = HEX`; a name with a backslash, a newline
//! or a carriage return is escaped and its line starts with a backslash. A
//! checksum line may have one space or two, upper or lower case hex, and a
//! CRLF ending.

use std::io::Read;

use sha2::Digest as _;
use toolwing_core::{
    Category, Compat, Ctx, Error, InputKind, Meta, Output, Reference, Score, Tool, Values, en,
};

/// How many bytes are read at a time.
const CHUNK: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    Md5,
    Sha1,
    Sha256,
    Sha512,
    /// BLAKE2b with an output of this many bits: a multiple of 8, up to 512.
    Blake2b(u16),
    Blake3,
}

impl Algorithm {
    /// The ones the app's page shows, in its order.
    pub const ALL: [Algorithm; 6] = [
        Algorithm::Md5,
        Algorithm::Sha1,
        Algorithm::Sha256,
        Algorithm::Sha512,
        Algorithm::Blake2b(512),
        Algorithm::Blake3,
    ];

    /// The name people know it by.
    pub fn title(self) -> String {
        match self {
            Algorithm::Md5 => "MD5".into(),
            Algorithm::Sha1 => "SHA-1".into(),
            Algorithm::Sha256 => "SHA-256".into(),
            Algorithm::Sha512 => "SHA-512".into(),
            Algorithm::Blake2b(512) => "BLAKE2b".into(),
            Algorithm::Blake2b(bits) => format!("BLAKE2b-{bits}"),
            Algorithm::Blake3 => "BLAKE3".into(),
        }
    }

    /// The name in `--tag` lines, as GNU writes it.
    pub fn tag(self) -> String {
        match self {
            Algorithm::Md5 => "MD5".into(),
            Algorithm::Sha1 => "SHA1".into(),
            Algorithm::Sha256 => "SHA256".into(),
            Algorithm::Sha512 => "SHA512".into(),
            Algorithm::Blake2b(512) => "BLAKE2b".into(),
            Algorithm::Blake2b(bits) => format!("BLAKE2b-{bits}"),
            Algorithm::Blake3 => "BLAKE3".into(),
        }
    }

    pub fn digest_len(self) -> usize {
        match self {
            Algorithm::Md5 => 16,
            Algorithm::Sha1 => 20,
            Algorithm::Sha256 | Algorithm::Blake3 => 32,
            Algorithm::Sha512 => 64,
            Algorithm::Blake2b(bits) => usize::from(bits / 8),
        }
    }
}

/// A digest being computed.
pub struct Hasher(State);

enum State {
    Md5(md5::Md5),
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
    Blake2b(Box<blake2b_simd::State>),
    Blake3(Box<blake3::Hasher>),
}

impl Hasher {
    /// `None` for a BLAKE2b length that is not a multiple of 8 from 8 to 512.
    pub fn new(algorithm: Algorithm) -> Option<Hasher> {
        Some(Hasher(match algorithm {
            Algorithm::Md5 => State::Md5(md5::Md5::new()),
            Algorithm::Sha1 => State::Sha1(sha1::Sha1::new()),
            Algorithm::Sha256 => State::Sha256(sha2::Sha256::new()),
            Algorithm::Sha512 => State::Sha512(sha2::Sha512::new()),
            Algorithm::Blake2b(bits) => {
                if bits == 0 || !bits.is_multiple_of(8) || bits > 512 {
                    return None;
                }
                State::Blake2b(Box::new(
                    blake2b_simd::Params::new()
                        .hash_length(usize::from(bits / 8))
                        .to_state(),
                ))
            }
            Algorithm::Blake3 => State::Blake3(Box::new(blake3::Hasher::new())),
        }))
    }

    pub fn update(&mut self, data: &[u8]) {
        match &mut self.0 {
            State::Md5(h) => sha2::Digest::update(h, data),
            State::Sha1(h) => sha2::Digest::update(h, data),
            State::Sha256(h) => sha2::Digest::update(h, data),
            State::Sha512(h) => sha2::Digest::update(h, data),
            State::Blake2b(h) => {
                h.update(data);
            }
            State::Blake3(h) => {
                h.update(data);
            }
        }
    }

    pub fn finish(self) -> Vec<u8> {
        match self.0 {
            State::Md5(h) => h.finalize().to_vec(),
            State::Sha1(h) => h.finalize().to_vec(),
            State::Sha256(h) => h.finalize().to_vec(),
            State::Sha512(h) => h.finalize().to_vec(),
            State::Blake2b(h) => h.finalize().as_bytes().to_vec(),
            State::Blake3(h) => h.finalize().as_bytes().to_vec(),
        }
    }
}

/// The digest of everything `input` gives, in constant memory.
pub fn digest_reader(
    ctx: &Ctx,
    algorithm: Algorithm,
    input: &mut dyn Read,
) -> Result<Vec<u8>, Error> {
    let mut hasher = Hasher::new(algorithm).ok_or_else(|| Error::usage("invalid length"))?;
    let mut buf = vec![0; CHUNK];
    loop {
        ctx.check()?;
        let n = match input.read(&mut buf) {
            Ok(0) => return Ok(hasher.finish()),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        };
        hasher.update(&buf[..n]);
    }
}

/// BLAKE3's modes (b3sum's `--keyed` and `--derive-key`).
pub enum Blake3Mode {
    Hash,
    /// With a 32-byte key.
    Keyed([u8; 32]),
    /// Keys derived for a context string.
    DeriveKey(String),
}

/// BLAKE3 of everything `input` gives, `len` bytes of its output from byte
/// `seek` on (BLAKE3's output is as long as asked for), in constant memory.
pub fn blake3_reader(
    ctx: &Ctx,
    mode: &Blake3Mode,
    input: &mut dyn Read,
    seek: u64,
    len: usize,
) -> Result<Vec<u8>, Error> {
    let mut hasher = match mode {
        Blake3Mode::Hash => blake3::Hasher::new(),
        Blake3Mode::Keyed(key) => blake3::Hasher::new_keyed(key),
        Blake3Mode::DeriveKey(context) => blake3::Hasher::new_derive_key(context),
    };
    let mut buf = vec![0; CHUNK];
    loop {
        ctx.check()?;
        let n = match input.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        };
        hasher.update(&buf[..n]);
    }
    let mut out = vec![0; len];
    let mut reader = hasher.finalize_xof();
    reader.set_position(seek);
    reader.fill(&mut out);
    Ok(out)
}

/// The digests of several algorithms in one pass over `input`, as the app's
/// Hashes page needs them: a file is read once, not once per algorithm.
pub fn digest_all(
    ctx: &Ctx,
    algorithms: &[Algorithm],
    input: &mut dyn Read,
) -> Result<Vec<Vec<u8>>, Error> {
    let mut hashers = algorithms
        .iter()
        .map(|&a| Hasher::new(a).ok_or_else(|| Error::usage("invalid length")))
        .collect::<Result<Vec<_>, _>>()?;
    let mut buf = vec![0; CHUNK];
    loop {
        ctx.check()?;
        let n = match input.read(&mut buf) {
            Ok(0) => return Ok(hashers.into_iter().map(Hasher::finish).collect()),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        };
        for h in &mut hashers {
            h.update(&buf[..n]);
        }
    }
}

pub fn hex(digest: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(digest)
}

pub fn base64(digest: &[u8]) -> String {
    data_encoding::BASE64.encode(digest)
}

/// The algorithm and digest a pasted hash is, among `digests`: hex in any
/// case, or Base64; a whole `HEX  NAME` line is read for its first word.
pub fn find_match(expected: &str, digests: &[(Algorithm, Vec<u8>)]) -> Option<Algorithm> {
    let word = expected.split_whitespace().next()?;
    let bytes = data_encoding::HEXLOWER_PERMISSIVE
        .decode(word.as_bytes())
        .or_else(|_| data_encoding::BASE64.decode(word.as_bytes()))
        .ok()?;
    digests.iter().find(|(_, d)| *d == bytes).map(|(a, _)| *a)
}

/// The command that writes this algorithm's lines, when there is one.
pub fn command_of(algorithm: Algorithm) -> Option<&'static str> {
    match algorithm {
        Algorithm::Md5 => Some("md5sum"),
        Algorithm::Sha1 => Some("sha1sum"),
        Algorithm::Sha256 => Some("sha256sum"),
        Algorithm::Sha512 => Some("sha512sum"),
        Algorithm::Blake2b(_) => Some("b2sum"),
        Algorithm::Blake3 => None,
    }
}

/// How a line is written.
#[derive(Clone, Copy, Debug, Default)]
pub struct Style {
    /// BSD style: `TAG (NAME) = HEX`.
    pub tag: bool,
    /// `*` before the name instead of a space (`-b`).
    pub binary: bool,
    /// End with NUL and leave the name as it is (`-z`).
    pub zero: bool,
}

/// One output line, with its ending.
pub fn format_line(algorithm: Algorithm, digest: &[u8], name: &str, style: Style) -> Vec<u8> {
    let escape = !style.zero && needs_escape(name);
    let name = if escape {
        escape_name(name)
    } else {
        name.to_owned()
    };
    let mut line = String::new();
    if escape {
        line.push('\\');
    }
    if style.tag {
        line.push_str(&format!("{} ({name}) = {}", algorithm.tag(), hex(digest)));
    } else {
        let mark = if style.binary { '*' } else { ' ' };
        line.push_str(&format!("{} {mark}{name}", hex(digest)));
    }
    let mut bytes = line.into_bytes();
    bytes.push(if style.zero { 0 } else { b'\n' });
    bytes
}

pub fn needs_escape(name: &str) -> bool {
    name.contains(['\\', '\n', '\r'])
}

pub fn escape_name(name: &str) -> String {
    name.replace('\\', r"\\")
        .replace('\n', r"\n")
        .replace('\r', r"\r")
}

/// GNU's `filename_unescape`: only `\\`, `\n` and `\r`.
fn unescape_name(name: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(name.len());
    let mut bytes = name.iter();
    while let Some(&b) = bytes.next() {
        if b != b'\\' {
            out.push(b);
            continue;
        }
        out.push(match bytes.next()? {
            b'\\' => b'\\',
            b'n' => b'\n',
            b'r' => b'\r',
            _ => return None,
        });
    }
    Some(out)
}

/// A checksum line read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checksum {
    pub algorithm: Algorithm,
    /// The expected digest.
    pub digest: Vec<u8>,
    pub name: String,
}

/// What a line of a checksum list is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    /// Empty, or a `#` comment: skipped, even with `--strict`.
    Skip,
    Checksum(Checksum),
    Improper,
}

/// Whether the lines read so far had one separator (`HEX NAME`) or the usual
/// two (`HEX  NAME`, `HEX *NAME`): GNU's `bsd_reversed`. It holds for the whole
/// run, every list included, and a line of the other kind is improper, so an
/// attacker cannot hide a file behind a name with a leading space.
#[derive(Clone, Copy, Debug, Default)]
pub struct Separators(Option<bool>);

fn is_white(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// Reads one line of a checksum list (without its `\n`) for `family`, the
/// command's algorithm, as GNU coreutils 9.7's `split_3` does; for `b2sum`
/// the length comes from the line.
pub fn parse_line(line: &[u8], family: Algorithm, separators: &mut Separators) -> Line {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.is_empty() || line[0] == b'#' {
        return Line::Skip;
    }
    match split(line, family, separators) {
        Some(c) => Line::Checksum(c),
        None => Line::Improper,
    }
}

fn split(s: &[u8], family: Algorithm, separators: &mut Separators) -> Option<Checksum> {
    let mut i = s.iter().take_while(|&&b| is_white(b)).count();
    let escaped = s.get(i) == Some(&b'\\');
    if escaped {
        i += 1;
    }
    let tag = match family {
        Algorithm::Blake2b(_) => "BLAKE2b".to_owned(),
        other => other.tag(),
    };
    if s[i..].starts_with(tag.as_bytes()) {
        return split_tagged(&s[i + tag.len()..], family, escaped);
    }
    let rest = &s[i..];
    let hex_len = match family {
        Algorithm::Blake2b(_) => {
            let n = rest.iter().take_while(|b| b.is_ascii_hexdigit()).count();
            if n < 2 || !n.is_multiple_of(2) || n > 128 {
                return None;
            }
            n
        }
        other => other.digest_len() * 2,
    };
    let token = rest.iter().take_while(|&&b| !is_white(b)).count();
    // The digest must be followed by at least one white space.
    if token == rest.len() || token != hex_len {
        return None;
    }
    let algorithm = match family {
        Algorithm::Blake2b(_) => Algorithm::Blake2b(u16::try_from(hex_len * 4).ok()?),
        other => other,
    };
    let digest = data_encoding::HEXLOWER_PERMISSIVE
        .decode(&rest[..token])
        .ok()?;
    let mut name = &rest[token + 1..];
    let one_separator = name.len() == 1 || !matches!(name.first(), Some(b' ' | b'*'));
    if one_separator {
        if separators.0 == Some(false) {
            return None;
        }
        separators.0 = Some(true);
    } else if separators.0 != Some(true) {
        separators.0 = Some(false);
        name = &name[1..];
    }
    let name = if escaped {
        unescape_name(name)?
    } else {
        name.to_vec()
    };
    Some(Checksum {
        algorithm,
        digest,
        name: String::from_utf8_lossy(&name).into_owned(),
    })
}

/// After the tag: `-BITS` for BLAKE2b, then ` (NAME) = HEX`, the name ending
/// at the line's last `)`.
fn split_tagged(s: &[u8], family: Algorithm, escaped: bool) -> Option<Checksum> {
    let mut i = 0;
    let algorithm = match family {
        Algorithm::Blake2b(_) => {
            if s.first() == Some(&b'-') {
                let digits = s[1..].iter().take_while(|b| b.is_ascii_digit()).count();
                let bits: u16 = std::str::from_utf8(&s[1..1 + digits]).ok()?.parse().ok()?;
                if bits == 0 || bits > 512 || !bits.is_multiple_of(8) {
                    return None;
                }
                i = 1 + digits;
                Algorithm::Blake2b(bits)
            } else {
                Algorithm::Blake2b(512)
            }
        }
        other => other,
    };
    if s.get(i) == Some(&b' ') {
        i += 1;
    }
    if s.get(i) != Some(&b'(') {
        return None;
    }
    let s = &s[i + 1..];
    let close = s.iter().rposition(|&b| b == b')')?;
    let name = &s[..close];
    let name = if escaped {
        unescape_name(name)?
    } else {
        name.to_vec()
    };
    let mut rest = &s[close + 1..];
    rest = &rest[rest.iter().take_while(|&&b| is_white(b)).count()..];
    rest = rest.strip_prefix(b"=")?;
    rest = &rest[rest.iter().take_while(|&&b| is_white(b)).count()..];
    if rest.len() != algorithm.digest_len() * 2 {
        return None;
    }
    let digest = data_encoding::HEXLOWER_PERMISSIVE.decode(rest).ok()?;
    Some(Checksum {
        algorithm,
        digest,
        name: String::from_utf8_lossy(&name).into_owned(),
    })
}

/// The registry's hash tools: one per command, so `tw list` and the app know
/// them. The CLI writes their command lines by hand (`toolwing-cli`'s `sums`).
pub struct Sum {
    meta: &'static Meta,
    algorithm: Algorithm,
}

impl Sum {
    pub fn algorithm(&self) -> Algorithm {
        self.algorithm
    }
}

const fn sum_meta(command: &'static str, title: &'static str, summary: &'static str) -> Meta {
    Meta {
        id: command,
        group: command,
        action: "",
        action_flag: None,
        title,
        summary,
        category: Category::Crypto,
        keywords: &["hash", "checksum", "digest", "sum"],
        input: InputKind::Bytes,
        streams: true,
        reference: Some(Reference {
            name: "GNU coreutils",
            version: "9.7",
            compat: Compat::DropIn,
        }),
    }
}

static MD5_META: Meta = sum_meta(
    "md5sum",
    en!("MD5"),
    en!("MD5 checksums of files, or check them"),
);
static SHA1_META: Meta = sum_meta(
    "sha1sum",
    en!("SHA-1"),
    en!("SHA-1 checksums of files, or check them"),
);
static SHA256_META: Meta = sum_meta(
    "sha256sum",
    en!("SHA-256"),
    en!("SHA-256 checksums of files, or check them"),
);
static SHA512_META: Meta = sum_meta(
    "sha512sum",
    en!("SHA-512"),
    en!("SHA-512 checksums of files, or check them"),
);
static B2_META: Meta = sum_meta(
    "b2sum",
    en!("BLAKE2b"),
    en!("BLAKE2b checksums of files, or check them"),
);

pub static MD5SUM: Sum = Sum {
    meta: &MD5_META,
    algorithm: Algorithm::Md5,
};
pub static SHA1SUM: Sum = Sum {
    meta: &SHA1_META,
    algorithm: Algorithm::Sha1,
};
pub static SHA256SUM: Sum = Sum {
    meta: &SHA256_META,
    algorithm: Algorithm::Sha256,
};
pub static SHA512SUM: Sum = Sum {
    meta: &SHA512_META,
    algorithm: Algorithm::Sha512,
};
pub static B2SUM: Sum = Sum {
    meta: &B2_META,
    algorithm: Algorithm::Blake2b(512),
};

static B3_META: Meta = Meta {
    reference: Some(Reference {
        name: "b3sum",
        version: "1.8.2",
        compat: Compat::DropIn,
    }),
    ..sum_meta(
        "b3sum",
        en!("BLAKE3"),
        en!("BLAKE3 checksums of files, or check them, as b3sum does"),
    )
};

/// `b3sum`: BLAKE3, with b3sum's own command line (not coreutils'), so not
/// among [`SUMS`]; the app's Hashes page has BLAKE3 already.
pub static B3SUM: Sum = Sum {
    meta: &B3_META,
    algorithm: Algorithm::Blake3,
};

/// The hash commands, for the CLI and the app's Hashes page.
pub static SUMS: [&Sum; 5] = [&MD5SUM, &SHA1SUM, &SHA256SUM, &SHA512SUM, &B2SUM];

impl Tool for Sum {
    fn meta(&self) -> &'static Meta {
        self.meta
    }

    /// The line `printf … | tw sha256sum` writes.
    fn run(&self, ctx: &Ctx, input: &[u8], _values: &Values) -> Result<Output, Error> {
        let digest = digest_reader(ctx, self.algorithm, &mut &input[..])?;
        let line = format_line(self.algorithm, &digest, "-", Style::default());
        Ok(Output::Text(String::from_utf8_lossy(&line).into_owned()))
    }

    fn detect(&self, sample: &[u8]) -> Option<Score> {
        let s = std::str::from_utf8(sample).ok()?.trim();
        let hex_len = self.algorithm.digest_len() * 2;
        (s.len() == hex_len && s.bytes().all(|b| b.is_ascii_hexdigit())).then_some(Score(60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(algorithm: Algorithm, data: &[u8]) -> String {
        hex(&digest_reader(&Ctx::new(), algorithm, &mut &data[..]).unwrap_or_default())
    }

    /// The values GNU coreutils 9.7 gives for "hello world".
    /// b3sum 1.8.2's values for "abc": plain, keyed with 32 zeros, derived for
    /// "ctx", 8 bytes, and 4 from byte 2.
    #[test]
    fn blake3_modes_match_b3sum() {
        let b3 = |mode: &Blake3Mode, seek, len| {
            hex(&blake3_reader(&Ctx::new(), mode, &mut &b"abc"[..], seek, len).unwrap_or_default())
        };
        let plain = "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85";
        assert_eq!(b3(&Blake3Mode::Hash, 0, 32), plain);
        assert_eq!(b3(&Blake3Mode::Hash, 0, 8), "6437b3ac38465133");
        assert_eq!(b3(&Blake3Mode::Hash, 2, 4), "b3ac3846");
        assert_eq!(b3(&Blake3Mode::Hash, 0, 0), "");
        let keyed = Blake3Mode::Keyed(*b"00000000000000000000000000000000");
        assert_eq!(
            b3(&keyed, 0, 32),
            "91584f1fb59612f3d47c42ae41005f005e09f1f388f7112d4931f39d8790ef83"
        );
        let derived = Blake3Mode::DeriveKey("ctx".into());
        assert_eq!(
            b3(&derived, 0, 32),
            "51b03b2a289e3f32acf87f00c3620dd6cc2c29706de6f8184386550dab656d36"
        );
    }

    #[test]
    fn digests_match_coreutils() {
        let d = b"hello world";
        assert_eq!(of(Algorithm::Md5, d), "5eb63bbbe01eeed093cb22bb8f5acdc3");
        assert_eq!(
            of(Algorithm::Sha1, d),
            "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed"
        );
        assert_eq!(
            of(Algorithm::Sha256, d),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
        assert_eq!(
            of(Algorithm::Blake2b(256), d),
            "256c83b297114d201b30179f3f0ef0cace9783622da5974326b436178aeef610"
        );
        assert!(of(Algorithm::Blake2b(512), d).starts_with("021ced8799296ceca557832ab941a50b"));
        assert!(Hasher::new(Algorithm::Blake2b(7)).is_none());
        // BLAKE3's own test value for "hello world".
        assert_eq!(
            of(Algorithm::Blake3, d),
            "d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24"
        );
    }

    #[test]
    fn one_pass_gives_every_digest_and_finds_a_pasted_one() {
        let all =
            digest_all(&Ctx::new(), &Algorithm::ALL, &mut &b"hello world"[..]).unwrap_or_default();
        assert_eq!(hex(&all[2]), of(Algorithm::Sha256, b"hello world"));
        let pairs: Vec<_> = Algorithm::ALL.iter().copied().zip(all).collect();
        let sha256 = "B94D27B9934D3E08A52E52D7DA7DABFAC484EFE37A5380EE9088F7ACE2EFCDE9  a.txt";
        assert_eq!(find_match(sha256, &pairs), Some(Algorithm::Sha256));
        assert_eq!(
            find_match(&base64(&pairs[0].1), &pairs),
            Some(Algorithm::Md5)
        );
        assert_eq!(find_match("abcd", &pairs), None);
        assert_eq!(find_match("", &pairs), None);
    }

    #[test]
    fn lines_are_written_as_gnu_writes_them() {
        let d = [0xab; 16];
        let line = |name, style| String::from_utf8(format_line(Algorithm::Md5, &d, name, style));
        let ab = "abababababababababababababababab";
        assert_eq!(
            line("a.txt", Style::default()),
            Ok(format!("{ab}  a.txt\n"))
        );
        let binary = Style {
            binary: true,
            ..Style::default()
        };
        assert_eq!(line("a.txt", binary), Ok(format!("{ab} *a.txt\n")));
        let tag = Style {
            tag: true,
            ..Style::default()
        };
        assert_eq!(line("a.txt", tag), Ok(format!("MD5 (a.txt) = {ab}\n")));
        assert_eq!(
            line("back\\slash", Style::default()),
            Ok(format!("\\{ab}  back\\\\slash\n"))
        );
        assert_eq!(
            line("cr\rname", Style::default()),
            Ok(format!("\\{ab}  cr\\rname\n"))
        );
        let zero = Style {
            zero: true,
            ..Style::default()
        };
        assert_eq!(line("new\nline", zero), Ok(format!("{ab}  new\nline\0")));
    }

    #[test]
    fn checksum_lines_are_read_as_gnu_reads_them() {
        let hex = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        let name = |line: &str| match parse_line(
            line.as_bytes(),
            Algorithm::Sha256,
            &mut Separators::default(),
        ) {
            Line::Checksum(c) => Some(c.name),
            _ => None,
        };
        assert_eq!(name(&format!("{hex}  a.txt")).as_deref(), Some("a.txt"));
        assert_eq!(name(&format!("{hex} *a.txt")).as_deref(), Some("a.txt"));
        assert_eq!(name(&format!("{hex} a.txt")).as_deref(), Some("a.txt"));
        assert_eq!(name(&format!("  {hex}\t a.txt")).as_deref(), Some("a.txt"));
        assert_eq!(
            name(&format!("{hex}  we ird.txt\r")).as_deref(),
            Some("we ird.txt")
        );
        assert_eq!(
            name(&format!("{}  a.txt", hex.to_uppercase())).as_deref(),
            Some("a.txt")
        );
        assert_eq!(
            name(&format!("SHA256 (a.txt) = {hex}")).as_deref(),
            Some("a.txt")
        );
        assert_eq!(
            name(&format!("SHA256(a (1).txt)= {hex}")).as_deref(),
            Some("a (1).txt")
        );
        assert_eq!(
            name(&format!("\\{hex}  back\\\\slash")).as_deref(),
            Some("back\\slash")
        );
        assert_eq!(name(&format!("\\{hex}  bad\\escape")), None);
        assert_eq!(name(&format!("MD5 (a.txt) = {hex}")), None);
        assert_eq!(name(&format!("{}  a.txt", &hex[2..])), None);
        assert_eq!(name(hex), None);
        let skip = |line: &str| {
            parse_line(
                line.as_bytes(),
                Algorithm::Sha256,
                &mut Separators::default(),
            )
        };
        assert_eq!(skip("# comment"), Line::Skip);
        assert_eq!(skip(""), Line::Skip);
        assert_eq!(skip("\r"), Line::Skip);
        assert_eq!(skip("   "), Line::Improper);
        assert_eq!(skip(" # not a comment"), Line::Improper);
        assert_eq!(skip("garbage"), Line::Improper);
    }

    /// GNU coreutils 9.7: after a line with one separator, a line with two
    /// keeps its space or star in the name; the other way round, a line with
    /// one separator is improper.
    #[test]
    fn the_first_line_decides_the_separator() {
        let hex = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        let names = |lines: &[String]| {
            let mut sep = Separators::default();
            lines
                .iter()
                .map(
                    |l| match parse_line(l.as_bytes(), Algorithm::Sha256, &mut sep) {
                        Line::Checksum(c) => Some(c.name),
                        _ => None,
                    },
                )
                .collect::<Vec<_>>()
        };
        let one = format!("{hex} a.txt");
        let two = format!("{hex}  b.txt");
        let star = format!("{hex} *c.txt");
        assert_eq!(
            names(&[one.clone(), two.clone(), star.clone()]),
            [
                Some("a.txt".into()),
                Some(" b.txt".into()),
                Some("*c.txt".into())
            ]
        );
        assert_eq!(
            names(&[two, one, star]),
            [Some("b.txt".into()), None, Some("c.txt".into())]
        );
    }

    #[test]
    fn b2sum_lines_carry_their_length() {
        let h256 = "256c83b297114d201b30179f3f0ef0cace9783622da5974326b436178aeef610";
        let alg = |line: &str| match parse_line(
            line.as_bytes(),
            Algorithm::Blake2b(512),
            &mut Separators::default(),
        ) {
            Line::Checksum(c) => Some(c.algorithm),
            _ => None,
        };
        assert_eq!(
            alg(&format!("{h256}  a.txt")),
            Some(Algorithm::Blake2b(256))
        );
        assert_eq!(
            alg(&format!("BLAKE2b-256 (a.txt) = {h256}")),
            Some(Algorithm::Blake2b(256))
        );
        assert_eq!(alg(&format!("BLAKE2b (a.txt) = {h256}")), None);
        assert_eq!(alg(&format!("BLAKE2b-7 (a.txt) = {h256}")), None);
        assert_eq!(alg(&format!("{}  a.txt", &h256[..3])), None);
    }
}
