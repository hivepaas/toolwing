//! `cert`: what an X.509 certificate says (RFC 5280), from PEM or DER: who
//! it is for and from, when it is valid, its names, its key and what it may
//! be used for, and its fingerprints as `openssl x509 -fingerprint` writes
//! them. Several certificates (a chain) are each read, and said to be in
//! order or not: each issued by the next, leaf first, root last.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use sha1::Digest as _;
use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, Field, InputKind, Meta, Output, Param, ParamKind, Reference,
    Score, Tool, Value, Values, en,
};
use x509_parser::certificate::X509Certificate;
use x509_parser::extensions::{GeneralName, ParsedExtension};
use x509_parser::objects::{oid_registry, oid2sn};
use x509_parser::public_key::PublicKey;

pub struct Read;

static META: Meta = Meta {
    id: "cert",
    group: "cert",
    action: "",
    action_flag: None,
    title: en!("Certificate"),
    summary: en!("What an X.509 certificate or chain says: names, validity, key, fingerprints"),
    category: Category::Crypto,
    keywords: &[
        "cert",
        "certificate",
        "x509",
        "x.509",
        "pem",
        "der",
        "tls",
        "ssl",
        "chain",
        "san",
    ],
    input: InputKind::Bytes,
    streams: false,
    reference: Some(Reference {
        name: "openssl x509 -noout -text",
        version: "3",
        compat: Compat::Output,
    }),
};

static PARAMS: &[Param] = &[Param {
    key: "at",
    label: en!("Valid at"),
    help: en!("The time validity is judged at, in RFC 3339; now when left out"),
    kind: ParamKind::Text,
    default: Value::Text(String::new()),
    cli: Cli {
        long: "at",
        short: None,
    },
}];

/// The certificates in `input`: PEM's `CERTIFICATE` blocks, or one in DER.
fn certificates(input: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
    if input.first() == Some(&0x30) {
        return Ok(vec![input.to_vec()]);
    }
    let mut ders = Vec::new();
    for pem in x509_parser::pem::Pem::iter_from_buffer(input) {
        let pem = pem.map_err(|e| Error::input(format!("not PEM: {e}")))?;
        match pem.label.as_str() {
            "CERTIFICATE" | "TRUSTED CERTIFICATE" | "X509 CERTIFICATE" => ders.push(pem.contents),
            label if label.contains("PRIVATE KEY") => {
                return Err(Error::input(
                    "this is a private key, not a certificate (it is not shown)",
                ));
            }
            label if label.contains("CERTIFICATE REQUEST") => {
                return Err(Error::input(
                    "this is a certificate request (CSR), not a certificate",
                ));
            }
            label => {
                return Err(Error::input(format!(
                    "a {label:?} block, not a certificate"
                )));
            }
        }
    }
    if ders.is_empty() {
        return Err(Error::input(
            "no certificate: neither PEM (-----BEGIN CERTIFICATE-----) nor DER",
        ));
    }
    Ok(ders)
}

