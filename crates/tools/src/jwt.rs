//! `jwt decode`, `jwt verify` and `jwt encode`: JSON Web Tokens (RFC 7519),
//! signed as JWS (RFC 7515).
//!
//! Verifying takes the key as it comes: an HMAC secret, a public key in PEM,
//! a JWK, or a JWKS (the key is picked by the token's `kid`). A key that does
//! not fit the token's algorithm is refused rather than tried: an HS256 token
//! "verified" with a public key is how forged tokens get through. Expiry and
//! not-before are checked here, against `--relative-to` when given, with a
//! leeway.

use std::str::FromStr as _;

use jiff::tz::TimeZone;
use jiff::{Span, Timestamp, Zoned};
use jsonwebtoken::{Algorithm, AlgorithmFamily, DecodingKey, EncodingKey, Header, Validation};
use serde_json::{Map, Value as Json};
use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, ErrorKind, Field, InputKind, Meta, Output, Param, ParamKind,
    Reference, Score, Span as Where, Tool, Value, Values, en,
};

use crate::time;

pub struct Decode;
pub struct Verify;
pub struct Encode;

const REFERENCE: Option<Reference> = Some(Reference {
    name: "RFC 7519",
    version: "2015",
    compat: Compat::Standard,
});

const fn meta(
    action: &'static str,
    id: &'static str,
    title: &'static str,
    summary: &'static str,
) -> Meta {
    Meta {
        id,
        group: "jwt",
        action,
        action_flag: None,
        title,
        summary,
        category: Category::Crypto,
        keywords: &["jwt", "jws", "token", "bearer", "oauth", "oidc", "claims"],
        input: InputKind::Text { trim_newline: true },
        streams: false,
        reference: REFERENCE,
    }
}

static DECODE_META: Meta = meta(
    "decode",
    "jwt.decode",
    en!("JWT decode"),
    en!("Read a JWT's header and claims, its times in your zone"),
);
static VERIFY_META: Meta = meta(
    "verify",
    "jwt.verify",
    en!("JWT verify"),
    en!("Check a JWT's signature with its secret or public key, and its expiry"),
);
static ENCODE_META: Meta = meta(
    "encode",
    "jwt.encode",
    en!("JWT encode"),
    en!("Sign claims into a JWT"),
);

const ALGORITHMS: &[(&str, &str)] = &[
    ("HS256", "HMAC with SHA-256"),
    ("HS384", "HMAC with SHA-384"),
    ("HS512", "HMAC with SHA-512"),
    ("RS256", "RSA PKCS#1 v1.5 with SHA-256"),
    ("RS384", "RSA PKCS#1 v1.5 with SHA-384"),
    ("RS512", "RSA PKCS#1 v1.5 with SHA-512"),
    ("PS256", "RSA-PSS with SHA-256"),
    ("PS384", "RSA-PSS with SHA-384"),
    ("PS512", "RSA-PSS with SHA-512"),
    ("ES256", "ECDSA P-256 with SHA-256"),
    ("ES384", "ECDSA P-384 with SHA-384"),
    ("EdDSA", "Ed25519"),
];

const TZ: Param = Param {
    key: "tz",
    label: en!("Time zone"),
    help: en!("The zone to show times in; this computer's when left out"),
    kind: ParamKind::Text,
    default: Value::Text(String::new()),
    cli: Cli {
        long: "tz",
        short: None,
    },
};

const RELATIVE_TO: Param = Param {
    key: "relative_to",
    label: en!("Now is"),
    help: en!(
        "The moment expiry is checked against and \"in 3 hours\" counts from; the clock when left out"
    ),
    kind: ParamKind::Text,
    default: Value::Text(String::new()),
    cli: Cli {
        long: "relative-to",
        short: None,
    },
};

const BASE64_SECRET: Param = Param {
    key: "base64_secret",
    label: en!("The secret is Base64"),
    help: en!("Decode the HMAC secret from Base64 (standard or URL-safe) first"),
    kind: ParamKind::Flag,
    default: Value::Bool(false),
    cli: Cli {
        long: "base64-secret",
        short: None,
    },
};

