//! Password hashes: `bcrypt` (hash and verify, as OpenBSD's bcrypt and
//! `htpasswd -B` make them) and `argon2` (as the reference C's `argon2`
//! command makes them, RFC 9106). The password is the input, and a secret:
//! the app masks it, and "copy as command" reads it from TW_SECRET. Both are
//! slow by design, so the app runs them on Ctrl+Enter, with the costs held
//! to limits a run that cannot be cancelled can afford (architecture.md).

pub use argon2::Algorithm;
use argon2::{Argon2, Params, Version};
use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, ErrorKind, Field, InputKind, Meta, Output, Param, ParamKind,
    Reference, Tool, Value, Values, en,
};

// ---- bcrypt -----------------------------------------------------------------

/// bcrypt's Base64: its own alphabet, no padding (OpenBSD's `bcrypt.c`).
fn bcrypt_base64() -> data_encoding::Encoding {
    bcrypt_base64_checked(false)
}

/// bcrypt's Base64; with `strict`, the unused bits of the last character
/// must be zero, as bcrypt writes them.
fn bcrypt_base64_checked(strict: bool) -> data_encoding::Encoding {
    let mut spec = data_encoding::Specification::new();
    spec.symbols
        .push_str("./ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789");
    // A salt's last character holds 4 unused bits; OpenBSD ignores them
    // in a salt given to hash with.
    spec.check_trailing_bits = strict;
    // 64 distinct symbols, tested: it cannot fail.
    spec.encoding()
        .expect("bcrypt's alphabet is 64 distinct symbols")
}

/// The most the app hashes at: cost 16 takes some seconds; each step up
/// doubles it, and a run cannot be cancelled.
const BCRYPT_APP_COST: i64 = 16;

/// bcrypt reads at most 72 bytes of a password.
const BCRYPT_MAX: usize = 72;

/// The password as bcrypt takes it: one line, as `htpasswd -i` reads it.
fn bcrypt_password(input: &[u8]) -> Result<&[u8], Error> {
    if input.contains(&0) {
        // C's bcrypt stops at a NUL; the hash would not be of the password.
        return Err(Error::input("a bcrypt password cannot hold a NUL byte"));
    }
    Ok(input)
}

static BCRYPT_REFERENCE: Option<Reference> = Some(Reference {
    name: "htpasswd -B",
    version: "2.4",
    compat: Compat::Output,
});

const fn bcrypt_meta(
    action: &'static str,
    id: &'static str,
    title: &'static str,
    summary: &'static str,
) -> Meta {
    Meta {
        id,
        group: "bcrypt",
        action,
        action_flag: None,
        title,
        summary,
        category: Category::Crypto,
        keywords: &[
            "bcrypt", "password", "hash", "htpasswd", "blowfish", "2y", "2b",
        ],
        input: InputKind::Text { trim_newline: true },
        streams: false,
        reference: BCRYPT_REFERENCE,
    }
}

static BCRYPT_HASH_META: Meta = bcrypt_meta(
    "hash",
    "bcrypt.hash",
    en!("bcrypt hash"),
    en!("Hash a password with bcrypt, at a cost, as htpasswd -B does"),
);
static BCRYPT_VERIFY_META: Meta = bcrypt_meta(
    "verify",
    "bcrypt.verify",
    en!("bcrypt verify"),
    en!("Check a password against a bcrypt hash ($2y$, $2b$, $2a$)"),
);

static BCRYPT_HASH_PARAMS: &[Param] = &[
    Param {
        key: "cost",
        label: en!("Cost"),
        help: en!("Rounds as a power of 2, 4 to 31; each one more doubles the time"),
        kind: ParamKind::Int { min: 4, max: 31 },
        default: Value::Int(12),
        cli: Cli {
            long: "cost",
            short: Some('C'),
        },
    },
    Param {
        key: "prefix",
        label: en!("Prefix"),
        help: en!("The hash's prefix: 2b (OpenBSD), 2y (PHP, htpasswd) or 2a; the same hash"),
        kind: ParamKind::Choice {
            options: &[
                ("2b", en!("$2b$, as OpenBSD writes it")),
                ("2y", en!("$2y$, as PHP and htpasswd write it")),
                ("2a", en!("$2a$, for older readers")),
            ],
            flags: false,
            unset: None,
        },
        default: Value::Text(String::new()),
        cli: Cli {
            long: "prefix",
            short: None,
        },
    },
    Param {
        key: "salt",
        label: en!("Salt"),
        help: en!("22 characters of bcrypt's Base64, for a hash made again; random when left out"),
        kind: ParamKind::Text,
        default: Value::Text(String::new()),
        cli: Cli {
            long: "salt",
            short: None,
        },
    },
];

static BCRYPT_VERIFY_PARAMS: &[Param] = &[Param {
    key: "hash",
    label: en!("Hash"),
    help: en!("The bcrypt hash to check the password against"),
    kind: ParamKind::Text,
    default: Value::Text(String::new()),
    cli: Cli {
        long: "hash",
        short: None,
    },
}];

