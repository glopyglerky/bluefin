use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::{
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use url::{Host, Url};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct CountryCode(String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountryCodeError;

impl fmt::Display for CountryCodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("country must contain exactly two ASCII letters")
    }
}
#[cfg(feature = "http")]
impl std::error::Error for CountryCodeError {}

impl CountryCode {
    /// Validates the ISO-style syntax. Source registries determine actual coverage.
    pub fn parse(value: &str) -> Result<Self, CountryCodeError> {
        if value.len() != 2 || !value.bytes().all(|b| b.is_ascii_alphabetic()) {
            return Err(CountryCodeError);
        }
        Ok(Self(value.to_ascii_uppercase()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn lower(&self) -> String {
        self.0.to_ascii_lowercase()
    }
}
impl fmt::Display for CountryCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl<'de> Deserialize<'de> for CountryCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
    Movie,
    Series,
    Season,
    Episode,
    Artist,
    Release,
    ReleaseGroup,
    Recording,
    Work,
    MusicVideo,
    Person,
    League,
    Team,
    SportsEvent,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogId {
    pub source: String,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogIdentity {
    pub kind: MediaKind,
    pub ids: Vec<CatalogId>,
}

impl CatalogIdentity {
    pub fn id(&self, source: &str) -> Option<&str> {
        self.ids
            .iter()
            .find(|id| id.source.eq_ignore_ascii_case(source))
            .map(|id| id.id.as_str())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupRequest {
    pub identity: CatalogIdentity,
    /// None requests all regions reported by the source, never an implicit US default.
    #[serde(default)]
    pub country: Option<CountryCode>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccessType {
    Subscription,
    Rent,
    Buy,
    Free,
    Ads,
    Addon,
    Download,
    Other,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DestinationKind {
    Title,
    Player,
    Download,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Destination {
    pub kind: DestinationKind,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceIdentity {
    pub source: String,
    pub id: String,
    pub name: String,
    pub home_url: Option<String>,
    pub logo_url: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Price {
    pub amount: Option<String>,
    pub currency: Option<String>,
    pub formatted: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferEvidence {
    pub source: String,
    pub observed_at: u64,
    pub available_since: Option<u64>,
    pub expires_at: Option<u64>,
    pub expires_soon: bool,
    pub region_verified: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessOffer {
    pub id: String,
    pub identity: CatalogIdentity,
    /// Unknown means the evidence does not establish a licensed country.
    pub country: Option<CountryCode>,
    pub service: ServiceIdentity,
    pub access_type: AccessType,
    pub raw_access_type: Option<String>,
    pub addon: Option<ServiceIdentity>,
    pub destinations: Vec<Destination>,
    pub price: Option<Price>,
    pub quality: Option<String>,
    pub evidence: OfferEvidence,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceStatus {
    Complete,
    Partial,
    NoOffers,
    MissingCredentials,
    Denied,
    RateLimited,
    Unavailable,
    NotFound,
    IdentityMismatch,
    InvalidResponse,
    UnsupportedIdentity,
    UnsupportedCountry,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverageScope {
    RegionalAvailability,
    LinkedAccess,
    CatalogMetadata,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Coverage {
    SourceCatalog,
    Partial,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attribution {
    pub name: String,
    pub url: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceObservation {
    pub source: String,
    pub status: SourceStatus,
    pub observed_at: u64,
    pub scope: CoverageScope,
    pub coverage: Coverage,
    /// Source catalog page, never represented as an exact provider destination.
    pub catalog_url: Option<String>,
    pub attributions: Vec<Attribution>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferResponse {
    pub schema_version: u32,
    pub identity: CatalogIdentity,
    pub country: Option<CountryCode>,
    pub offers: Vec<AccessOffer>,
    pub sources: Vec<SourceObservation>,
}
impl OfferResponse {
    pub fn empty(request: &LookupRequest) -> Self {
        Self {
            schema_version: 1,
            identity: request.identity.clone(),
            country: request.country.clone(),
            offers: Vec::new(),
            sources: Vec::new(),
        }
    }
    pub fn with_status(
        request: &LookupRequest,
        source: &str,
        status: SourceStatus,
        scope: CoverageScope,
        observed_at: u64,
    ) -> Self {
        let mut result = Self::empty(request);
        result.sources.push(SourceObservation {
            source: source.to_string(),
            status,
            observed_at,
            scope,
            coverage: Coverage::Unknown,
            catalog_url: None,
            attributions: Vec::new(),
        });
        result
    }
    /// Joins evidence for exactly the same request. Mismatched results cannot leak across titles.
    pub fn merge(&mut self, other: Self) -> bool {
        if self.identity != other.identity || self.country != other.country {
            return false;
        }
        self.offers.extend(other.offers);
        self.sources.extend(other.sources);
        self.offers.sort_by(|a, b| a.id.cmp(&b.id));
        self.offers.dedup_by(|a, b| a.id == b.id);
        true
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryService {
    pub service: ServiceIdentity,
    pub access_types: Vec<AccessType>,
    pub addons: Vec<ServiceIdentity>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CountryRecord {
    pub code: CountryCode,
    pub name: String,
    pub services: Vec<RegistryService>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryResponse {
    pub schema_version: u32,
    pub countries: Vec<CountryRecord>,
    pub sources: Vec<SourceObservation>,
}
impl Default for RegistryResponse {
    fn default() -> Self {
        Self {
            schema_version: 1,
            countries: Vec::new(),
            sources: Vec::new(),
        }
    }
}
impl RegistryResponse {
    pub fn merge(&mut self, other: Self) {
        for mut country in other.countries {
            if let Some(existing) = self.countries.iter_mut().find(|c| c.code == country.code) {
                existing.services.append(&mut country.services);
                existing.services.sort_by(|a, b| {
                    (&a.service.source, &a.service.id).cmp(&(&b.service.source, &b.service.id))
                });
                existing.services.dedup_by(|a, b| {
                    a.service.source == b.service.source && a.service.id == b.service.id
                });
            } else {
                self.countries.push(country);
            }
        }
        self.countries.sort_by(|a, b| a.code.cmp(&b.code));
        self.sources.extend(other.sources);
    }
}

/// Public HTTPS links supplied by catalog evidence. This helper never makes network requests.
/// New public providers work without a name/domain allowlist. Credentials and local targets do not.
pub fn safe_https_url(value: &str) -> Option<String> {
    if value.is_empty()
        || value.len() > 8192
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return None;
    }
    let parsed = Url::parse(value).ok()?;
    if parsed.scheme() != "https" || !parsed.username().is_empty() || parsed.password().is_some() {
        return None;
    }
    if parsed.port().is_some_and(|port| port != 443) {
        return None;
    }
    match parsed.host()? {
        Host::Ipv4(ip) if !public_v4(ip) => return None,
        Host::Ipv6(ip) if !public_v6(ip) => return None,
        Host::Domain(host) => {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            if !host.contains('.')
                || ["localhost", "local", "internal", "home", "lan", "invalid"]
                    .iter()
                    .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
            {
                return None;
            }
            // Url normalizes most numeric host forms; parse any remaining literal defensively.
            if let Ok(ip) = host.parse::<IpAddr>() {
                match ip {
                    IpAddr::V4(ip) if !public_v4(ip) => return None,
                    IpAddr::V6(ip) if !public_v6(ip) => return None,
                    _ => {}
                }
            }
        }
        _ => {}
    }
    Some(parsed.to_string())
}
fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
        || (a == 192 && b == 0 && (c == 0 || c == 2))
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113))
}
fn public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return public_v4(v4);
    }
    let octets = ip.octets();
    (octets[0] & 0xe0) == 0x20
        && !ip.is_unspecified()
        && !ip.is_loopback()
        && (octets[0] & 0xfe) != 0xfc
        && (octets[0] != 0xfe || (octets[1] & 0xc0) != 0x80)
        && octets[0] != 0xff
        && !(octets[0] == 0x20 && octets[1] == 0x01 && octets[2] == 0x0d && octets[3] == 0xb8)
}

/// Stable evidence identity, independent of source ordering, freshness and volatile price.
pub fn offer_id(offer: &AccessOffer) -> String {
    let mut ids = offer.identity.ids.clone();
    ids.sort();
    let mut destinations: Vec<_> = offer
        .destinations
        .iter()
        .map(|d| (&d.kind, &d.url))
        .collect();
    destinations.sort();
    let key = (
        offer.identity.kind,
        ids,
        &offer.country,
        &offer.service.source,
        &offer.service.id,
        offer.access_type,
        &offer.raw_access_type,
        offer.addon.as_ref().map(|a| (&a.source, &a.id)),
        &offer.quality,
        destinations,
    );
    let bytes = serde_json::to_vec(&key).unwrap_or_default();
    let digest = Sha256::digest(bytes);
    let mut id = String::from("offer:");
    for byte in digest {
        id.push_str(&format!("{byte:02x}"));
    }
    id
}