static DECODE_PARAMS: &[Param] = &[TZ, RELATIVE_TO];

static VERIFY_PARAMS: &[Param] = &[
    Param {
        key: "key",
        label: en!("Key"),
        help: en!("The HMAC secret, or the public key: PEM, a JWK or a JWKS"),
        kind: ParamKind::Secret,
        default: Value::Text(String::new()),
        cli: Cli {
            long: "key",
            short: Some('k'),
        },
    },
    BASE64_SECRET,
    Param {
        key: "alg",
        label: en!("Algorithm"),
        help: en!("Accept only this algorithm; the token's own when left out"),
        kind: ParamKind::Choice {
            options: ALGORITHMS,
            flags: false,
            unset: Some("The token's"),
        },
        default: Value::Text(String::new()),
        cli: Cli {
            long: "alg",
            short: None,
        },
    },
    Param {
        key: "iss",
        label: en!("Issuer"),
        help: en!("Require this issuer (iss)"),
        kind: ParamKind::Text,
        default: Value::Text(String::new()),
        cli: Cli {
            long: "iss",
            short: None,
        },
    },
    Param {
        key: "aud",
        label: en!("Audience"),
        help: en!("Require this audience (aud)"),
        kind: ParamKind::Text,
        default: Value::Text(String::new()),
        cli: Cli {
            long: "aud",
            short: None,
        },
    },
    Param {
        key: "leeway",
        label: en!("Leeway, seconds"),
        help: en!("Allow this much clock difference when checking expiry and not-before"),
        kind: ParamKind::Int {
            min: 0,
            max: 86_400,
        },
        default: Value::Int(60),
        cli: Cli {
            long: "leeway",
            short: None,
        },
    },
    Param {
        key: "ignore_time",
        label: en!("Ignore expiry"),
        help: en!("Check the signature only, not expiry (exp) or not-before (nbf)"),
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli {
            long: "ignore-time",
            short: None,
        },
    },
    TZ,
    RELATIVE_TO,
];

static ENCODE_PARAMS: &[Param] = &[
    Param {
        key: "alg",
        label: en!("Algorithm"),
        help: en!("The signing algorithm"),
        kind: ParamKind::Choice {
            options: ALGORITHMS,
            flags: false,
            unset: None,
        },
        default: Value::Text(String::new()),
        cli: Cli {
            long: "alg",
            short: None,
        },
    },
    Param {
        key: "key",
        label: en!("Key"),
        help: en!("The HMAC secret, or the private key in PEM"),
        kind: ParamKind::Secret,
        default: Value::Text(String::new()),
        cli: Cli {
            long: "key",
            short: Some('k'),
        },
    },
    BASE64_SECRET,
    Param {
        key: "kid",
        label: en!("Key ID"),
        help: en!("The kid to put in the header"),
        kind: ParamKind::Text,
        default: Value::Text(String::new()),
        cli: Cli {
            long: "kid",
            short: None,
        },
    },
    Param {
        key: "iat",
        label: en!("Add iat"),
        help: en!("Set the issued-at claim (iat) to now"),
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli {
            long: "iat",
            short: None,
        },
    },
    Param {
        key: "expires_in",
        label: en!("Expires in"),
        help: en!("Set the expiry claim (exp) this long from now, such as 1h or 30 minutes"),
        kind: ParamKind::Text,
        default: Value::Text(String::new()),
        cli: Cli {
            long: "expires-in",
            short: None,
        },
    },
    RELATIVE_TO,
];

/// A signed JWT taken apart.
#[derive(Debug)]
struct Token {
    /// The three parts, as they were.
    compact: String,
    header: Map<String, Json>,
    payload: Json,
    signature: String,
}

