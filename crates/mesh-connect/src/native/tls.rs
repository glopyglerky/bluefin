use std::sync::{Arc, OnceLock};
use rustls::{
    client::{danger::{ServerCertVerified, ServerCertVerifier, HandshakeSignatureValid}, ResolvesClientCert, WebPkiServerVerifier},
    server::{danger::{ClientCertVerified, ClientCertVerifier}, ClientHello, ResolvesServerCert, WebPkiClientVerifier},
    crypto::{CryptoProvider, WebPkiSupportedAlgorithms, verify_tls13_signature},
    pki_types::{CertificateDer, ServerName, UnixTime},
    sign::CertifiedKey,
    CipherSuite, ClientConfig, CommonState, DigitallySignedStruct, DistinguishedName,
    NamedGroup, ProtocolVersion, RootCertStore, ServerConfig, SignatureScheme,
};
use sha2::{Digest, Sha256};
use crate::{Error, Result};
use super::identity::{DeviceIdentity, verify_signature};

pub(crate) const SERVER_NAME:&str="mesh-connect.invalid";
pub(crate) const EXPORTER_LABEL:&[u8]=b"mesh-connect/device-binding/v1";
const ML_DSA_65_OID:&str="2.16.840.1.101.3.4.3.18";
const MAX_CERTIFICATE:usize=16*1024;

