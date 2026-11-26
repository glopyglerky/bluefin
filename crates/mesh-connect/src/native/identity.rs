use std::{fmt, sync::Arc};
use ml_dsa::{KeyInit, Keypair, MlDsa65, Signature, SigningKey, Signer as _, Verifier as _};
use rustls::{pki_types::{CertificateDer, SubjectPublicKeyInfoDer}, sign::{CertifiedKey, Signer as TlsSigner, SigningKey as TlsSigningKey}, SignatureAlgorithm, SignatureScheme};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};
use yasna::{models::{GeneralizedTime, ObjectIdentifier}, Tag};
use crate::{Error, Result, protocol::{ML_DSA_65_PUBLIC_KEY_BYTES, ML_DSA_65_SIGNATURE_BYTES}};

const ML_DSA_OID: &[u64] = &[2,16,840,1,101,3,4,3,18];

/// A persistent device key. The application must protect its seed with the
/// platform's existing secret store. Debug never prints seed or public key.
#[derive(Clone)]
pub struct DeviceIdentity {
    key: Arc<SigningKey<MlDsa65>>,
    public: Arc<Vec<u8>>,
    certificate: Arc<Vec<u8>>,
}
impl fmt::Debug for DeviceIdentity {
    fn fmt(&self,f:&mut fmt::Formatter<'_>)->fmt::Result{
        f.debug_struct("DeviceIdentity").finish_non_exhaustive()
    }
}
impl DeviceIdentity {
    pub fn generate()->Result<Self>{
        let mut seed=Zeroizing::new([0u8;32]);
        getrandom::fill(seed.as_mut()).map_err(|_|Error::Randomness)?;
        Self::from_seed(*seed)
    }
    pub fn from_seed(mut seed:[u8;32])->Result<Self>{
        if seed==[0;32]{return Err(Error::InvalidIdentity);}
        let key=SigningKey::<MlDsa65>::new_from_slice(&seed).map_err(|_|Error::InvalidIdentity)?;
        seed.zeroize();
        let public=key.verifying_key().encode().to_vec();
        let certificate=make_certificate(&key,&public)?;
        Ok(Self{key:Arc::new(key),public:Arc::new(public),certificate:Arc::new(certificate)})
    }
    /// Explicit secret export for the application's protected key store only.
    pub fn export_seed(&self)->Zeroizing<[u8;32]>{
        let mut seed=Zeroizing::new([0;32]);seed.copy_from_slice(self.key.as_seed());seed
    }
    pub fn public_key(&self)->Vec<u8>{self.public.as_ref().clone()}
    pub fn certificate_der(&self)->&[u8]{self.certificate.as_ref()}
    pub fn certificate_sha256(&self)->[u8;32]{Sha256::digest(self.certificate_der()).into()}
    pub fn sign(&self,message:&[u8])->Result<Vec<u8>>{
        if message.len()>crate::protocol::MAX_FRAME_BYTES{return Err(Error::TooLarge);}
        sign(&self.key,message)
    }
    pub(crate) fn iroh_secret(&self)->Result<iroh::SecretKey>{
        // RFC 5869 HKDF separates the outer addressing key from the PQ signing
        // key while preserving a stable endpoint for the protected server seed.
        let seed=self.export_seed();
        let hkdf=hkdf::Hkdf::<Sha256>::new(Some(b"mesh-connect/identity/v1"),seed.as_ref());
        let mut derived=Zeroizing::new([0u8;32]);
        hkdf.expand(b"iroh/ed25519",derived.as_mut()).map_err(|_|Error::Crypto)?;
        Ok(iroh::SecretKey::from_bytes(&derived))
    }
    pub(crate) fn certified_key(&self)->Arc<CertifiedKey>{
        Arc::new(CertifiedKey::new(
            vec![CertificateDer::from(self.certificate.as_ref().clone())],
            Arc::new(TlsKey{key:self.key.clone(),spki:spki(&self.public)})))
    }
}
/// Verify the owner's domain-separated device proof using ML-DSA-65.
pub fn verify_signature(public_key:&[u8],message:&[u8],signature:&[u8])->Result<()>{
    if public_key.len()!=ML_DSA_65_PUBLIC_KEY_BYTES || signature.len()!=ML_DSA_65_SIGNATURE_BYTES
        || message.len()>crate::protocol::MAX_FRAME_BYTES{return Err(Error::InvalidIdentity);}
    let key=ml_dsa::VerifyingKey::<MlDsa65>::new_from_slice(public_key).map_err(|_|Error::InvalidIdentity)?;
    let sig=Signature::<MlDsa65>::try_from(signature).map_err(|_|Error::Crypto)?;
    key.verify(message,&sig).map_err(|_|Error::Authentication)
}
fn sign(key:&SigningKey<MlDsa65>,message:&[u8])->Result<Vec<u8>>{
    let sig:Signature<MlDsa65>=key.try_sign(message).map_err(|_|Error::Crypto)?;
    Ok(sig.encode().to_vec())
}
#[derive(Clone)]
struct TlsKey{key:Arc<SigningKey<MlDsa65>>,spki:Vec<u8>}
impl fmt::Debug for TlsKey{
    fn fmt(&self,f:&mut fmt::Formatter<'_>)->fmt::Result{f.write_str("ML-DSA-65 signing key")}
}
impl TlsSigningKey for TlsKey{
    fn choose_scheme(&self,offered:&[SignatureScheme])->Option<Box<dyn TlsSigner>>{
        offered.contains(&SignatureScheme::ML_DSA_65).then(||Box::new(self.clone()) as Box<dyn TlsSigner>)
    }
    fn public_key(&self)->Option<SubjectPublicKeyInfoDer<'_>>{Some(SubjectPublicKeyInfoDer::from(self.spki.as_slice()))}
    fn algorithm(&self)->SignatureAlgorithm{SignatureAlgorithm::Unknown(0)}
}
impl TlsSigner for TlsKey{
    fn sign(&self,message:&[u8])->std::result::Result<Vec<u8>,rustls::Error>{
        sign(&self.key,message).map_err(|_|rustls::Error::General("ML-DSA signing failed".into()))
    }
    fn scheme(&self)->SignatureScheme{SignatureScheme::ML_DSA_65}
}
fn algorithm(w:yasna::DERWriter<'_>){
    w.write_sequence(|w|w.next().write_oid(&ObjectIdentifier::from_slice(ML_DSA_OID)));
}
fn spki(public:&[u8])->Vec<u8>{
    yasna::construct_der(|w|w.write_sequence(|w|{
        algorithm(w.next());w.next().write_bitvec_bytes(public,public.len()*8);
    }))
}
fn name(w:yasna::DERWriter<'_>){
    w.write_sequence(|w|w.next().write_set(|w|w.next().write_sequence(|w|{
        w.next().write_oid(&ObjectIdentifier::from_slice(&[2,5,4,3]));
        w.next().write_utf8_string("mesh-connect.invalid");
    })));
}
fn extension(w:yasna::DERWriter<'_>,oid:&[u64],critical:bool,value:&[u8]){
    w.write_sequence(|w|{
        w.next().write_oid(&ObjectIdentifier::from_slice(oid));
        if critical{w.next().write_bool(true);}
        w.next().write_bytes(value);
    });
}
/// RFC 5280 DER assembly only; all signing is the upstream FIPS 204 primitive.
/// Stable validity avoids changing a pinned identity when a device restarts.
/// Authorization and key revocation are always checked by the live account owner.
fn make_certificate(key:&SigningKey<MlDsa65>,public:&[u8])->Result<Vec<u8>>{
    let mut serial=Sha256::digest(public)[..20].to_vec();serial[0]&=0x7f;serial[0]|=1;
    let not_before=time::OffsetDateTime::from_unix_timestamp(1_735_689_600).map_err(|_|Error::Crypto)?;
    let not_after=time::OffsetDateTime::from_unix_timestamp(4_891_363_200).map_err(|_|Error::Crypto)?;
    let basic=yasna::construct_der(|w|w.write_sequence(|_|{}));
    let usage=yasna::construct_der(|w|w.write_bitvec_bytes(&[0x80],1));
    let eku=yasna::construct_der(|w|w.write_sequence(|w|{
        w.next().write_oid(&ObjectIdentifier::from_slice(&[1,3,6,1,5,5,7,3,1]));
        w.next().write_oid(&ObjectIdentifier::from_slice(&[1,3,6,1,5,5,7,3,2]));
    }));
    let san=yasna::construct_der(|w|w.write_sequence(|w|{
        w.next().write_tagged_implicit(Tag::context(2),|w|w.write_ia5_string("mesh-connect.invalid"));
    }));
    let tbs=yasna::construct_der(|w|w.write_sequence(|w|{
        w.next().write_tagged(Tag::context(0),|w|w.write_u8(2));
        w.next().write_bigint_bytes(&serial,true);
        algorithm(w.next());name(w.next());
        w.next().write_sequence(|w|{
            w.next().write_generalized_time(&GeneralizedTime::from_datetime(not_before));
            w.next().write_generalized_time(&GeneralizedTime::from_datetime(not_after));
        });
        name(w.next());w.next().write_der(&spki(public));
        w.next().write_tagged(Tag::context(3),|w|w.write_sequence(|w|{
            extension(w.next(),&[2,5,29,19],true,&basic);
            extension(w.next(),&[2,5,29,15],true,&usage);
            extension(w.next(),&[2,5,29,37],false,&eku);
            extension(w.next(),&[2,5,29,17],false,&san);
        }));
    }));
    let signature=sign(key,&tbs)?;
    Ok(yasna::construct_der(|w|w.write_sequence(|w|{
        w.next().write_der(&tbs);algorithm(w.next());
        w.next().write_bitvec_bytes(&signature,signature.len()*8);
    })))
}