/// Takes `input` apart: three Base64url parts, the first two JSON. A
/// `Bearer ` in front, as copied from an `Authorization` header, is dropped.
fn split(input: &str) -> Result<Token, Error> {
    let trimmed = input.trim();
    let token = trimmed
        .strip_prefix("Bearer ")
        .or_else(|| trimmed.strip_prefix("bearer "))
        .unwrap_or(trimmed)
        .trim();
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() == 5 {
        return Err(Error::input(
            "this is an encrypted JWT (JWE, five parts): only its issuer's key reads it",
        ));
    }
    if parts.len() != 3 {
        return Err(Error::input(format!(
            "a JWT is three Base64url parts separated by dots; this has {}",
            parts.len()
        )));
    }
    let start = input.len() - input.trim_start().len() + (trimmed.len() - token.len());
    let at = |part: usize| {
        let offset = start + parts[..part].iter().map(|p| p.len() + 1).sum::<usize>();
        Where::at(input.as_bytes(), offset)
    };
    let json = |part: usize, what: &str| -> Result<Json, Error> {
        let bytes = base64url(parts[part])
            .ok_or_else(|| Error::input(format!("the {what} is not Base64url")).at(at(part)))?;
        serde_json::from_slice(&bytes)
            .map_err(|e| Error::input(format!("the {what} is not JSON: {e}")).at(at(part)))
    };
    let header = match json(0, "header")? {
        Json::Object(h) => h,
        _ => return Err(Error::input("the header is not a JSON object").at(at(0))),
    };
    let payload = json(1, "payload")?;
    Ok(Token {
        compact: token.to_owned(),
        header,
        payload,
        signature: parts[2].to_owned(),
    })
}

/// Base64url with or without padding, as tokens come; standard Base64 too,
/// for secrets.
fn base64url(text: &str) -> Option<Vec<u8>> {
    let bare = text.trim_end_matches('=');
    data_encoding::BASE64URL_NOPAD
        .decode(bare.as_bytes())
        .or_else(|_| data_encoding::BASE64_NOPAD.decode(bare.as_bytes()))
        .ok()
}

fn show(v: &Json) -> String {
    match v {
        Json::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The zone and "now" a run uses.
fn clock(values: &Values, params: &[Param]) -> Result<(TimeZone, Zoned), Error> {
    let tz = time::zone(&values.text(params, "tz"))?;
    let clock = Zoned::now().with_time_zone(tz.clone());
    let relative_to = values.text(params, "relative_to");
    if relative_to.is_empty() {
        return Ok((tz, clock));
    }
    let now = time::parse(&relative_to, &tz, &clock, "")?;
    Ok((tz, now))
}

/// A NumericDate claim as a moment.
fn moment(v: &Json, tz: &TimeZone) -> Option<Zoned> {
    let seconds = v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))?;
    Timestamp::from_second(seconds)
        .ok()
        .map(|t| t.to_zoned(tz.clone()))
}

const TIME_CLAIMS: [&str; 4] = ["exp", "nbf", "iat", "auth_time"];

impl Tool for Decode {
    fn meta(&self) -> &'static Meta {
        &DECODE_META
    }

    fn params(&self) -> &'static [Param] {
        DECODE_PARAMS
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let text = std::str::from_utf8(input).map_err(|_| Error::input("input is not UTF-8"))?;
        let token = split(text)?;
        let (tz, now) = clock(values, DECODE_PARAMS)?;
        let mut fields: Vec<Field> = token
            .header
            .iter()
            .map(|(k, v)| Field::new(format!("header.{k}"), k, show(v)).in_section("Header"))
            .collect();
        match &token.payload {
            Json::Object(claims) => {
                for (k, v) in claims {
                    let mut value = show(v);
                    if TIME_CLAIMS.contains(&k.as_str())
                        && let Some(z) = moment(v, &tz)
                    {
                        value = format!(
                            "{value} · {} · {}",
                            time::local(&z),
                            time::describe(&now, &z)
                        );
                    }
                    fields.push(Field::new(format!("payload.{k}"), k, value).in_section("Payload"));
                }
            }
            other => {
                fields.push(Field::new("payload", "payload", show(other)).in_section("Payload"))
            }
        }
        let alg = token.header.get("alg").map(show).unwrap_or_default();
        fields.push(
            Field::new(
                "signature",
                "state",
                format!("{alg}, not checked: jwt verify checks it with the key"),
            )
            .in_section("Signature"),
        );
        let json = serde_json::json!({
            "header": token.header,
            "payload": token.payload,
            "signature": token.signature,
        });
        Ok(Output::Record {
            fields,
            json: json.to_string(),
        })
    }

    fn detect(&self, sample: &[u8]) -> Option<Score> {
        let s = std::str::from_utf8(sample).ok()?;
        let looks = s.trim().split('.').count() == 3 && split(s).is_ok();
        looks.then_some(Score(95))
    }
}

