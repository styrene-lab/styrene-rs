//! Styrene identity-bound X.509 certificate issuance.
//!
//! This module provides the low-level PKI operations used by control-plane
//! transports. Key material is deterministically derived from the Styrene root
//! through the TLS certificate key family, then encoded as Ed25519 PKCS#8 for `rcgen`.

use rcgen::{
    date_time_ymd, BasicConstraints, CertificateParams, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose, SanType, SerialNumber,
};
use sha2::{Digest, Sha256};
use std::fmt;
use std::net::IpAddr;
use zeroize::Zeroizing;

use crate::derive::KeyDeriver;
use crate::identity::identity_hash;
use crate::signer::RootSecret;

const SPIFFE_BASE: &str = "spiffe://styrene.dev";
const MAX_LABEL_BYTES: usize = 256;

/// Deterministic issuance profile for certificate validity and rotation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyreneCertificateProfile {
    pub profile: String,
    pub ca_epoch: String,
    pub leaf_epoch: String,
    pub ca_not_before_year: i32,
    pub ca_not_after_year: i32,
    pub leaf_not_before_year: i32,
    pub leaf_not_after_year: i32,
}

impl Default for StyreneCertificateProfile {
    fn default() -> Self {
        Self {
            profile: "default".to_string(),
            ca_epoch: "0".to_string(),
            leaf_epoch: "0".to_string(),
            ca_not_before_year: 2026,
            ca_not_after_year: 2036,
            leaf_not_before_year: 2026,
            leaf_not_after_year: 2031,
        }
    }
}

impl StyreneCertificateProfile {
    /// Use a different leaf epoch while retaining the same CA epoch.
    pub fn with_leaf_epoch(mut self, leaf_epoch: impl Into<String>) -> Self {
        self.leaf_epoch = leaf_epoch.into();
        self
    }

    /// Use a different CA epoch. This also changes the signing trust anchor.
    pub fn with_ca_epoch(mut self, ca_epoch: impl Into<String>) -> Self {
        self.ca_epoch = ca_epoch.into();
        self
    }

    /// Group independent issuance policies under a named profile.
    pub fn with_profile(mut self, profile: impl Into<String>) -> Self {
        self.profile = profile.into();
        self
    }
}

/// Role encoded into certificate usages and Styrene URI SANs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CertificateRole {
    Ca,
    Server,
    Client,
}

/// PEM/DER certificate material derived from a Styrene identity.
pub struct StyreneCertificate {
    pub role: CertificateRole,
    pub identity_hash: String,
    pub label: String,
    pub profile: String,
    pub epoch: String,
    pub uri_san: String,
    pub cert_pem: String,
    pub cert_der: Vec<u8>,
    private_key_pem: Zeroizing<String>,
    private_key_der: Zeroizing<Vec<u8>>,
    pub fingerprint_sha256: String,
}

impl StyreneCertificate {
    /// Borrow the private key as PEM. Ownership stays with the certificate for zeroization.
    pub fn private_key_pem(&self) -> &str {
        self.private_key_pem.as_str()
    }

    /// Borrow the private key as PKCS#8 DER. Ownership stays with the certificate for zeroization.
    pub fn private_key_der(&self) -> &[u8] {
        self.private_key_der.as_slice()
    }
}

impl fmt::Debug for StyreneCertificate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StyreneCertificate")
            .field("role", &self.role)
            .field("identity_hash", &self.identity_hash)
            .field("label", &self.label)
            .field("profile", &self.profile)
            .field("epoch", &self.epoch)
            .field("uri_san", &self.uri_san)
            .field("cert_pem", &self.cert_pem)
            .field("cert_der", &self.cert_der)
            .field("private_key_pem", &"[REDACTED]")
            .field("private_key_der", &"[REDACTED]")
            .field("fingerprint_sha256", &self.fingerprint_sha256)
            .finish()
    }
}

/// A CA certificate plus a leaf certificate signed by it.
#[derive(Debug)]
pub struct StyreneCertificateChain {
    pub profile: StyreneCertificateProfile,
    pub ca_cert_pem: String,
    pub ca_cert_der: Vec<u8>,
    pub ca_fingerprint_sha256: String,
    pub leaf: StyreneCertificate,
}