pub(crate) fn provider()->Result<Arc<CryptoProvider>>{
    static STRICT:OnceLock<std::result::Result<Arc<CryptoProvider>,()>>=OnceLock::new();
    STRICT.get_or_init(||{
        let mut p=rustls_provider_rustcrypto_pq::provider();
        p.cipher_suites.retain(|suite|suite.suite()==CipherSuite::TLS13_AES_256_GCM_SHA384);
        p.kx_groups.retain(|group|group.name()==NamedGroup::X25519MLKEM768);
        let selected=*p.signature_verification_algorithms.mapping.iter()
            .find(|(scheme,_)|*scheme==SignatureScheme::ML_DSA_65).ok_or(())?;
        if p.cipher_suites.len()!=1 || p.kx_groups.len()!=1 || selected.1.is_empty(){return Err(());}
        // The one retained immutable mapping lives for the process, like the
        // provider's original static algorithm tables. OnceLock bounds it.
        p.signature_verification_algorithms=WebPkiSupportedAlgorithms{
            all:selected.1,mapping:Box::leak(vec![selected].into_boxed_slice()),
        };
        Ok(Arc::new(p))
    }).as_ref().cloned().map_err(|_|Error::Crypto)
}
fn invalid_cert()->rustls::Error{
    rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure)
}
pub(crate) fn certificate_key(cert:&CertificateDer<'_>)->std::result::Result<Vec<u8>,rustls::Error>{
    if cert.is_empty() || cert.len()>MAX_CERTIFICATE{return Err(invalid_cert());}
    let (remaining,parsed)=x509_parser::parse_x509_certificate(cert.as_ref()).map_err(|_|invalid_cert())?;
    let key=&parsed.tbs_certificate.subject_pki;
    if !remaining.is_empty() || key.algorithm.algorithm.to_id_string()!=ML_DSA_65_OID
        || key.algorithm.parameters.is_some() || key.subject_public_key.unused_bits!=0
        || key.subject_public_key.data.len()!=crate::protocol::ML_DSA_65_PUBLIC_KEY_BYTES
        || parsed.signature_algorithm.algorithm.to_id_string()!=ML_DSA_65_OID
        || parsed.signature_algorithm.parameters.is_some()
        || parsed.tbs_certificate.signature.algorithm.to_id_string()!=ML_DSA_65_OID
        || parsed.tbs_certificate.signature.parameters.is_some()
        || parsed.signature_value.unused_bits!=0
        || parsed.tbs_certificate.subject!=parsed.tbs_certificate.issuer {
        return Err(invalid_cert());
    }
    let bytes=key.subject_public_key.data.to_vec();
    verify_signature(&bytes,parsed.tbs_certificate.as_ref(),parsed.signature_value.data.as_ref())
        .map_err(|_|invalid_cert())?;
    Ok(bytes)
}
fn roots(cert:&CertificateDer<'_>)->std::result::Result<Arc<RootCertStore>,rustls::Error>{
    let mut roots=RootCertStore::empty();roots.add(cert.clone().into_owned())?;
    Ok(Arc::new(roots))
}
fn tls13(message:&[u8],cert:&CertificateDer<'_>,sig:&DigitallySignedStruct,p:&CryptoProvider)
    ->std::result::Result<HandshakeSignatureValid,rustls::Error>{
    if sig.scheme!=SignatureScheme::ML_DSA_65{return Err(invalid_cert());}
    verify_tls13_signature(message,cert,sig,&p.signature_verification_algorithms)
}
#[derive(Debug)]
struct PinnedServer{pin:[u8;32],provider:Arc<CryptoProvider>}
impl ServerCertVerifier for PinnedServer{
    fn verify_server_cert(&self,end_entity:&CertificateDer<'_>,intermediates:&[CertificateDer<'_>],
        server_name:&ServerName<'_>,ocsp:&[u8],now:UnixTime)
        ->std::result::Result<ServerCertVerified,rustls::Error>{
        if !intermediates.is_empty()
            || <[u8;32]>::from(Sha256::digest(end_entity.as_ref()))!=self.pin{return Err(invalid_cert());}
        certificate_key(end_entity)?;
        WebPkiServerVerifier::builder_with_provider(roots(end_entity)?,self.provider.clone())
            .build().map_err(|_|invalid_cert())?
            .verify_server_cert(end_entity,&[],server_name,ocsp,now)
    }
    fn verify_tls12_signature(&self,_:&[u8],_:&CertificateDer<'_>,_:&DigitallySignedStruct)
        ->std::result::Result<HandshakeSignatureValid,rustls::Error>{Err(invalid_cert())}
    fn verify_tls13_signature(&self,m:&[u8],c:&CertificateDer<'_>,s:&DigitallySignedStruct)
        ->std::result::Result<HandshakeSignatureValid,rustls::Error>{tls13(m,c,s,&self.provider)}
    fn supported_verify_schemes(&self)->Vec<SignatureScheme>{vec![SignatureScheme::ML_DSA_65]}
}
/// Accepting this certificate establishes key possession only. The account
/// owner must separately authorize that key with the invitation and passkey.
#[derive(Debug)]
struct PossessionOnlyDevice{provider:Arc<CryptoProvider>}
impl ClientCertVerifier for PossessionOnlyDevice{
    fn root_hint_subjects(&self)->&[DistinguishedName]{&[]}
    fn client_auth_mandatory(&self)->bool{true}
    fn verify_client_cert(&self,end_entity:&CertificateDer<'_>,intermediates:&[CertificateDer<'_>],now:UnixTime)
        ->std::result::Result<ClientCertVerified,rustls::Error>{
        if !intermediates.is_empty(){return Err(invalid_cert());}
        certificate_key(end_entity)?;
        WebPkiClientVerifier::builder_with_provider(roots(end_entity)?,self.provider.clone())
            .build().map_err(|_|invalid_cert())?.verify_client_cert(end_entity,&[],now)
    }
    fn verify_tls12_signature(&self,_:&[u8],_:&CertificateDer<'_>,_:&DigitallySignedStruct)
        ->std::result::Result<HandshakeSignatureValid,rustls::Error>{Err(invalid_cert())}
    fn verify_tls13_signature(&self,m:&[u8],c:&CertificateDer<'_>,s:&DigitallySignedStruct)
        ->std::result::Result<HandshakeSignatureValid,rustls::Error>{tls13(m,c,s,&self.provider)}
    fn supported_verify_schemes(&self)->Vec<SignatureScheme>{vec![SignatureScheme::ML_DSA_65]}
}
#[derive(Debug)]
struct FixedIdentity(Arc<CertifiedKey>);
impl ResolvesClientCert for FixedIdentity{
    fn resolve(&self,_:&[&[u8]],schemes:&[SignatureScheme])->Option<Arc<CertifiedKey>>{
        schemes.contains(&SignatureScheme::ML_DSA_65).then(||self.0.clone())
    }
    fn has_certs(&self)->bool{true}
}
impl ResolvesServerCert for FixedIdentity{
    fn resolve(&self,hello:ClientHello<'_>)->Option<Arc<CertifiedKey>>{
        hello.signature_schemes().contains(&SignatureScheme::ML_DSA_65).then(||self.0.clone())
    }
}
pub(crate) fn client(identity:&DeviceIdentity,pin:[u8;32],service:&str)->Result<Arc<ClientConfig>>{
    let p=provider()?;
    let mut cfg=ClientConfig::builder_with_provider(p.clone())
        .with_protocol_versions(&[&rustls::version::TLS13]).map_err(|_|Error::Crypto)?
        .dangerous().with_custom_certificate_verifier(Arc::new(PinnedServer{pin,provider:p}))
        .with_client_cert_resolver(Arc::new(FixedIdentity(identity.certified_key())));
    cfg.alpn_protocols=vec![service.as_bytes().to_vec()];
    cfg.enable_early_data=false;
    cfg.resumption=rustls::client::Resumption::disabled();
    Ok(Arc::new(cfg))
}
pub(crate) fn server(identity:&DeviceIdentity,service:&str)->Result<Arc<ServerConfig>>{
    let p=provider()?;
    let mut cfg=ServerConfig::builder_with_provider(p.clone())
        .with_protocol_versions(&[&rustls::version::TLS13]).map_err(|_|Error::Crypto)?
        .with_client_cert_verifier(Arc::new(PossessionOnlyDevice{provider:p}))
        .with_cert_resolver(Arc::new(FixedIdentity(identity.certified_key())));
    cfg.alpn_protocols=vec![service.as_bytes().to_vec()];
    cfg.max_early_data_size=0;
    cfg.send_tls13_tickets=0;
    cfg.session_storage=Arc::new(rustls::server::NoServerSessionStorage{});
    Ok(Arc::new(cfg))
}
pub(crate) fn check_negotiated(state:&CommonState,service:&str)->Result<()>{
    if state.is_handshaking()
        || state.protocol_version()!=Some(ProtocolVersion::TLSv1_3)
        || state.alpn_protocol()!=Some(service.as_bytes())
        || state.negotiated_cipher_suite().map(|s|s.suite())!=Some(CipherSuite::TLS13_AES_256_GCM_SHA384)
        || state.negotiated_key_exchange_group().map(|g|g.name())!=Some(NamedGroup::X25519MLKEM768)
        || state.handshake_kind()==Some(rustls::HandshakeKind::Resumed) {
        return Err(Error::Authentication);
    }
    Ok(())
}