fn check(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Check, message)
}

fn family_name(f: AlgorithmFamily) -> &'static str {
    match f {
        AlgorithmFamily::Hmac => "HMAC",
        AlgorithmFamily::Rsa => "RSA",
        AlgorithmFamily::Ec => "EC",
        AlgorithmFamily::Ed => "Ed25519",
    }
}

fn algorithm(name: &str) -> Result<Algorithm, Error> {
    if name.eq_ignore_ascii_case("none") {
        return Err(check(
            "the token is unsigned (alg none): there is nothing to verify, and it must not be trusted",
        ));
    }
    Algorithm::from_str(name).map_err(|_| Error::input(format!("no algorithm named {name:?}")))
}

/// The key to verify an `alg` token with, and how to name it.
fn decoding_key(
    alg: Algorithm,
    key: &str,
    base64: bool,
    kid: Option<&str>,
) -> Result<(DecodingKey, String), Error> {
    let family = alg.family();
    let trimmed = key.trim();
    let unfit = |what: &str| {
        Error::usage(format!(
            "the token is signed with {alg:?}, but the key is {what}: refused, as a key of the \
             wrong kind is how forged tokens get through"
        ))
    };
    if trimmed.starts_with("-----BEGIN") {
        if trimmed.contains("PRIVATE KEY") {
            return Err(Error::usage(
                "this is a private key: verify with the public key",
            ));
        }
        let pem = trimmed.as_bytes();
        let decoded = match family {
            AlgorithmFamily::Hmac => return Err(unfit("a public key")),
            AlgorithmFamily::Rsa => DecodingKey::from_rsa_pem(pem),
            AlgorithmFamily::Ec => DecodingKey::from_ec_pem(pem),
            AlgorithmFamily::Ed => DecodingKey::from_ed_pem(pem),
        };
        let key = decoded.map_err(|e| {
            Error::usage(format!(
                "the key is not an {} public key in PEM: {e}",
                family_name(family)
            ))
        })?;
        return Ok((key, "this public key".into()));
    }
    if trimmed.starts_with('{') {
        let json: Json = serde_json::from_str(trimmed)
            .map_err(|e| Error::usage(format!("the key is not JSON: {e}")))?;
        let jwk: jsonwebtoken::jwk::Jwk = if json.get("keys").is_some() {
            let set: jsonwebtoken::jwk::JwkSet = serde_json::from_value(json)
                .map_err(|e| Error::usage(format!("the key is not a JWKS: {e}")))?;
            let found = match kid {
                Some(kid) => set.find(kid).cloned(),
                None if set.keys.len() == 1 => set.keys.first().cloned(),
                None => {
                    return Err(Error::usage(format!(
                        "the JWKS has {} keys and the token names none (no kid)",
                        set.keys.len()
                    )));
                }
            };
            found.ok_or_else(|| check(format!("the JWKS has no key {:?}", kid.unwrap_or(""))))?
        } else {
            serde_json::from_value(json)
                .map_err(|e| Error::usage(format!("the key is not a JWK: {e}")))?
        };
        let named = jwk
            .common
            .key_id
            .clone()
            .map(|k| format!("JWK {k}"))
            .unwrap_or_else(|| "this JWK".into());
        let key = DecodingKey::from_jwk(&jwk)
            .map_err(|e| Error::usage(format!("the JWK cannot be used: {e}")))?;
        return Ok((key, named));
    }
    if family != AlgorithmFamily::Hmac {
        return Err(Error::usage(format!(
            "the token is signed with {alg:?}: give its public key (PEM, a JWK or a JWKS), not a secret"
        )));
    }
    let secret = if base64 {
        base64url(trimmed).ok_or_else(|| Error::usage("the secret is not Base64"))?
    } else {
        key.as_bytes().to_vec()
    };
    Ok((DecodingKey::from_secret(&secret), "this secret".into()))
}