pub struct BcryptHash;
pub struct BcryptVerify;

/// A bcrypt hash's parts: `$2b$12$` and 53 characters of salt and hash.
pub struct BcryptParts {
    pub prefix: String,
    pub cost: u32,
    pub salt: String,
}

/// Reads a bcrypt hash, refusing what bcrypt would not have written.
pub fn bcrypt_parts(hash: &str) -> Result<BcryptParts, Error> {
    let bad = |why: &str| Error::input(format!("not a bcrypt hash: {why}"));
    let b = hash.as_bytes();
    if !hash.is_ascii() || b.len() != 60 {
        return Err(bad(&format!(
            "it has {} characters, where bcrypt's have 60",
            hash.chars().count()
        )));
    }
    if b[0] != b'$' || b[3] != b'$' || b[6] != b'$' {
        return Err(bad("it reads $2b$, a cost, $, then the salt and hash"));
    }
    let prefix = &hash[1..3];
    match prefix {
        "2a" | "2b" | "2y" => {}
        // crypt_blowfish's bug, kept for old hashes only (its 2011 advisory).
        "2x" => {
            return Err(Error::input(
                "a $2x$ hash was made by crypt_blowfish's bug with 8-bit characters; it is not checked",
            ));
        }
        _ => return Err(bad(&format!("${prefix}$ is not a bcrypt prefix"))),
    }
    // Two digits, as crypt_blowfish reads them: "+4" is not a cost.
    let cost: u32 = Some(&hash[4..6])
        .filter(|c| c.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|c| c.parse().ok())
        .filter(|c| (4..=31).contains(c))
        .ok_or_else(|| bad("its cost is not 04 to 31"))?;
    let rest = &hash[7..];
    let alphabet = bcrypt_base64();
    let (salt, sum) = rest.as_bytes().split_at(22);
    if alphabet.decode(salt).is_err() || alphabet.decode(sum).is_err() {
        return Err(bad("its salt and hash are not bcrypt's Base64"));
    }
    // A hash bcrypt wrote: the unused bits of the salt's and the hash's
    // last characters are zero. crypt_blowfish writes them so and compares
    // the strings, so another can never match (found by fuzzing).
    let strict = bcrypt_base64_checked(true);
    if strict.decode(salt).is_err() || strict.decode(sum).is_err() {
        return Err(bad(
            "its salt or hash ends in a character bcrypt never writes there",
        ));
    }
    Ok(BcryptParts {
        prefix: prefix.to_owned(),
        cost,
        salt: rest[..22].to_owned(),
    })
}

fn bcrypt_salt(salt: &str) -> Result<[u8; 16], Error> {
    let bad = || Error::usage("a bcrypt salt is 22 characters of ./A-Za-z0-9");
    if salt.len() != 22 {
        return Err(bad());
    }
    let bytes = bcrypt_base64().decode(salt.as_bytes()).map_err(|_| bad())?;
    <[u8; 16]>::try_from(bytes.as_slice()).map_err(|_| bad())
}

impl Tool for BcryptHash {
    fn meta(&self) -> &'static Meta {
        &BCRYPT_HASH_META
    }

    fn params(&self) -> &'static [Param] {
        BCRYPT_HASH_PARAMS
    }

    fn run(&self, ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let p = BCRYPT_HASH_PARAMS;
        let password = bcrypt_password(input)?;
        if password.len() > BCRYPT_MAX {
            return Err(Error::input(format!(
                "bcrypt uses only a password's first {BCRYPT_MAX} bytes, and this one has {}: \
                 a longer one would match it",
                password.len()
            )));
        }
        let cost = values.int(p, "cost");
        if !(4..=31).contains(&cost) {
            return Err(Error::usage("the cost is 4 to 31"));
        }
        if ctx.is_interactive() && cost > BCRYPT_APP_COST {
            return Err(Error::usage(format!(
                "the app hashes at cost {BCRYPT_APP_COST} at most; tw bcrypt -C {cost} takes longer"
            )));
        }
        let salt = match values.text(p, "salt") {
            s if s.is_empty() => {
                let mut salt = [0; 16];
                getrandom::fill(&mut salt)
                    .map_err(|e| Error::new(ErrorKind::Io, format!("no random salt: {e}")))?;
                salt
            }
            s => bcrypt_salt(&s)?,
        };
        ctx.check()?;
        let parts = bcrypt::hash_with_salt(password, cost as u32, salt)
            .map_err(|e| Error::input(e.to_string()))?;
        let version = match values.text(p, "prefix").as_str() {
            "2y" => bcrypt::Version::TwoY,
            "2a" => bcrypt::Version::TwoA,
            _ => bcrypt::Version::TwoB,
        };
        Ok(Output::Text(parts.format_for_version(version)))
    }

    fn costly(&self) -> bool {
        true
    }

    fn secret_input(&self) -> bool {
        true
    }
}