impl StyreneCertificateChain {
    /// Return the leaf certificate followed by its issuing CA certificate.
    pub fn cert_chain_pem(&self) -> String {
        format!("{}{}", self.leaf.cert_pem, self.ca_cert_pem)
    }

    /// Return the trust anchor for clients that should verify this chain.
    pub fn ca_bundle_pem(&self) -> &str {
        &self.ca_cert_pem
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StyrenePkiError {
    #[error("certificate label must not be empty")]
    EmptyLabel,
    #[error("certificate label is invalid: {0}")]
    InvalidLabel(&'static str),
    #[error("certificate validity window is invalid")]
    InvalidValidity,
    #[error("certificate generation failed: {0}")]
    Certificate(#[from] rcgen::Error),
}

pub fn styrene_agent_uri(identity_hash: &str, agent_label: &str) -> String {
    styrene_uri(identity_hash, "agent", agent_label)
}

pub fn styrene_client_uri(identity_hash: &str, client_label: &str) -> String {
    styrene_uri(identity_hash, "client", client_label)
}

pub fn styrene_ca_uri(identity_hash: &str, ca_scope: &str) -> String {
    styrene_uri(identity_hash, "ca", ca_scope)
}

/// Derive a self-signed CA certificate for a Styrene identity and scope.
pub fn derive_ca_certificate(
    root: &RootSecret,
    ca_scope: &str,
) -> Result<StyreneCertificate, StyrenePkiError> {
    derive_ca_certificate_with_profile(root, ca_scope, &StyreneCertificateProfile::default())
}

/// Derive a self-signed CA certificate for a Styrene identity, scope, and rotation profile.
pub fn derive_ca_certificate_with_profile(
    root: &RootSecret,
    ca_scope: &str,
    profile: &StyreneCertificateProfile,
) -> Result<StyreneCertificate, StyrenePkiError> {
    validate_label(ca_scope)?;
    validate_profile(profile)?;
    let identity_hash = identity_hash(root);
    let seed_label = format!("styrene/tls/ca/{}/{ca_scope}/{}", profile.profile, profile.ca_epoch);
    let uri = styrene_ca_uri(&identity_hash, ca_scope);
    let key_pair = key_pair_from_derived_seed(root, &seed_label)?;
    let params = ca_params(&identity_hash, ca_scope, &uri, profile)?;
    let cert = params.self_signed(&key_pair)?;
    Ok(material_from_cert(
        CertificateRole::Ca,
        identity_hash,
        ca_scope,
        &profile.profile,
        &profile.ca_epoch,
        uri,
        cert,
        key_pair,
    ))
}

/// Derive a server certificate signed by a deterministic Styrene CA.
pub fn derive_server_certificate_chain(
    root: &RootSecret,
    ca_scope: &str,
    agent_label: &str,
    subject_alt_names: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<StyreneCertificateChain, StyrenePkiError> {
    derive_server_certificate_chain_with_profile(
        root,
        ca_scope,
        agent_label,
        subject_alt_names,
        &StyreneCertificateProfile::default(),
    )
}

/// Derive a server certificate chain for a specific issuance profile.
pub fn derive_server_certificate_chain_with_profile(
    root: &RootSecret,
    ca_scope: &str,
    agent_label: &str,
    subject_alt_names: impl IntoIterator<Item = impl AsRef<str>>,
    profile: &StyreneCertificateProfile,
) -> Result<StyreneCertificateChain, StyrenePkiError> {
    derive_leaf_certificate_chain(
        root,
        ca_scope,
        CertificateRole::Server,
        agent_label,
        subject_alt_names,
        profile,
    )
}

/// Derive a client certificate signed by a deterministic Styrene CA.
pub fn derive_client_certificate_chain(
    root: &RootSecret,
    ca_scope: &str,
    client_label: &str,
) -> Result<StyreneCertificateChain, StyrenePkiError> {
    derive_client_certificate_chain_with_profile(
        root,
        ca_scope,
        client_label,
        &StyreneCertificateProfile::default(),
    )
}

/// Derive a client certificate chain for a specific issuance profile.
pub fn derive_client_certificate_chain_with_profile(
    root: &RootSecret,
    ca_scope: &str,
    client_label: &str,
    profile: &StyreneCertificateProfile,
) -> Result<StyreneCertificateChain, StyrenePkiError> {
    derive_leaf_certificate_chain(
        root,
        ca_scope,
        CertificateRole::Client,
        client_label,
        std::iter::empty::<&str>(),
        profile,
    )
}

fn derive_leaf_certificate_chain(
    root: &RootSecret,
    ca_scope: &str,
    role: CertificateRole,
    label: &str,
    subject_alt_names: impl IntoIterator<Item = impl AsRef<str>>,
    profile: &StyreneCertificateProfile,
) -> Result<StyreneCertificateChain, StyrenePkiError> {
    validate_label(ca_scope)?;
    validate_label(label)?;
    validate_profile(profile)?;
    let identity_hash = identity_hash(root);
    let ca_seed_label =
        format!("styrene/tls/ca/{}/{ca_scope}/{}", profile.profile, profile.ca_epoch);
    let ca_uri = styrene_ca_uri(&identity_hash, ca_scope);
    let ca_key_pair = key_pair_from_derived_seed(root, &ca_seed_label)?;
    let ca_params = ca_params(&identity_hash, ca_scope, &ca_uri, profile)?;
    let ca_cert = ca_params.self_signed(&ca_key_pair)?;

    let issuer = rcgen::Issuer::new(ca_params, ca_key_pair);

    let leaf_seed_label = match role {
        CertificateRole::Server => {
            format!(
                "styrene/tls/server/{}/{ca_scope}/{label}/{}",
                profile.profile, profile.leaf_epoch
            )
        }
        CertificateRole::Client => {
            format!(
                "styrene/tls/client/{}/{ca_scope}/{label}/{}",
                profile.profile, profile.leaf_epoch
            )
        }
        CertificateRole::Ca => return Err(StyrenePkiError::EmptyLabel),
    };
    let leaf_uri = match role {
        CertificateRole::Server => styrene_agent_uri(&identity_hash, label),
        CertificateRole::Client => styrene_client_uri(&identity_hash, label),
        CertificateRole::Ca => unreachable!(),
    };
    let leaf_key_pair = key_pair_from_derived_seed(root, &leaf_seed_label)?;
    let leaf_params =
        leaf_params(&identity_hash, role, label, &leaf_uri, subject_alt_names, profile)?;
    let leaf_cert = leaf_params.signed_by(&leaf_key_pair, &issuer)?;

    let ca_cert_der = ca_cert.der().to_vec();
    let ca_fingerprint_sha256 = fingerprint_sha256(&ca_cert_der);
    Ok(StyreneCertificateChain {
        profile: profile.clone(),
        ca_cert_pem: ca_cert.pem(),
        ca_cert_der,
        ca_fingerprint_sha256,
        leaf: material_from_cert(
            role,
            identity_hash,
            label,
            &profile.profile,
            &profile.leaf_epoch,
            leaf_uri,
            leaf_cert,
            leaf_key_pair,
        ),
    })
}

fn ca_params(
    identity_hash: &str,
    scope: &str,
    uri: &str,
    profile: &StyreneCertificateProfile,
) -> Result<CertificateParams, StyrenePkiError> {
    let mut params = CertificateParams::new(Vec::<String>::new())?;
    params.serial_number = Some(deterministic_serial(&[
        "ca",
        identity_hash,
        scope,
        uri,
        &profile.profile,
        &profile.ca_epoch,
        "v1",
    ]));
    params.not_before = date_time_ymd(profile.ca_not_before_year, 1, 1);
    params.not_after = date_time_ymd(profile.ca_not_after_year, 1, 1);
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, format!("styrene:{identity_hash}:ca:{scope}"));
    params.subject_alt_names.push(SanType::URI(uri.try_into()?));
    params.key_usages.push(KeyUsagePurpose::DigitalSignature);
    params.key_usages.push(KeyUsagePurpose::KeyCertSign);
    params.key_usages.push(KeyUsagePurpose::CrlSign);
    Ok(params)
}

fn leaf_params(
    identity_hash: &str,
    role: CertificateRole,
    label: &str,
    uri: &str,
    subject_alt_names: impl IntoIterator<Item = impl AsRef<str>>,
    profile: &StyreneCertificateProfile,
) -> Result<CertificateParams, StyrenePkiError> {
    validate_label(label)?;
    let normalized_subject_alt_names = normalize_subject_alt_names(subject_alt_names)?;

    let mut params = CertificateParams::new(Vec::<String>::new())?;
    let mut serial_parts = vec![
        role_name(role),
        identity_hash,
        label,
        uri,
        &profile.profile,
        &profile.ca_epoch,
        &profile.leaf_epoch,
        "v1",
    ];
    serial_parts
        .extend(normalized_subject_alt_names.iter().map(|(serial_part, _)| serial_part.as_str()));
    params.serial_number = Some(deterministic_serial(&serial_parts));
    params.not_before = date_time_ymd(profile.leaf_not_before_year, 1, 1);
    params.not_after = date_time_ymd(profile.leaf_not_after_year, 1, 1);
    params.is_ca = IsCa::ExplicitNoCa;
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, format!("styrene:{identity_hash}:{}:{label}", role_name(role)));
    params.subject_alt_names.push(SanType::URI(uri.try_into()?));
    for (_, subject_alt_name) in normalized_subject_alt_names {
        params.subject_alt_names.push(subject_alt_name);
    }
    params.key_usages.push(KeyUsagePurpose::DigitalSignature);
    match role {
        CertificateRole::Server => {
            params.extended_key_usages.push(ExtendedKeyUsagePurpose::ServerAuth)
        }
        CertificateRole::Client => {
            params.extended_key_usages.push(ExtendedKeyUsagePurpose::ClientAuth)
        }
        CertificateRole::Ca => unreachable!(),
    }
    params.use_authority_key_identifier_extension = true;
    Ok(params)
}

fn key_pair_from_derived_seed(root: &RootSecret, label: &str) -> Result<KeyPair, StyrenePkiError> {
    if label.is_empty() {
        return Err(StyrenePkiError::EmptyLabel);
    }
    let deriver = KeyDeriver::new(root.as_bytes());
    let seed = Zeroizing::new(deriver.derive_tls_certificate_key(label)?);
    let pkcs8 = Zeroizing::new(ed25519_seed_to_pkcs8_der(&seed));
    Ok(KeyPair::try_from(pkcs8.as_slice())?)
}

fn material_from_cert(
    role: CertificateRole,
    identity_hash: String,
    label: &str,
    profile: &str,
    epoch: &str,
    uri_san: String,
    cert: rcgen::Certificate,
    key_pair: KeyPair,
) -> StyreneCertificate {
    material_from_raw(
        role,
        identity_hash,
        label,
        profile,
        epoch,
        uri_san,
        cert,
        Zeroizing::new(key_pair.serialize_pem()),
        Zeroizing::new(key_pair.serialize_der()),
    )
}

fn material_from_raw(
    role: CertificateRole,
    identity_hash: String,
    label: &str,
    profile: &str,
    epoch: &str,
    uri_san: String,
    cert: rcgen::Certificate,
    private_key_pem: Zeroizing<String>,
    private_key_der: Zeroizing<Vec<u8>>,
) -> StyreneCertificate {
    let cert_der = cert.der().to_vec();
    let fingerprint_sha256 = fingerprint_sha256(&cert_der);
    StyreneCertificate {
        role,
        identity_hash,
        label: label.to_string(),
        profile: profile.to_string(),
        epoch: epoch.to_string(),
        uri_san,
        cert_pem: cert.pem(),
        cert_der,
        private_key_pem,
        private_key_der,
        fingerprint_sha256,
    }
}

fn fingerprint_sha256(der: &[u8]) -> String {
    hex::encode(Sha256::digest(der))
}

fn role_name(role: CertificateRole) -> &'static str {
    match role {
        CertificateRole::Ca => "ca",
        CertificateRole::Server => "server",
        CertificateRole::Client => "client",
    }
}

fn validate_label(value: &str) -> Result<(), StyrenePkiError> {
    if value.is_empty() {
        return Err(StyrenePkiError::EmptyLabel);
    }
    if value.trim() != value {
        return Err(StyrenePkiError::InvalidLabel("leading or trailing whitespace"));
    }
    if value.len() > MAX_LABEL_BYTES {
        return Err(StyrenePkiError::InvalidLabel("too long"));
    }
    if value.chars().any(char::is_control) {
        return Err(StyrenePkiError::InvalidLabel("control character"));
    }
    Ok(())
}

fn validate_profile(profile: &StyreneCertificateProfile) -> Result<(), StyrenePkiError> {
    validate_label(&profile.profile)?;
    validate_label(&profile.ca_epoch)?;
    validate_label(&profile.leaf_epoch)?;
    if profile.ca_not_after_year <= profile.ca_not_before_year {
        return Err(StyrenePkiError::InvalidValidity);
    }
    if profile.leaf_not_after_year <= profile.leaf_not_before_year {
        return Err(StyrenePkiError::InvalidValidity);
    }
    if profile.leaf_not_before_year < profile.ca_not_before_year {
        return Err(StyrenePkiError::InvalidValidity);
    }
    if profile.leaf_not_after_year > profile.ca_not_after_year {
        return Err(StyrenePkiError::InvalidValidity);
    }
    Ok(())
}

fn normalize_subject_alt_names(
    subject_alt_names: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<Vec<(String, SanType)>, StyrenePkiError> {
    let mut normalized = Vec::new();
    for subject_alt_name in subject_alt_names {
        let subject_alt_name = subject_alt_name.as_ref().trim();
        if subject_alt_name.is_empty() {
            continue;
        }

        if let Ok(ip_addr) = subject_alt_name.parse::<IpAddr>() {
            normalized.push((format!("ip:{ip_addr}"), SanType::IpAddress(ip_addr)));
        } else {
            let dns_name = subject_alt_name.to_ascii_lowercase();
            normalized.push((format!("dns:{dns_name}"), SanType::DnsName(dns_name.try_into()?)));
        }
    }

    normalized.sort_by(|a, b| a.0.cmp(&b.0));
    normalized.dedup_by(|a, b| a.0 == b.0);
    Ok(normalized)
}

fn deterministic_serial(parts: &[&str]) -> SerialNumber {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    let mut serial = [0u8; 20];
    serial.copy_from_slice(&digest[..20]);
    serial[0] &= 0x7f;
    if serial[0] == 0 {
        serial[0] = 1;
    }
    SerialNumber::from_slice(&serial)
}

fn styrene_uri(identity_hash: &str, role: &str, label: &str) -> String {
    format!(
        "{SPIFFE_BASE}/identity/{}/{}{}",
        pct_encode_path(identity_hash),
        role,
        if label.is_empty() { String::new() } else { format!("/{}", pct_encode_path(label)) }
    )
}

fn pct_encode_path(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn ed25519_seed_to_pkcs8_der(seed: &[u8; 32]) -> Vec<u8> {
    let mut der = Vec::with_capacity(48);
    der.extend_from_slice(&[
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20,
    ]);
    der.extend_from_slice(seed);
    der
}

impl From<crate::derive::DeriveError> for StyrenePkiError {
    fn from(value: crate::derive::DeriveError) -> Self {
        match value {
            crate::derive::DeriveError::EmptyLabel => Self::EmptyLabel,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use x509_parser::extensions::{GeneralName, ParsedExtension};
    use x509_parser::prelude::{FromDer, X509Certificate};

    fn root() -> RootSecret {
        RootSecret::new([0x42; 32])
    }

    fn parse_cert(cert: &StyreneCertificate) -> X509Certificate<'_> {
        let (_, parsed) = X509Certificate::from_der(&cert.cert_der).unwrap();
        parsed
    }

    #[test]
    fn ca_certificate_is_deterministic_and_identity_bound() {
        let root = root();
        let a = derive_ca_certificate(&root, "auspex-control").unwrap();
        let b = derive_ca_certificate(&root, "auspex-control").unwrap();

        assert_eq!(a.cert_der, b.cert_der);
        assert_eq!(a.private_key_der(), b.private_key_der());
        assert_eq!(a.identity_hash, identity_hash(&root));
        assert_eq!(a.uri_san, styrene_ca_uri(&identity_hash(&root), "auspex-control"));

        let parsed = parse_cert(&a);
        assert_eq!(parsed.tbs_certificate.raw_serial().len(), 20);
        assert_ne!(parsed.tbs_certificate.raw_serial(), &[0u8; 20]);
    }

    #[test]
    fn server_certificate_has_uri_dns_and_ip_sans() {
        let root = root();
        let chain = derive_server_certificate_chain(
            &root,
            "cluster-a",
            "omegon-primary",
            ["omegon-primary.default.svc", "127.0.0.1"],
        )
        .unwrap();
        let parsed = parse_cert(&chain.leaf);
        let sans = parsed.subject_alternative_name().unwrap().unwrap();

        assert!(sans
            .value
            .general_names
            .iter()
            .any(|name| matches!(name, GeneralName::URI(uri) if *uri == chain.leaf.uri_san)));
        assert!(sans.value.general_names.iter().any(|name| {
            matches!(name, GeneralName::DNSName(name) if *name == "omegon-primary.default.svc")
        }));
        assert!(sans
            .value
            .general_names
            .iter()
            .any(|name| matches!(name, GeneralName::IPAddress(bytes) if *bytes == [127, 0, 0, 1])));
        assert!(chain.cert_chain_pem().starts_with(&chain.leaf.cert_pem));
        assert_eq!(chain.ca_bundle_pem(), chain.ca_cert_pem);
        assert_eq!(chain.ca_fingerprint_sha256, fingerprint_sha256(&chain.ca_cert_der));
    }

    #[test]
    fn server_subject_alt_names_are_canonicalized_for_determinism() {
        let root = root();
        let a = derive_server_certificate_chain(
            &root,
            "cluster-a",
            "omegon-primary",
            ["OMEGON-PRIMARY.DEFAULT.SVC", "127.0.0.1", "127.0.0.1"],
        )
        .unwrap();
        let b = derive_server_certificate_chain(
            &root,
            "cluster-a",
            "omegon-primary",
            ["127.0.0.1", "omegon-primary.default.svc"],
        )
        .unwrap();

        assert_eq!(a.leaf.cert_der, b.leaf.cert_der);
    }

    #[test]
    fn leaf_epoch_rotates_leaf_without_rotating_ca_or_identity_binding() {
        let root = root();
        let current = StyreneCertificateProfile::default().with_leaf_epoch("2026q1");
        let next = StyreneCertificateProfile::default().with_leaf_epoch("2026q2");

        let a = derive_server_certificate_chain_with_profile(
            &root,
            "cluster-a",
            "omegon-primary",
            ["omegon-primary.default.svc"],
            &current,
        )
        .unwrap();
        let b = derive_server_certificate_chain_with_profile(
            &root,
            "cluster-a",
            "omegon-primary",
            ["omegon-primary.default.svc"],
            &next,
        )
        .unwrap();

        assert_eq!(a.ca_cert_der, b.ca_cert_der);
        assert_ne!(a.leaf.cert_der, b.leaf.cert_der);
        assert_ne!(a.leaf.private_key_der(), b.leaf.private_key_der());
        assert_eq!(a.leaf.identity_hash, b.leaf.identity_hash);
        assert_eq!(a.leaf.uri_san, b.leaf.uri_san);
        assert_eq!(a.leaf.epoch, "2026q1");
        assert_eq!(b.leaf.epoch, "2026q2");
    }

    #[test]
    fn ca_epoch_rotates_trust_anchor() {
        let root = root();
        let current = StyreneCertificateProfile::default().with_ca_epoch("2026h1");
        let next = StyreneCertificateProfile::default().with_ca_epoch("2026h2");

        let a = derive_client_certificate_chain_with_profile(
            &root,
            "cluster-a",
            "auspex-desktop",
            &current,
        )
        .unwrap();
        let b = derive_client_certificate_chain_with_profile(
            &root,
            "cluster-a",
            "auspex-desktop",
            &next,
        )
        .unwrap();

        assert_ne!(a.ca_cert_der, b.ca_cert_der);
        assert_ne!(a.ca_bundle_pem(), b.ca_bundle_pem());
        assert_ne!(a.leaf.cert_der, b.leaf.cert_der);
        assert_eq!(a.leaf.identity_hash, b.leaf.identity_hash);
        assert_eq!(a.leaf.uri_san, b.leaf.uri_san);
    }

    #[test]
    fn ca_scope_separates_leaf_private_keys() {
        let root = root();
        let a = derive_server_certificate_chain(
            &root,
            "cluster-a",
            "omegon-primary",
            ["omegon-primary.default.svc"],
        )
        .unwrap();
        let b = derive_server_certificate_chain(
            &root,
            "cluster-b",
            "omegon-primary",
            ["omegon-primary.default.svc"],
        )
        .unwrap();

        assert_ne!(a.leaf.private_key_der(), b.leaf.private_key_der());
        assert_eq!(a.leaf.identity_hash, b.leaf.identity_hash);
    }

    #[test]
    fn client_certificate_has_client_auth_usage() {
        let root = root();
        let chain = derive_client_certificate_chain(&root, "cluster-a", "auspex-desktop").unwrap();
        let parsed = parse_cert(&chain.leaf);
        let sans = parsed.subject_alternative_name().unwrap().unwrap();
        let eku = parsed.extended_key_usage().unwrap().unwrap();

        assert!(sans
            .value
            .general_names
            .iter()
            .any(|name| matches!(name, GeneralName::URI(uri) if *uri == chain.leaf.uri_san)));
        assert!(eku.value.client_auth);
    }

    #[test]
    fn ca_certificate_marks_basic_constraints_ca() {
        let root = root();
        let ca = derive_ca_certificate(&root, "cluster-a").unwrap();
        let parsed = parse_cert(&ca);
        let basic_constraints = parsed
            .extensions()
            .iter()
            .find_map(|ext| match ext.parsed_extension() {
                ParsedExtension::BasicConstraints(value) => Some(value),
                _ => None,
            })
            .unwrap();

        assert!(basic_constraints.ca);
    }

    #[test]
    fn uri_labels_are_percent_encoded() {
        assert_eq!(
            styrene_agent_uri("abc", "primary driver/one"),
            "spiffe://styrene.dev/identity/abc/agent/primary%20driver%2Fone"
        );
    }

    #[test]
    fn rejects_blank_or_control_labels() {
        let root = root();

        assert!(matches!(derive_ca_certificate(&root, ""), Err(StyrenePkiError::EmptyLabel)));
        assert!(matches!(
            derive_ca_certificate(&root, " cluster-a"),
            Err(StyrenePkiError::InvalidLabel(_))
        ));
        assert!(matches!(
            derive_client_certificate_chain(&root, "cluster-a", "auspex\nclient"),
            Err(StyrenePkiError::InvalidLabel(_))
        ));
    }

    #[test]
    fn rejects_invalid_rotation_profiles() {
        let root = root();
        let empty_epoch = StyreneCertificateProfile::default().with_leaf_epoch("");
        let invalid_validity = StyreneCertificateProfile {
            leaf_not_before_year: 2030,
            leaf_not_after_year: 2029,
            ..StyreneCertificateProfile::default()
        };

        assert!(matches!(
            derive_client_certificate_chain_with_profile(
                &root,
                "cluster-a",
                "auspex-desktop",
                &empty_epoch
            ),
            Err(StyrenePkiError::EmptyLabel)
        ));
        assert!(matches!(
            derive_client_certificate_chain_with_profile(
                &root,
                "cluster-a",
                "auspex-desktop",
                &invalid_validity
            ),
            Err(StyrenePkiError::InvalidValidity)
        ));
    }

    #[test]
    fn debug_redacts_private_key_material() {
        let root = root();
        let cert = derive_ca_certificate(&root, "cluster-a").unwrap();
        let debug = format!("{cert:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains(cert.private_key_pem()));
        assert!(!debug.contains(&hex::encode(cert.private_key_der())));
    }
}