/// Bytes as OpenSSL writes a fingerprint or key id: hex in pairs, by colons.
fn colons(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn time(t: i64) -> String {
    jiff::Timestamp::from_second(t)
        .map(|t| t.to_string())
        .unwrap_or_else(|_| t.to_string())
}

/// How long `seconds` is, roughly, as people say it.
fn span(seconds: i64) -> String {
    let days = seconds / 86_400;
    match days {
        0 => "today".to_owned(),
        1 => "1 day".to_owned(),
        d if d < 60 => format!("{d} days"),
        d if d < 730 => format!("{} months", d / 30),
        d => format!("{} years", d / 365),
    }
}

fn key(cert: &X509Certificate) -> String {
    let spki = cert.public_key();
    let oid = spki.algorithm.algorithm.to_id_string();
    match oid.as_str() {
        "1.3.101.112" => return "Ed25519".to_owned(),
        "1.3.101.113" => return "Ed448".to_owned(),
        _ => {}
    }
    match spki.parsed() {
        Ok(PublicKey::RSA(r)) => format!("RSA, {} bits", r.key_size()),
        Ok(PublicKey::EC(_)) => {
            let curve = spki
                .algorithm
                .parameters
                .as_ref()
                .and_then(|p| p.as_oid().ok());
            let curve = curve.map(|c| c.to_id_string()).unwrap_or_default();
            let name = match curve.as_str() {
                "1.2.840.10045.3.1.7" => "P-256",
                "1.3.132.0.34" => "P-384",
                "1.3.132.0.35" => "P-521",
                other => other,
            };
            format!("EC, {name}")
        }
        Ok(PublicKey::DSA(_)) => "DSA".to_owned(),
        _ => oid2sn(&spki.algorithm.algorithm, oid_registry()).map_or(oid, str::to_owned),
    }
}

fn san(name: &GeneralName) -> String {
    match name {
        GeneralName::DNSName(d) => format!("DNS:{d}"),
        GeneralName::RFC822Name(e) => format!("email:{e}"),
        GeneralName::URI(u) => format!("URI:{u}"),
        GeneralName::IPAddress(b) => {
            let ip = match b.len() {
                4 => <[u8; 4]>::try_from(*b)
                    .ok()
                    .map(|a| IpAddr::V4(Ipv4Addr::from(a))),
                16 => <[u8; 16]>::try_from(*b)
                    .ok()
                    .map(|a| IpAddr::V6(Ipv6Addr::from(a))),
                _ => None,
            };
            ip.map_or_else(|| format!("IP:{}", colons(b)), |ip| format!("IP:{ip}"))
        }
        GeneralName::DirectoryName(n) => format!("DirName:{n}"),
        other => format!("{other:?}"),
    }
}

/// One certificate's fields, in `section`, and its JSON.
fn describe(
    cert: &X509Certificate,
    der: &[u8],
    at: i64,
    section: &str,
) -> (Vec<Field>, serde_json::Value) {
    let mut fields = Vec::new();
    let mut add = |key: &str, label: &'static str, value: String| {
        fields.push(Field::new(key, label, value).in_section(section));
    };
    let subject = cert.subject().to_string();
    let issuer = cert.issuer().to_string();
    let self_signed = cert.subject().as_raw() == cert.issuer().as_raw();
    add("subject", en!("Subject"), subject.clone());
    add(
        "issuer",
        en!("Issuer"),
        if self_signed {
            format!("{issuer} (itself)")
        } else {
            issuer.clone()
        },
    );
    add("serial", en!("Serial"), cert.raw_serial_as_string());
    let (from, until) = (
        cert.validity().not_before.timestamp(),
        cert.validity().not_after.timestamp(),
    );
    add("not_before", en!("Not before"), time(from));
    add("not_after", en!("Not after"), time(until));
    let status = if at < from {
        format!("not yet valid: from {}, in {}", time(from), span(from - at))
    } else if at > until {
        format!("expired {} ago", span(at - until))
    } else {
        format!("valid, expires in {}", span(until - at))
    };
    add("status", en!("Status"), status.clone());
    let sans: Vec<String> = cert
        .subject_alternative_name()
        .ok()
        .flatten()
        .map(|e| e.value.general_names.iter().map(san).collect())
        .unwrap_or_default();
    if !sans.is_empty() {
        add("sans", en!("Names (SAN)"), sans.join(", "));
    }
    add("key", en!("Key"), key(cert));
    let signature = oid2sn(&cert.signature_algorithm.algorithm, oid_registry()).map_or_else(
        |_| cert.signature_algorithm.algorithm.to_id_string(),
        str::to_owned,
    );
    add("signature", en!("Signature"), signature.clone());
    let ca = match cert.basic_constraints().ok().flatten().map(|e| e.value) {
        Some(bc) if bc.ca => match bc.path_len_constraint {
            Some(n) => format!("yes, up to {n} below it"),
            None => "yes".to_owned(),
        },
        _ => "no".to_owned(),
    };
    add("ca", en!("CA"), ca.clone());
    let usage = cert.key_usage().ok().flatten().map(|e| e.value.to_string());
    if let Some(u) = &usage {
        add("key_usage", en!("Key usage"), u.clone());
    }
    let extended = cert.extended_key_usage().ok().flatten().map(|e| {
        let e = e.value;
        let mut v: Vec<String> = [
            (e.server_auth, "TLS server"),
            (e.client_auth, "TLS client"),
            (e.code_signing, "code signing"),
            (e.email_protection, "email"),
            (e.time_stamping, "time stamping"),
            (e.ocsp_signing, "OCSP signing"),
            (e.any, "any"),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .map(|(_, name)| name.to_owned())
        .collect();
        v.extend(e.other.iter().map(|o| o.to_id_string()));
        v.join(", ")
    });
    if let Some(e) = &extended {
        add("extended_key_usage", en!("Extended key usage"), e.clone());
    }
    let mut ski = None;
    let mut aki = None;
    for ext in cert.iter_extensions() {
        match ext.parsed_extension() {
            ParsedExtension::SubjectKeyIdentifier(id) => ski = Some(colons(id.0)),
            ParsedExtension::AuthorityKeyIdentifier(a) => {
                aki = a.key_identifier.as_ref().map(|id| colons(id.0))
            }
            _ => {}
        }
    }
    if let Some(k) = &ski {
        add("subject_key_id", en!("Subject key id"), k.clone());
    }
    if let Some(k) = &aki {
        add("authority_key_id", en!("Authority key id"), k.clone());
    }
    let sha256 = colons(&sha2::Sha256::digest(der));
    let sha1 = colons(&sha1::Sha1::digest(der));
    add("sha256", en!("SHA-256 fingerprint"), sha256.clone());
    add("sha1", en!("SHA-1 fingerprint"), sha1.clone());
    let json = serde_json::json!({
        "subject": subject, "issuer": issuer, "self_signed": self_signed,
        "serial": cert.raw_serial_as_string(),
        "not_before": time(from), "not_after": time(until), "status": status,
        "sans": sans, "key": key(cert), "signature": signature, "ca": ca,
        "key_usage": usage, "extended_key_usage": extended,
        "subject_key_id": ski, "authority_key_id": aki,
        "sha256": sha256, "sha1": sha1,
    });
    (fields, json)
}

/// Whether the chain is in order (each issued by the next, leaf first), and
/// if not, the order it goes in, when the certificates make one chain.
fn chain(certs: &[X509Certificate]) -> String {
    let issued_by =
        |a: &X509Certificate, b: &X509Certificate| a.issuer().as_raw() == b.subject().as_raw();
    let self_signed = |c: &X509Certificate| issued_by(c, c);
    let in_order = certs.windows(2).all(|w| issued_by(&w[0], &w[1]));
    let ends = certs.last().is_some_and(self_signed);
    if in_order {
        let end = if ends {
            ", ending at a self-signed root"
        } else {
            ", the root left out"
        };
        return format!("in order, leaf first{end}");
    }
    // The leaf issues none of the others; from it, follow the issuers.
    let leaf = (0..certs.len()).find(|&i| {
        !certs
            .iter()
            .enumerate()
            .any(|(j, c)| j != i && issued_by(c, &certs[i]))
    });
    let mut order = Vec::new();
    let mut at = leaf;
    while let Some(i) = at {
        if order.contains(&i) {
            break;
        }
        order.push(i);
        at = (0..certs.len()).find(|&j| j != i && issued_by(&certs[i], &certs[j]));
    }
    if order.len() == certs.len() {
        let order: Vec<String> = order.iter().map(|i| (i + 1).to_string()).collect();
        format!("not in order: it goes {}", order.join(", "))
    } else {
        "not one chain: some certificate's issuer is not here".to_owned()
    }
}

impl Tool for Read {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let at = match values.text(PARAMS, "at").trim() {
            "" => jiff::Timestamp::now(),
            t => t
                .parse::<jiff::Timestamp>()
                .map_err(|e| Error::usage(format!("--at is not an RFC 3339 time: {e}")))?,
        }
        .as_second();
        let ders = certificates(input)?;
        let mut parsed = Vec::new();
        for (i, der) in ders.iter().enumerate() {
            let (_, cert) = x509_parser::parse_x509_certificate(der)
                .map_err(|e| Error::input(format!("certificate {} does not parse: {e}", i + 1)))?;
            parsed.push(cert);
        }
        let mut fields = Vec::new();
        let mut json = Vec::new();
        let many = parsed.len() > 1;
        if many {
            fields.push(Field::new("chain", en!("Chain"), chain(&parsed)));
        }
        for (i, (cert, der)) in parsed.iter().zip(&ders).enumerate() {
            let cn = cert
                .subject()
                .iter_common_name()
                .next()
                .and_then(|c| c.as_str().ok())
                .unwrap_or("");
            let section = match (many, cn) {
                (false, _) => "Certificate".to_owned(),
                (true, "") => format!("Certificate {}", i + 1),
                (true, cn) => format!("Certificate {} · {cn}", i + 1),
            };
            let (f, j) = describe(cert, der, at, &section);
            fields.extend(f.into_iter().map(|f| Field {
                key: format!("{}.{}", i + 1, f.key),
                ..f
            }));
            json.push(j);
        }
        let json = if many {
            serde_json::Value::from(json)
        } else {
            json.remove(0)
        };
        Ok(Output::Record {
            fields,
            json: json.to_string(),
        })
    }

    fn detect(&self, sample: &[u8]) -> Option<Score> {
        sample
            .windows(27)
            .any(|w| w == b"-----BEGIN CERTIFICATE-----")
            .then_some(Score(95))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(file: &str, at: &str) -> Result<(Vec<(String, String)>, serde_json::Value), Error> {
        let path = format!("{}/testdata/cert/{file}", env!("CARGO_MANIFEST_DIR"));
        let input = std::fs::read(&path).map_err(|e| Error::input(e.to_string()))?;
        let mut v = Values::new();
        v.set("at", Value::Text(at.into()));
        match Read.run(&Ctx::new(), &input, &v)? {
            Output::Record { fields, json } => Ok((
                fields.into_iter().map(|f| (f.key, f.value)).collect(),
                serde_json::from_str(&json).unwrap_or_default(),
            )),
            _ => Err(Error::input("not a record")),
        }
    }

    fn get<'a>(fields: &'a [(String, String)], key: &str) -> Option<&'a str> {
        fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// testdata/cert/leaf.pem, as OpenSSL 3.5.7 reads it (leaf.openssl.txt).
    #[test]
    fn reads_what_openssl_reads() {
        let (f, json) = read("leaf.pem", "2026-11-04T00:00:00Z").unwrap_or_default();
        assert_eq!(
            get(&f, "1.subject"),
            Some("O=Example Test, CN=api.example.test")
        );
        assert_eq!(
            get(&f, "1.issuer"),
            Some("C=VN, O=Toolwing Test, CN=Toolwing Test Intermediate")
        );
        assert_eq!(get(&f, "1.serial"), Some("0a:1b:2c:3d:4e:5f"));
        assert_eq!(get(&f, "1.not_before"), Some("2026-03-01T00:00:00Z"));
        assert_eq!(get(&f, "1.not_after"), Some("2027-03-01T00:00:00Z"));
        assert_eq!(get(&f, "1.status"), Some("valid, expires in 3 months"));
        assert_eq!(
            get(&f, "1.sans"),
            Some(
                "DNS:api.example.test, DNS:*.example.test, IP:10.0.0.12, IP:2001:db8::1, email:ops@example.test"
            )
        );
        assert_eq!(get(&f, "1.key"), Some("Ed25519"));
        assert_eq!(get(&f, "1.signature"), Some("ecdsa-with-SHA256"));
        assert_eq!(get(&f, "1.ca"), Some("no"));
        assert_eq!(get(&f, "1.key_usage"), Some("Digital Signature"));
        assert_eq!(
            get(&f, "1.extended_key_usage"),
            Some("TLS server, TLS client")
        );
        assert_eq!(
            get(&f, "1.subject_key_id"),
            Some("05:2E:CA:E7:00:0B:80:5E:3C:4C:76:7C:EF:AA:45:57:3D:73:03:23")
        );
        assert_eq!(
            get(&f, "1.authority_key_id"),
            Some("4D:43:EB:0A:1A:18:AC:F1:51:A9:98:1E:80:B8:81:F5:59:CF:43:2A")
        );
        assert_eq!(
            get(&f, "1.sha256"),
            Some(
                "18:76:7F:48:AF:62:92:7D:17:14:26:BF:3C:D1:BF:73:AD:5A:38:2D:6A:5E:B1:39:B4:B8:D3:8C:0D:A2:90:2A"
            )
        );
        assert_eq!(
            get(&f, "1.sha1"),
            Some("D7:26:06:51:9C:3B:E0:B1:C0:D7:AD:C2:ED:BE:6B:48:EE:B4:FB:86")
        );
        assert_eq!(json["sans"][3], "IP:2001:db8::1");
        // DER says the same.
        let (der, _) = read("leaf.der", "2026-11-04T00:00:00Z").unwrap_or_default();
        assert_eq!(get(&der, "1.sha256"), get(&f, "1.sha256"));
    }

    #[test]
    fn keys_cas_and_validity() {
        let (root, _) = read("root.pem", "2026-11-04T00:00:00Z").unwrap_or_default();
        assert_eq!(get(&root, "1.key"), Some("EC, P-256"));
        assert_eq!(get(&root, "1.ca"), Some("yes"));
        assert!(get(&root, "1.issuer").is_some_and(|i| i.ends_with("(itself)")));
        let (inter, _) = read("inter.pem", "2026-11-04T00:00:00Z").unwrap_or_default();
        assert_eq!(get(&inter, "1.key"), Some("EC, P-384"));
        assert_eq!(get(&inter, "1.ca"), Some("yes, up to 0 below it"));
        let (old, _) = read("expired.pem", "2026-11-04T00:00:00Z").unwrap_or_default();
        assert_eq!(get(&old, "1.key"), Some("RSA, 2048 bits"));
        assert_eq!(get(&old, "1.status"), Some("expired 5 years ago"));
        let (early, _) = read("leaf.pem", "2026-01-15T00:00:00Z").unwrap_or_default();
        assert!(get(&early, "1.status").is_some_and(|s| s.starts_with("not yet valid")));
    }

    #[test]
    fn chains_in_order_and_not() {
        let (f, json) = read("chain.pem", "2026-11-04T00:00:00Z").unwrap_or_default();
        assert_eq!(
            get(&f, "chain"),
            Some("in order, leaf first, ending at a self-signed root")
        );
        assert_eq!(json.as_array().map(Vec::len), Some(3));
        let (f, _) = read("chain-out-of-order.pem", "2026-11-04T00:00:00Z").unwrap_or_default();
        assert_eq!(get(&f, "chain"), Some("not in order: it goes 2, 3, 1"));
    }

    #[test]
    fn says_what_it_is_given_when_not_a_certificate() {
        let key =
            b"-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIA==\n-----END PRIVATE KEY-----\n";
        let e = Read
            .run(&Ctx::new(), key, &Values::new())
            .err()
            .map(|e| e.message);
        assert!(e.is_some_and(|m| m.contains("private key") && !m.contains("MC4C")));
        assert!(Read.run(&Ctx::new(), b"hello", &Values::new()).is_err());
    }
}