impl Tool for BcryptVerify {
    fn meta(&self) -> &'static Meta {
        &BCRYPT_VERIFY_META
    }

    fn params(&self) -> &'static [Param] {
        BCRYPT_VERIFY_PARAMS
    }

    fn run(&self, ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let hash = values.text(BCRYPT_VERIFY_PARAMS, "hash");
        let hash = hash.trim();
        if hash.is_empty() {
            return Err(Error::usage("give the hash to check: --hash '$2y$…'"));
        }
        let parts = bcrypt_parts(hash)?;
        if ctx.is_interactive() && i64::from(parts.cost) > BCRYPT_APP_COST {
            return Err(Error::usage(format!(
                "the app checks at cost {BCRYPT_APP_COST} at most; this hash's is {}",
                parts.cost
            )));
        }
        let password = bcrypt_password(input)?;
        ctx.check()?;
        // Past 72 bytes, as the hash's maker did: every bcrypt truncates.
        let matches = bcrypt::verify(password, hash).map_err(|e| Error::input(e.to_string()))?;
        if !matches {
            return Err(Error::new(
                ErrorKind::Check,
                "the password does not match the hash",
            ));
        }
        Ok(Output::Fields(vec![
            Field::new("matches", "Matches", "yes"),
            Field::new("prefix", "Prefix", format!("${}$", parts.prefix)),
            Field::new("cost", "Cost", parts.cost.to_string()),
            Field::new("salt", "Salt", parts.salt),
        ]))
    }

    fn costly(&self) -> bool {
        true
    }

    fn secret_input(&self) -> bool {
        true
    }
}

// ---- Argon2 -----------------------------------------------------------------

/// What an Argon2 hash is made with: the `argon2` command's options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Argon2Spec {
    pub algorithm: Algorithm,
    /// 0x10 or 0x13.
    pub version: u32,
    /// Iterations, `-t`.
    pub t_cost: u32,
    /// Memory in KiB, `-k` (or 2^`-m`).
    pub m_cost: u32,
    /// Lanes and threads, `-p`.
    pub lanes: u32,
    /// Bytes of hash, `-l`.
    pub out_len: u32,
}

impl Default for Argon2Spec {
    /// The `argon2` command's defaults.
    fn default() -> Self {
        Argon2Spec {
            algorithm: Algorithm::Argon2i,
            version: 0x13,
            t_cost: 3,
            m_cost: 4096,
            lanes: 1,
            out_len: 32,
        }
    }
}

/// The type's name: `argon2id`, as encoded hashes spell it.
pub fn argon2_name(a: Algorithm) -> &'static str {
    match a {
        Algorithm::Argon2d => "argon2d",
        Algorithm::Argon2i => "argon2i",
        _ => "argon2id",
    }
}

/// The type's name as the `argon2` command prints it: `Argon2id`.
pub fn argon2_title(a: Algorithm) -> &'static str {
    match a {
        Algorithm::Argon2d => "Argon2d",
        Algorithm::Argon2i => "Argon2i",
        _ => "Argon2id",
    }
}

/// The reference's own words for a failure (`argon2_error_message`), checked
/// in its order (`validate_inputs`): the `argon2` command prints them.
fn argon2_check(spec: &Argon2Spec, pwd: &[u8], salt: &[u8]) -> Result<(), &'static str> {
    const MAX: u64 = 0xFFFF_FFFF;
    if spec.out_len < 4 {
        return Err("Output is too short");
    }
    if pwd.len() as u64 > MAX {
        return Err("Password is too long");
    }
    if salt.len() < 8 {
        return Err("Salt is too short");
    }
    if salt.len() as u64 > MAX {
        return Err("Salt is too long");
    }
    if spec.m_cost < 8 || u64::from(spec.m_cost) < 8 * u64::from(spec.lanes) {
        return Err("Memory cost is too small");
    }
    if spec.t_cost < 1 {
        return Err("Time cost is too small");
    }
    if spec.lanes < 1 {
        return Err("Too few lanes");
    }
    if spec.lanes > 0xFF_FFFF {
        return Err("Too many lanes");
    }
    if spec.version != 0x10 && spec.version != 0x13 {
        return Err("Invalid version");
    }
    Ok(())
}

/// The most the app runs: 1 GiB, 64 passes, 1 KiB of hash.
const ARGON2_APP_MEMORY: u32 = 1 << 20;
const ARGON2_APP_PASSES: u32 = 64;
const ARGON2_APP_LEN: u32 = 1024;

fn argon2_app_limits(ctx: &Ctx, spec: &Argon2Spec) -> Result<(), Error> {
    if !ctx.is_interactive() {
        return Ok(());
    }
    let over = |what: String| {
        Err(Error::usage(format!(
            "the app runs Argon2 with {what} at most; tw argon2 runs the rest"
        )))
    };
    if spec.m_cost > ARGON2_APP_MEMORY {
        return over(format!("{ARGON2_APP_MEMORY} KiB (1 GiB) of memory"));
    }
    if spec.t_cost > ARGON2_APP_PASSES {
        return over(format!("{ARGON2_APP_PASSES} iterations"));
    }
    if spec.out_len > ARGON2_APP_LEN {
        return over(format!("{ARGON2_APP_LEN} bytes of hash"));
    }
    Ok(())
}