impl Tool for Verify {
    fn meta(&self) -> &'static Meta {
        &VERIFY_META
    }

    fn params(&self) -> &'static [Param] {
        VERIFY_PARAMS
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let p = VERIFY_PARAMS;
        let text = std::str::from_utf8(input).map_err(|_| Error::input("input is not UTF-8"))?;
        let token = split(text)?;
        let header_alg = token.header.get("alg").map(show).unwrap_or_default();
        let wanted = values.text(p, "alg");
        if !wanted.is_empty() && wanted != header_alg {
            return Err(check(format!(
                "the token is signed with {header_alg}, not {wanted}"
            )));
        }
        let alg = algorithm(&header_alg)?;
        let key = values.text(p, "key");
        if key.trim().is_empty() {
            return Err(Error::usage(
                "give the key: the HMAC secret, or the public key as PEM, a JWK or a JWKS",
            ));
        }
        let kid = token.header.get("kid").and_then(Json::as_str);
        let (decoding, named) = decoding_key(alg, &key, values.flag(p, "base64_secret"), kid)?;
        let (iss, aud) = (values.text(p, "iss"), values.text(p, "aud"));
        let mut validation = Validation::new(alg);
        validation.required_spec_claims.clear();
        validation.validate_exp = false;
        validation.validate_nbf = false;
        validation.leeway = 0;
        validation.validate_aud = !aud.is_empty();
        if !iss.is_empty() {
            validation.set_issuer(&[iss.as_str()]);
        }
        if !aud.is_empty() {
            validation.set_audience(&[aud.as_str()]);
        }
        let claim = |name: &str| {
            token
                .payload
                .get(name)
                .map(show)
                .unwrap_or_else(|| "none".into())
        };
        jsonwebtoken::decode::<Json>(&token.compact, &decoding, &validation).map_err(|e| {
            use jsonwebtoken::errors::ErrorKind as K;
            match e.kind() {
                K::InvalidSignature => check("the signature does not match: the token was changed, or signed with another key"),
                K::InvalidIssuer => check(format!("the issuer is {}, not {iss}", claim("iss"))),
                K::InvalidAudience => check(format!("the audience is {}, not {aud}", claim("aud"))),
                K::InvalidAlgorithm => check(format!("the key does not fit {alg:?}")),
                _ => check(e.to_string()),
            }
        })?;
        let (tz, now) = clock(values, p)?;
        let leeway = jiff::SignedDuration::from_secs(values.int(p, "leeway"));
        let ignore_time = values.flag(p, "ignore_time");
        let mut fields = vec![
            Field::new("result", "Result", "Valid"),
            Field::new("signature", "Signature", format!("{alg:?}, by {named}")),
        ];
        let exp = token.payload.get("exp").and_then(|v| moment(v, &tz));
        let nbf = token.payload.get("nbf").and_then(|v| moment(v, &tz));
        if !ignore_time {
            if let Some(exp) = &exp
                && now.timestamp() > exp.timestamp() + leeway
            {
                return Err(check(format!(
                    "expired {} ({})",
                    time::describe(&now, exp),
                    time::local(exp)
                )));
            }
            if let Some(nbf) = &nbf
                && nbf.timestamp() > now.timestamp() + leeway
            {
                return Err(check(format!(
                    "not valid yet: from {} ({})",
                    time::local(nbf),
                    time::describe(&now, nbf)
                )));
            }
        }
        let state = |z: &Option<Zoned>, none: &str| match (z, ignore_time) {
            (_, true) => "not checked".to_owned(),
            (Some(z), false) => format!("{} · {}", time::describe(&now, z), time::local(z)),
            (None, false) => none.to_owned(),
        };
        fields.push(Field::new(
            "expires",
            "Expires",
            state(&exp, "never: no exp claim"),
        ));
        fields.push(Field::new(
            "not_before",
            "Not before",
            state(&nbf, "no nbf claim"),
        ));
        if !iss.is_empty() {
            fields.push(Field::new("issuer", "Issuer", iss));
        }
        if !aud.is_empty() {
            fields.push(Field::new("audience", "Audience", aud));
        }
        Ok(Output::Fields(fields))
    }
}