/// The raw hash and its encoded form, `$argon2id$v=19$m=…,t=…,p=…$salt$hash`,
/// as the reference C writes them. An error is the reference's message.
pub fn argon2_hash(
    spec: &Argon2Spec,
    pwd: &[u8],
    salt: &[u8],
) -> Result<(Vec<u8>, String), &'static str> {
    argon2_check(spec, pwd, salt)?;
    let raw = argon2_raw(spec, pwd, salt)?;
    let b64 = data_encoding::BASE64_NOPAD;
    let encoded = format!(
        "${}$v={}$m={},t={},p={}${}${}",
        argon2_name(spec.algorithm),
        spec.version,
        spec.m_cost,
        spec.t_cost,
        spec.lanes,
        b64.encode(salt),
        b64.encode(&raw)
    );
    Ok((raw, encoded))
}

fn argon2_raw(spec: &Argon2Spec, pwd: &[u8], salt: &[u8]) -> Result<Vec<u8>, &'static str> {
    let version = if spec.version == 0x10 {
        Version::V0x10
    } else {
        Version::V0x13
    };
    let len = usize::try_from(spec.out_len).map_err(|_| "Output is too long")?;
    let params = Params::new(spec.m_cost, spec.t_cost, spec.lanes, Some(len))
        .map_err(|_| "Memory cost is too small")?;
    let mut out = Vec::new();
    out.try_reserve_exact(len)
        .map_err(|_| "Memory allocation error")?;
    out.resize(len, 0);
    Argon2::new(spec.algorithm, version, params)
        .hash_password_into(pwd, salt, &mut out)
        .map_err(|e| match e {
            argon2::Error::OutOfMemory => "Memory allocation error",
            argon2::Error::SaltTooShort => "Salt is too short",
            argon2::Error::OutputTooShort => "Output is too short",
            argon2::Error::OutputTooLong => "Output is too long",
            _ => "Memory cost is too small",
        })?;
    Ok(out)
}

/// An encoded hash's parts, read as the reference's `decode_string` reads
/// them: `$v=` may be left out (version 16, then), Base64 without padding
/// and with no stray bits.
pub fn argon2_decode(encoded: &str) -> Result<(Argon2Spec, Vec<u8>, Vec<u8>), &'static str> {
    const FAIL: &str = "Decoding failed";
    let rest = encoded.strip_prefix('$').ok_or(FAIL)?;
    let (name, mut rest) = rest.split_once('$').ok_or(FAIL)?;
    let algorithm = match name {
        "argon2d" => Algorithm::Argon2d,
        "argon2i" => Algorithm::Argon2i,
        "argon2id" => Algorithm::Argon2id,
        _ => return Err("Incorrect parameter"),
    };
    fn number<'a>(s: &'a str, key: &str) -> Option<(u32, &'a str)> {
        let s = s.strip_prefix(key)?;
        let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
        let n = s[..end].parse().ok()?;
        Some((n, &s[end..]))
    }
    let mut version = 0x10;
    if let Some((v, r)) = number(rest, "v=") {
        version = v;
        rest = r.strip_prefix('$').ok_or(FAIL)?;
    }
    let (m_cost, r) = number(rest, "m=").ok_or(FAIL)?;
    let (t_cost, r) = number(r, ",t=").ok_or(FAIL)?;
    let (lanes, r) = number(r, ",p=").ok_or(FAIL)?;
    let r = r.strip_prefix('$').ok_or(FAIL)?;
    let (salt, hash) = r.split_once('$').ok_or(FAIL)?;
    let b64 = data_encoding::BASE64_NOPAD;
    let salt = b64.decode(salt.as_bytes()).map_err(|_| FAIL)?;
    let hash = b64.decode(hash.as_bytes()).map_err(|_| FAIL)?;
    let out_len = u32::try_from(hash.len()).map_err(|_| FAIL)?;
    let spec = Argon2Spec {
        algorithm,
        version,
        t_cost,
        m_cost,
        lanes,
        out_len,
    };
    // The reference checks what it decoded before it hashes.
    argon2_check(&spec, b"", &salt)?;
    Ok((spec, salt, hash))
}

/// Whether `pwd` is the password of `encoded`; an error is the reference's
/// message for an encoding it cannot read.
pub fn argon2_verify(encoded: &str, pwd: &[u8]) -> Result<bool, &'static str> {
    let (spec, salt, want) = argon2_decode(encoded)?;
    let got = argon2_raw(&spec, pwd, &salt)?;
    // In constant time, as the reference compares.
    let diff = got.iter().zip(&want).fold(0u8, |d, (a, b)| d | (a ^ b));
    Ok(diff == 0 && got.len() == want.len())
}

static ARGON2_REFERENCE: Option<Reference> = Some(Reference {
    name: "argon2",
    version: "20190702",
    compat: Compat::DropIn,
});

const fn argon2_meta(
    action: &'static str,
    id: &'static str,
    title: &'static str,
    summary: &'static str,
) -> Meta {
    Meta {
        id,
        group: "argon2",
        action,
        action_flag: None,
        title,
        summary,
        category: Category::Crypto,
        keywords: &["argon2", "argon2id", "password", "hash", "kdf", "phc"],
        input: InputKind::Bytes,
        streams: false,
        reference: ARGON2_REFERENCE,
    }
}

static ARGON2_HASH_META: Meta = argon2_meta(
    "hash",
    "argon2.hash",
    en!("Argon2 hash"),
    en!("Hash a password with Argon2id, Argon2i or Argon2d, as the argon2 command does"),
);
static ARGON2_VERIFY_META: Meta = argon2_meta(
    "verify",
    "argon2.verify",
    en!("Argon2 verify"),
    en!("Check a password against an encoded Argon2 hash ($argon2id$…)"),
);

static ARGON2_HASH_PARAMS: &[Param] = &[
    Param {
        key: "salt",
        label: en!("Salt"),
        help: en!("The salt, 8 characters or more"),
        kind: ParamKind::Text,
        default: Value::Text(String::new()),
        cli: Cli {
            long: "salt",
            short: None,
        },
    },
    Param {
        key: "type",
        label: en!("Type"),
        help: en!("Argon2id, the one RFC 9106 recommends; Argon2i, the command's default; Argon2d"),
        kind: ParamKind::Choice {
            options: &[
                ("id", en!("Argon2id")),
                ("i", en!("Argon2i")),
                ("d", en!("Argon2d")),
            ],
            flags: false,
            unset: None,
        },
        default: Value::Text(String::new()),
        cli: Cli {
            long: "type",
            short: None,
        },
    },
    Param {
        key: "iterations",
        label: en!("Iterations"),
        help: en!("Passes over the memory, -t"),
        kind: ParamKind::Int {
            min: 1,
            max: 0xFFFF_FFFF,
        },
        default: Value::Int(3),
        cli: Cli {
            long: "iterations",
            short: Some('t'),
        },
    },
    Param {
        key: "memory",
        label: en!("Memory (KiB)"),
        help: en!("Memory in KiB, -k: at least 8 for each thread"),
        kind: ParamKind::Int {
            min: 8,
            max: 0xFFFF_FFFF,
        },
        default: Value::Int(4096),
        cli: Cli {
            long: "memory",
            short: Some('k'),
        },
    },
    Param {
        key: "parallelism",
        label: en!("Parallelism"),
        help: en!("Lanes, -p; the hash depends on it"),
        kind: ParamKind::Int {
            min: 1,
            max: 0xFF_FFFF,
        },
        default: Value::Int(1),
        cli: Cli {
            long: "parallelism",
            short: Some('p'),
        },
    },
    Param {
        key: "length",
        label: en!("Length"),
        help: en!("Bytes of hash, -l"),
        kind: ParamKind::Int {
            min: 4,
            max: 0xFFFF_FFFF,
        },
        default: Value::Int(32),
        cli: Cli {
            long: "length",
            short: Some('l'),
        },
    },
    Param {
        key: "version",
        label: en!("Version"),
        help: en!("Argon2's version: 19 (1.3), or 16 (1.0) for old hashes"),
        kind: ParamKind::Choice {
            options: &[("13", en!("1.3 (19)")), ("10", en!("1.0 (16)"))],
            flags: false,
            unset: None,
        },
        default: Value::Text(String::new()),
        cli: Cli {
            long: "version",
            short: Some('v'),
        },
    },
];

static ARGON2_VERIFY_PARAMS: &[Param] = &[Param {
    key: "encoded",
    label: en!("Encoded hash"),
    help: en!("The hash to check the password against: $argon2id$v=19$…"),
    kind: ParamKind::Text,
    default: Value::Text(String::new()),
    cli: Cli {
        long: "encoded",
        short: None,
    },
}];

pub struct Argon2Hash;
pub struct Argon2Verify;

/// The spec the app's params give; types and versions as their names.
pub fn argon2_spec(values: &Values) -> Result<Argon2Spec, Error> {
    let p = ARGON2_HASH_PARAMS;
    let int = |key: &str| {
        u32::try_from(values.int(p, key))
            .map_err(|_| Error::usage(format!("{key} is a whole number from 1 to 4294967295")))
    };
    Ok(Argon2Spec {
        algorithm: match values.text(p, "type").as_str() {
            "i" => Algorithm::Argon2i,
            "d" => Algorithm::Argon2d,
            _ => Algorithm::Argon2id,
        },
        version: if values.text(p, "version") == "10" {
            0x10
        } else {
            0x13
        },
        t_cost: int("iterations")?,
        m_cost: int("memory")?,
        lanes: int("parallelism")?,
        out_len: int("length")?,
    })
}