impl Tool for Encode {
    fn meta(&self) -> &'static Meta {
        &ENCODE_META
    }

    fn params(&self) -> &'static [Param] {
        ENCODE_PARAMS
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let p = ENCODE_PARAMS;
        let text = std::str::from_utf8(input).map_err(|_| Error::input("input is not UTF-8"))?;
        let mut claims = if text.trim().is_empty() {
            Map::new()
        } else {
            match serde_json::from_str::<Json>(text) {
                Ok(Json::Object(m)) => m,
                Ok(_) => return Err(Error::input("the claims are not a JSON object")),
                Err(e) => {
                    let offset = text
                        .lines()
                        .take(e.line().saturating_sub(1))
                        .map(|l| l.len() + 1)
                        .sum::<usize>()
                        + e.column().saturating_sub(1);
                    return Err(Error::input(format!("the claims are not JSON: {e}"))
                        .at(Where::at(text.as_bytes(), offset)));
                }
            }
        };
        let name = values.text(p, "alg");
        let alg = algorithm(if name.is_empty() { "HS256" } else { &name })?;
        let key = values.text(p, "key");
        if key.is_empty() {
            return Err(Error::usage(
                "give the key: the HMAC secret, or the private key in PEM",
            ));
        }
        let (_, now) = clock(values, p)?;
        if values.flag(p, "iat") {
            claims.insert("iat".into(), Json::from(now.timestamp().as_second()));
        }
        let expires_in = values.text(p, "expires_in");
        if !expires_in.trim().is_empty() {
            let span: Span = expires_in.trim().parse().map_err(|_| {
                Error::usage(format!(
                    "cannot read {expires_in:?} as a length of time, such as 1h or 30 minutes"
                ))
            })?;
            let exp = now
                .checked_add(span)
                .map_err(|e| Error::usage(e.to_string()))?;
            claims.insert("exp".into(), Json::from(exp.timestamp().as_second()));
        }
        let pem = key.trim().as_bytes();
        let signing = match alg.family() {
            AlgorithmFamily::Hmac => {
                let secret = if values.flag(p, "base64_secret") {
                    base64url(key.trim()).ok_or_else(|| Error::usage("the secret is not Base64"))?
                } else {
                    key.as_bytes().to_vec()
                };
                Ok(EncodingKey::from_secret(&secret))
            }
            _ if !key.trim().starts_with("-----BEGIN") => {
                return Err(Error::usage(format!(
                    "{alg:?} signs with a private key in PEM, not a secret"
                )));
            }
            AlgorithmFamily::Rsa => EncodingKey::from_rsa_pem(pem),
            AlgorithmFamily::Ec => EncodingKey::from_ec_pem(pem),
            AlgorithmFamily::Ed => EncodingKey::from_ed_pem(pem),
        }
        .map_err(|e| {
            Error::usage(format!(
                "the key is not an {} private key in PEM: {e}",
                family_name(alg.family())
            ))
        })?;
        let mut header = Header::new(alg);
        let kid = values.text(p, "kid");
        if !kid.is_empty() {
            header.kid = Some(kid);
        }
        let token = jsonwebtoken::encode(&header, &Json::Object(claims), &signing)
            .map_err(|e| Error::usage(e.to_string()))?;
        Ok(Output::Text(token))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 7515, appendix A.1: an HS256 JWS and its key.
    const RFC_TOKEN: &str = "eyJ0eXAiOiJKV1QiLA0KICJhbGciOiJIUzI1NiJ9.eyJpc3MiOiJqb2UiLA0KICJleHAiOjEzMDA4MTkzODAsDQogImh0dHA6Ly9leGFtcGxlLmNvbS9pc19yb290Ijp0cnVlfQ.dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const RFC_KEY: &str =
        "AyM1SysPpbyDfgZld3umj1qzKObwVMkoqQ-EstJQLr_T-1qS0gZH75aKtMN3Yj0iPS4hcgUuTwjAzZr1Z9CAow";
    /// Its exp, 2011-03-22T18:43:00Z, less an hour.
    const BEFORE_RFC_EXP: &str = "2011-03-22T17:43:00Z";

    fn values(set: &[(&str, Value)]) -> Values {
        let mut v = Values::new();
        v.set("tz", Value::Text("UTC".into()));
        for (k, x) in set {
            v.set(k, x.clone());
        }
        v
    }

    fn text(x: &str) -> Value {
        Value::Text(x.into())
    }

    fn verify(token: &str, set: &[(&str, Value)]) -> Result<Output, Error> {
        Verify.run(&Ctx::new(), token.as_bytes(), &values(set))
    }

    fn encode(claims: &str, set: &[(&str, Value)]) -> String {
        match Encode.run(&Ctx::new(), claims.as_bytes(), &values(set)) {
            Ok(Output::Text(t)) => t,
            other => panic!("{other:?}"),
        }
    }

    fn key(name: &str) -> String {
        let path = format!("{}/src/testdata/jwt/{name}.pem", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(path).unwrap_or_default()
    }

    #[test]
    fn decodes_the_rfc_example_with_its_times() {
        let v = values(&[("relative_to", text(BEFORE_RFC_EXP))]);
        let out = Decode.run(&Ctx::new(), format!("Bearer {RFC_TOKEN}\n").as_bytes(), &v);
        let Ok(Output::Record { fields, json }) = out else {
            panic!("{out:?}")
        };
        let get = |k: &str| fields.iter().find(|f| f.key == k).map(|f| f.value.clone());
        assert_eq!(get("header.alg").as_deref(), Some("HS256"));
        assert_eq!(get("payload.iss").as_deref(), Some("joe"));
        assert_eq!(
            get("payload.http://example.com/is_root").as_deref(),
            Some("true")
        );
        assert_eq!(
            get("payload.exp").as_deref(),
            Some("1300819380 · Tue 2011-03-22 18:43:00 +00:00 · in 1 hour")
        );
        let json: Json = serde_json::from_str(&json).unwrap_or_default();
        assert_eq!(json["payload"]["exp"], Json::from(1300819380));
    }

    #[test]
    fn verifies_the_rfc_example() {
        let set = [
            ("key", text(RFC_KEY)),
            ("base64_secret", Value::Bool(true)),
            ("relative_to", text(BEFORE_RFC_EXP)),
        ];
        assert!(matches!(verify(RFC_TOKEN, &set), Ok(Output::Fields(_))));
        // A day later it has expired, unless time is not checked.
        let later = [
            set[0].clone(),
            set[1].clone(),
            ("relative_to", text("2011-03-23T18:43:00Z")),
        ];
        let e = verify(RFC_TOKEN, &later).expect_err("expired");
        assert_eq!(e.kind, ErrorKind::Check);
        assert!(e.message.starts_with("expired 1 day ago"), "{}", e.message);
        let ignoring = [
            later[0].clone(),
            later[1].clone(),
            later[2].clone(),
            ("ignore_time", Value::Bool(true)),
        ];
        assert!(verify(RFC_TOKEN, &ignoring).is_ok());
        // A changed payload no longer matches.
        let parts: Vec<&str> = RFC_TOKEN.split('.').collect();
        let forged = format!("{}.{}.{}", parts[0], "eyJpc3MiOiJtYWxsb3J5In0", parts[2]);
        let e = verify(&forged, &set).expect_err("forged");
        assert!(
            e.message.starts_with("the signature does not match"),
            "{}",
            e.message
        );
    }

    #[test]
    fn every_family_signs_and_verifies() {
        for (alg, private, public) in [
            ("RS256", "rsa-private", "rsa-public"),
            ("PS384", "rsa-private", "rsa-public"),
            ("ES256", "ec-private", "ec-public"),
            ("EdDSA", "ed-private", "ed-public"),
        ] {
            let token = encode(
                r#"{"sub":"42"}"#,
                &[("alg", text(alg)), ("key", text(&key(private)))],
            );
            assert!(
                verify(&token, &[("key", text(&key(public)))]).is_ok(),
                "{alg}"
            );
            let wrong = if public == "ec-public" {
                "ed-public"
            } else {
                "ec-public"
            };
            assert!(
                verify(&token, &[("key", text(&key(wrong)))]).is_err(),
                "{alg} with {wrong}"
            );
        }
        let token = encode(r#"{"sub":"42"}"#, &[("key", text("s3cret"))]);
        assert!(verify(&token, &[("key", text("s3cret"))]).is_ok());
        assert!(verify(&token, &[("key", text("other"))]).is_err());
    }

    #[test]
    fn a_key_of_the_wrong_kind_is_refused() {
        // The classic forgery: HS256 "signed" with the RSA public key's PEM.
        let public = key("rsa-public");
        let forged = encode(r#"{"admin":true}"#, &[("key", text(&public))]);
        let e = verify(&forged, &[("key", text(&public))]).expect_err("refused");
        assert_eq!(e.kind, ErrorKind::Usage);
        assert!(e.message.contains("refused"), "{}", e.message);
        // An RS256 token and a secret.
        let token = encode(
            "{}",
            &[("alg", text("RS256")), ("key", text(&key("rsa-private")))],
        );
        assert!(verify(&token, &[("key", text("s3cret"))]).is_err());
        // Pinning the algorithm.
        let token = encode("{}", &[("key", text("s3cret"))]);
        let e =
            verify(&token, &[("key", text("s3cret")), ("alg", text("RS256"))]).expect_err("pinned");
        assert_eq!(e.message, "the token is signed with HS256, not RS256");
    }

    #[test]
    fn issuer_audience_expiry_and_jwks() {
        let now = "2026-11-03T08:00:00Z";
        let token = encode(
            r#"{"iss":"https://id.example.com","aud":"api"}"#,
            &[
                ("key", text("s3cret")),
                ("iat", Value::Bool(true)),
                ("expires_in", text("1h")),
                ("relative_to", text(now)),
            ],
        );
        let ok = |set: &[(&str, Value)]| {
            let mut all = vec![("key", text("s3cret")), ("relative_to", text(now))];
            all.extend_from_slice(set);
            verify(&token, &all)
        };
        assert!(
            ok(&[
                ("iss", text("https://id.example.com")),
                ("aud", text("api"))
            ])
            .is_ok()
        );
        assert_eq!(
            ok(&[("iss", text("evil"))]).map_err(|e| e.message),
            Err("the issuer is https://id.example.com, not evil".into())
        );
        assert!(ok(&[("aud", text("web"))]).is_err());
        assert!(
            ok(&[("relative_to", text("2026-11-03T09:00:30Z"))]).is_ok(),
            "within the leeway"
        );
        assert!(
            ok(&[("relative_to", text("2026-11-03T09:01:30Z"))]).is_err(),
            "past the leeway"
        );

        // A JWKS: the key found by the token's kid.
        let token = encode("{}", &[("key", text("s3cret")), ("kid", text("k2"))]);
        let k = data_encoding::BASE64URL_NOPAD.encode(b"s3cret");
        let jwks = format!(
            r#"{{"keys":[{{"kty":"oct","kid":"k1","k":"AAAA"}},{{"kty":"oct","kid":"k2","k":"{k}"}}]}}"#
        );
        assert!(verify(&token, &[("key", text(&jwks))]).is_ok());
    }

    #[test]
    fn what_is_not_a_signed_jwt() {
        assert!(split("a.b").is_err());
        let jwe = split("a.b.c.d.e")
            .err()
            .map(|e| e.message)
            .unwrap_or_default();
        assert!(jwe.contains("encrypted"), "{jwe}");
        let e = split("eyJhbGciOiJIUzI1NiJ9.###.x").expect_err("bad payload");
        assert_eq!(e.span.map(|s| s.column), Some(22));
        assert!(Decode.detect(RFC_TOKEN.as_bytes()).is_some());
        assert!(Decode.detect(b"hello.world.again").is_none());
        let unsigned = "eyJhbGciOiJub25lIn0.eyJzdWIiOiIxIn0.";
        assert!(verify(unsigned, &[("key", text("x"))]).is_err());
    }
}