impl Tool for Argon2Hash {
    fn meta(&self) -> &'static Meta {
        &ARGON2_HASH_META
    }

    fn params(&self) -> &'static [Param] {
        ARGON2_HASH_PARAMS
    }

    /// As `argon2` prints it, but for the time it took and its check.
    fn run(&self, ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let spec = argon2_spec(values)?;
        let salt = values.text(ARGON2_HASH_PARAMS, "salt");
        if salt.is_empty() {
            return Err(Error::usage("give a salt of 8 characters or more"));
        }
        argon2_app_limits(ctx, &spec)?;
        ctx.check()?;
        let (raw, encoded) = argon2_hash(&spec, input, salt.as_bytes()).map_err(Error::input)?;
        Ok(Output::Fields(vec![
            Field::new("type", "Type", argon2_title(spec.algorithm)),
            Field::new("iterations", "Iterations", spec.t_cost.to_string()),
            Field::new("memory", "Memory", format!("{} KiB", spec.m_cost)),
            Field::new("parallelism", "Parallelism", spec.lanes.to_string()),
            Field::new("hash", "Hash", data_encoding::HEXLOWER.encode(&raw)),
            Field::new("encoded", "Encoded", encoded),
        ]))
    }

    /// `argon2 SALT -id -t 3 …`: the salt first, as the command takes it.
    fn command_args(&self, values: &Values) -> Option<Vec<String>> {
        let spec = argon2_spec(values).ok()?;
        let d = Argon2Spec::default();
        let mut args = vec!["argon2".to_owned(), values.text(ARGON2_HASH_PARAMS, "salt")];
        match spec.algorithm {
            Algorithm::Argon2i => {}
            Algorithm::Argon2d => args.push("-d".to_owned()),
            _ => args.push("-id".to_owned()),
        }
        let mut num = |flag: &str, n: u32, default: u32| {
            if n != default {
                args.extend([flag.to_owned(), n.to_string()]);
            }
        };
        num("-t", spec.t_cost, d.t_cost);
        num("-k", spec.m_cost, d.m_cost);
        num("-p", spec.lanes, d.lanes);
        num("-l", spec.out_len, d.out_len);
        if spec.version == 0x10 {
            args.extend(["-v".to_owned(), "10".to_owned()]);
        }
        Some(args)
    }

    fn costly(&self) -> bool {
        true
    }

    fn secret_input(&self) -> bool {
        true
    }
}

impl Tool for Argon2Verify {
    fn meta(&self) -> &'static Meta {
        &ARGON2_VERIFY_META
    }

    fn params(&self) -> &'static [Param] {
        ARGON2_VERIFY_PARAMS
    }

    fn run(&self, ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let encoded = values.text(ARGON2_VERIFY_PARAMS, "encoded");
        let encoded = encoded.trim();
        if encoded.is_empty() {
            return Err(Error::usage(
                "give the encoded hash to check: $argon2id$v=19$…",
            ));
        }
        let (spec, _, _) = argon2_decode(encoded)
            .map_err(|e| Error::input(format!("not an encoded Argon2 hash: {e}")))?;
        argon2_app_limits(ctx, &spec)?;
        ctx.check()?;
        if !argon2_verify(encoded, input).map_err(Error::input)? {
            return Err(Error::new(
                ErrorKind::Check,
                "The password does not match the supplied hash",
            ));
        }
        Ok(Output::Fields(vec![
            Field::new("matches", "Matches", "yes"),
            Field::new("type", "Type", argon2_title(spec.algorithm)),
            Field::new("version", "Version", spec.version.to_string()),
            Field::new("iterations", "Iterations", spec.t_cost.to_string()),
            Field::new("memory", "Memory", format!("{} KiB", spec.m_cost)),
            Field::new("parallelism", "Parallelism", spec.lanes.to_string()),
        ]))
    }

    /// `argon2 verify ENCODED`.
    fn command_args(&self, values: &Values) -> Option<Vec<String>> {
        Some(vec![
            "argon2".to_owned(),
            "verify".to_owned(),
            values.text(ARGON2_VERIFY_PARAMS, "encoded"),
        ])
    }

    fn costly(&self) -> bool {
        true
    }

    fn secret_input(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(tool: &dyn Tool, input: &[u8], set: &[(&str, Value)]) -> Result<Output, Error> {
        let mut v = Values::new();
        for (k, val) in set {
            v.set(k, val.clone());
        }
        tool.run(&Ctx::new(), input, &v)
    }

    fn text(o: Output) -> String {
        match o {
            Output::Text(s) => s,
            other => panic!("not text: {other:?}"),
        }
    }

    fn field(o: &Output, key: &str) -> String {
        match o {
            Output::Fields(f) => f
                .iter()
                .find(|f| f.key == key)
                .map(|f| f.value.clone())
                .unwrap_or_default(),
            other => panic!("not fields: {other:?}"),
        }
    }

    /// crypt_blowfish's own test vectors (Openwall, wrapper.c), which
    /// OpenBSD's bcrypt and PHP's crypt() give too.
    #[test]
    fn bcrypt_hashes_as_crypt_blowfish_does() {
        for (password, hash) in [
            (
                "U*U",
                "$2a$05$CCCCCCCCCCCCCCCCCCCCC.E5YPO9kmyuRGyh0XouQYb4YMJKvyOeW",
            ),
            (
                "U*U*",
                "$2a$05$CCCCCCCCCCCCCCCCCCCCC.VGOzA784oUp/Z0DY336zx7pLYAy0lwK",
            ),
            (
                "U*U*U",
                "$2a$05$XXXXXXXXXXXXXXXXXXXXXOAcXxm9kjPGEMsLznoKqmqw7tc8WCx4a",
            ),
            (
                "",
                "$2a$05$CCCCCCCCCCCCCCCCCCCCC.7uG0VCzI2bS7j6ymqJi9CdcdxiRTWNy",
            ),
            (
                "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
                "$2a$05$abcdefghijklmnopqrstuu5s2v8.iXieOjg/.AySBTTZIIVFJeBui",
            ),
        ] {
            let salt = &hash[7..29];
            let got = run(
                &BcryptHash,
                password.as_bytes(),
                &[
                    ("cost", Value::Int(5)),
                    ("prefix", Value::Text("2a".into())),
                    ("salt", Value::Text(salt.into())),
                ],
            );
            assert_eq!(got.map(text), Ok(hash.to_owned()), "{password:?}");
            let checked = run(
                &BcryptVerify,
                password.as_bytes(),
                &[("hash", Value::Text(hash.into()))],
            );
            assert!(checked.is_ok(), "{password:?}: {checked:?}");
        }
    }

    /// Hashes `htpasswd -nbB -C 5 user secret` made (Apache 2.4.69, Debian 13).
    #[test]
    fn bcrypt_checks_htpasswd_hashes() {
        for hash in [
            "$2y$05$CHEzABv8He3HA.aYEGJj/eZYsINOZh41wgAP3kRVOa1znFxhNCtWC",
            "$2y$05$dgYLUe7eP4eCP5e2/oypMuueu24xYaM3ZVWduHFtfdbCzVl1pji4a",
        ] {
            let ok = run(
                &BcryptVerify,
                b"secret",
                &[("hash", Value::Text(hash.into()))],
            )
            .unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(field(&ok, "prefix"), "$2y$");
            assert_eq!(field(&ok, "cost"), "5");
            let wrong = run(
                &BcryptVerify,
                b"Secret",
                &[("hash", Value::Text(hash.into()))],
            );
            assert_eq!(wrong.map_err(|e| e.kind), Err(ErrorKind::Check));
        }
    }

    #[test]
    fn bcrypt_refuses_what_it_would_get_wrong() {
        let long = [b'a'; 73];
        let e = run(&BcryptHash, &long, &[("cost", Value::Int(4))]).map_err(|e| e.kind);
        assert_eq!(e, Err(ErrorKind::Input));
        let e = run(&BcryptHash, b"a\0b", &[("cost", Value::Int(4))]).map_err(|e| e.kind);
        assert_eq!(e, Err(ErrorKind::Input));
        let e =
            run(&BcryptHash, b"a", &[("salt", Value::Text("short".into()))]).map_err(|e| e.kind);
        assert_eq!(e, Err(ErrorKind::Usage));
        for bad in [
            "$2x$05$CCCCCCCCCCCCCCCCCCCCC.E5YPO9kmyuRGyh0XouQYb4YMJKvyOeW",
            "$2a$03$CCCCCCCCCCCCCCCCCCCCC.E5YPO9kmyuRGyh0XouQYb4YMJKvyOeW",
            "$2a$05$CCCC",
            // A sign is not a digit; unused bits are zero (found by fuzzing).
            "$2b$+4$00uuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuUuuuuuuuuuuuuuuuuuuuuZ",
            "$2b$04$0000000000000p0000000000142021525300000142021525sssgg",
            "$argon2id$v=19$m=4096,t=3,p=1$c29tZXNhbHQ$qLml5cbqFAO6YxVHhrSBHP0UWdxrIxk",
        ] {
            let e = run(&BcryptVerify, b"U*U", &[("hash", Value::Text(bad.into()))]);
            assert_eq!(e.map_err(|e| e.kind), Err(ErrorKind::Input), "{bad}");
        }
        // The app holds the cost to what it can wait for.
        let mut v = Values::new();
        v.set("cost", Value::Int(17));
        let e = BcryptHash
            .run(&Ctx::interactive(), b"a", &v)
            .map_err(|e| e.kind);
        assert_eq!(e, Err(ErrorKind::Usage));
    }

    #[test]
    fn bcrypt_salts_are_random_when_not_given() {
        let a = run(&BcryptHash, b"a", &[("cost", Value::Int(4))]).map(text);
        let b = run(&BcryptHash, b"a", &[("cost", Value::Int(4))]).map(text);
        assert_ne!(a, b);
        let a = a.unwrap_or_default();
        assert!(a.starts_with("$2b$04$") && a.len() == 60, "{a}");
    }

    /// What `echo -n password | argon2 somesalt …` printed (Debian 13's
    /// argon2 0~20190702+dfsg-4+b2).
    #[test]
    fn argon2_hashes_as_the_reference_does() {
        let spec = Argon2Spec {
            t_cost: 2,
            m_cost: 1 << 16,
            lanes: 4,
            out_len: 24,
            ..Argon2Spec::default()
        };
        let (raw, encoded) = argon2_hash(&spec, b"password", b"somesalt").unwrap_or_default();
        assert_eq!(
            data_encoding::HEXLOWER.encode(&raw),
            "45d7ac72e76f242b20b77b9bf9bf9d5915894e669a24e6c6"
        );
        assert_eq!(
            encoded,
            "$argon2i$v=19$m=65536,t=2,p=4$c29tZXNhbHQ$RdescudvJCsgt3ub+b+dWRWJTmaaJObG"
        );
        let id = Argon2Spec {
            algorithm: Algorithm::Argon2id,
            ..Argon2Spec::default()
        };
        assert_eq!(
            argon2_hash(&id, b"password", b"somesalt").map(|h| h.1),
            Ok("$argon2id$v=19$m=4096,t=3,p=1$c29tZXNhbHQ$qLml5cbqFAO6YxVHhrSBHP0UWdxrIxkNcM8aMX3blzU".into())
        );
        let d10 = Argon2Spec {
            algorithm: Algorithm::Argon2d,
            version: 0x10,
            ..Argon2Spec::default()
        };
        assert_eq!(
            argon2_hash(&d10, b"password", b"somesalt")
                .map(|h| data_encoding::HEXLOWER.encode(&h.0)),
            Ok("febc2b1a367536b4be4c4810c6b89c0fb079ef224c28643fba8bfde9a6c2ea4a".into())
        );
        assert_eq!(
            argon2_hash(&id, b"p", b"short").map(|_| ()),
            Err("Salt is too short")
        );
        let small = Argon2Spec {
            m_cost: 15,
            lanes: 2,
            ..Argon2Spec::default()
        };
        assert_eq!(
            argon2_hash(&small, b"p", b"somesalt").map(|_| ()),
            Err("Memory cost is too small")
        );
    }

    #[test]
    fn argon2_verifies_its_encodings() {
        let encoded =
            "$argon2id$v=19$m=4096,t=3,p=1$c29tZXNhbHQ$qLml5cbqFAO6YxVHhrSBHP0UWdxrIxkNcM8aMX3blzU";
        assert_eq!(argon2_verify(encoded, b"password"), Ok(true));
        assert_eq!(argon2_verify(encoded, b"passwore"), Ok(false));
        // Without $v=, the reference reads version 16.
        let (spec, _, _) = argon2_decode(
            "$argon2d$m=4096,t=3,p=1$c29tZXNhbHQ$/rwrGjZ1NrS+TEgQxribD7B57yJMKGQ/uov96abC6ko",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(spec.version, 0x10);
        for bad in [
            "",
            "argon2id$v=19$m=4096,t=3,p=1$c29tZXNhbHQ$qLml",
            "$argon2x$v=19$m=4096,t=3,p=1$c29tZXNhbHQ$qLml5cbqFAO6YxVHhrSBHP0UWdxrIxkNcM8aMX3blzU",
            "$argon2id$v=19$m=4096,t=3$c29tZXNhbHQ$qLml5cbqFAO6YxVHhrSBHP0UWdxrIxkNcM8aMX3blzU",
            // A stray bit in the last character of the salt.
            "$argon2id$v=19$m=4096,t=3,p=1$c29tZXNhbHR$qLml5cbqFAO6YxVHhrSBHP0UWdxrIxkNcM8aMX3blzU",
            "$argon2id$v=19$m=4096,t=3,p=1$c29tZXNhbHQ$qLml5cbqFAO6YxVHhrSBHP0UWdxrIxkNcM8aMX3blzU$",
        ] {
            assert!(argon2_decode(bad).is_err(), "{bad}");
        }
        let ok = run(
            &Argon2Verify,
            b"password",
            &[("encoded", Value::Text(encoded.into()))],
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(field(&ok, "type"), "Argon2id");
        let wrong = run(
            &Argon2Verify,
            b"x",
            &[("encoded", Value::Text(encoded.into()))],
        );
        assert_eq!(wrong.map_err(|e| e.kind), Err(ErrorKind::Check));
    }

    #[test]
    fn argon2_commands_read_as_the_reference_takes_them() {
        let mut v = Values::new();
        v.set("salt", Value::Text("somesalt".into()));
        assert_eq!(
            Argon2Hash.command_args(&v),
            Some(vec!["argon2".into(), "somesalt".into(), "-id".into()])
        );
        v.set("type", Value::Text("i".into()));
        v.set("memory", Value::Int(65536));
        v.set("version", Value::Text("10".into()));
        assert_eq!(
            Argon2Hash.command_args(&v).map(|a| a.join(" ")),
            Some("argon2 somesalt -k 65536 -v 10".into())
        );
        let mut v = Values::new();
        v.set("memory", Value::Int(1 << 21));
        v.set("salt", Value::Text("somesalt".into()));
        let e = Argon2Hash
            .run(&Ctx::interactive(), b"p", &v)
            .map_err(|e| e.kind);
        assert_eq!(e, Err(ErrorKind::Usage));
    }
}
