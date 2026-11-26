//! Narrow TensorBundle v2 reader for the single, hash-pinned publisher archive.
//! No TensorFlow runtime, pickle, checkpoint execution, filesystem extraction or downloads.
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Read};

pub const ARCHIVE_SHA256: &str = "f3a90b39dd2874269e8b05a48a86745df897b848c61f3958efc80a39152bd692";
pub const RELEASE_COMMIT: &str = "556ef2121492d72398a988af1b38b55176f5973a";
pub const MAX_ARCHIVE_BYTES: usize = 80 * 1024 * 1024;

pub type Result<T> = std::result::Result<T, crate::Error>;
fn invalid() -> crate::Error {
    crate::Error::InvalidModel
}
pub fn digest(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}

#[derive(Debug)]
pub struct Weight {
    pub shape: Vec<usize>,
    pub values: Vec<f32>,
}
pub struct Checkpoint {
    pub weights: BTreeMap<String, Weight>,
    #[cfg(feature = "qualification")]
    pub member_hashes: BTreeMap<String, String>,
    /// Retained only by qualification; the product never interprets this executable graph.
    #[cfg(feature = "qualification")]
    pub meta: Vec<u8>,
}

impl Checkpoint {
    pub fn from_official_archive(bytes: &[u8], cancel: &crate::Cancellation) -> Result<Self> {
        cancel.check()?;
        if bytes.len() > MAX_ARCHIVE_BYTES || digest(bytes) != ARCHIVE_SHA256 {
            return Err(invalid());
        }
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
        let mut members = BTreeMap::new();
        #[cfg(feature = "qualification")]
        let mut member_hashes = BTreeMap::new();
        let mut total = 0usize;
        for entry in archive.entries()? {
            cancel.check()?;
            let entry = entry?;
            let name = entry.path()?.to_str().ok_or_else(invalid)?.to_owned();
            if ![
                "checkpoint",
                "model.index",
                "model.meta",
                "model.data-00000-of-00001",
                "./._checkpoint",
            ]
            .contains(&name.as_str())
                || !entry.header().entry_type().is_file()
                || entry.size() > 256 * 1024 * 1024
                || members.contains_key(&name)
            {
                return Err(invalid());
            }
            let mut value = Vec::new();
            let mut limited = entry.take(256 * 1024 * 1024 + 1);
            let mut buffer = [0; 65536];
            loop {
                cancel.check()?;
                let n = limited.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                total += n;
                if total > 320 * 1024 * 1024 {
                    return Err(invalid());
                }
                value.extend_from_slice(&buffer[..n]);
            }
            #[cfg(feature = "qualification")]
            member_hashes.insert(name.clone(), digest(&value));
            members.insert(name, value);
        }
        let index = members.remove("model.index").ok_or_else(invalid)?;
        let data = members
            .remove("model.data-00000-of-00001")
            .ok_or_else(invalid)?;
        #[cfg(feature = "qualification")]
        let meta = members.remove("model.meta").ok_or_else(invalid)?;
        let entries = table_entries(&index)?;
        let mut weights = BTreeMap::new();
        for (name, value) in entries {
            cancel.check()?;
            if name.is_empty() {
                continue;
            } // BundleHeaderProto
            let name = String::from_utf8(name).map_err(|_| invalid())?;
            let fields = fields(&value)?;
            let dtype = scalar(&fields, 1).unwrap_or(0);
            if dtype != 1 {
                continue;
            } // training global_step is INT64
            let shape = bytes_field(&fields, 2).ok_or_else(invalid)?;
            let mut dims = Vec::new();
            for field in fields_fn(shape)? {
                if field.0 == 2 {
                    let dim = fields_fn(field.2)?;
                    let size = scalar(&dim, 1).unwrap_or(0);
                    dims.push(usize::try_from(size).map_err(|_| invalid())?);
                }
            }
            if dims.len() > 4 {
                return Err(invalid());
            }
            if scalar(&fields, 3).unwrap_or(0) != 0 {
                return Err(invalid());
            }
            let offset = usize::try_from(scalar(&fields, 4).unwrap_or(0)).map_err(|_| invalid())?;
            let size = usize::try_from(scalar(&fields, 5).unwrap_or(0)).map_err(|_| invalid())?;
            let count = dims
                .iter()
                .try_fold(1usize, |a, b| a.checked_mul(*b))
                .ok_or_else(invalid)?;
            if count.checked_mul(4) != Some(size) || size > 64 * 1024 * 1024 {
                return Err(invalid());
            }
            let raw = data
                .get(offset..offset.checked_add(size).ok_or_else(invalid)?)
                .ok_or_else(invalid)?;
            let values: Vec<f32> = raw
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            if values.iter().any(|v| !v.is_finite()) {
                return Err(invalid());
            }
            weights.insert(
                name,
                Weight {
                    shape: dims,
                    values,
                },
            );
        }
        Ok(Self {
            weights,
            #[cfg(feature = "qualification")]
            member_hashes,
            #[cfg(feature = "qualification")]
            meta,
        })
    }
}

fn varint(b: &[u8], p: &mut usize) -> Result<u64> {
    let mut n = 0u64;
    for shift in (0..70).step_by(7) {
        let v = *b.get(*p).ok_or_else(invalid)?;
        *p += 1;
        if shift == 63 && v > 1 {
            return Err(invalid());
        }
        n |= u64::from(v & 127) << shift;
        if v < 128 {
            return Ok(n);
        }
    }
    Err(invalid())
}
type Field<'a> = (u64, u64, &'a [u8]);
fn fields_fn(b: &[u8]) -> Result<Vec<Field<'_>>> {
    fields(b)
}
fn fields(b: &[u8]) -> Result<Vec<Field<'_>>> {
    let mut p = 0;
    let mut result = Vec::new();
    while p < b.len() {
        let key = varint(b, &mut p)?;
        let start = p;
        let value = match key & 7 {
            0 => varint(b, &mut p)?,
            1 => {
                p = p.checked_add(8).ok_or_else(invalid)?;
                0
            }
            2 => {
                let size = usize::try_from(varint(b, &mut p)?).map_err(|_| invalid())?;
                let start = p;
                p = p.checked_add(size).ok_or_else(invalid)?;
                result.push((key >> 3, 0, b.get(start..p).ok_or_else(invalid)?));
                continue;
            }
            5 => {
                p = p.checked_add(4).ok_or_else(invalid)?;
                0
            }
            _ => return Err(invalid()),
        };
        result.push((key >> 3, value, b.get(start..p).ok_or_else(invalid)?));
    }
    Ok(result)
}
fn scalar(f: &[Field<'_>], key: u64) -> Option<u64> {
    f.iter().find(|v| v.0 == key).map(|v| v.1)
}
fn bytes_field<'a>(f: &[Field<'a>], key: u64) -> Option<&'a [u8]> {
    f.iter().find(|v| v.0 == key).map(|v| v.2)
}

fn block(file: &[u8], offset: u64, size: u64) -> Result<&[u8]> {
    let offset = usize::try_from(offset).map_err(|_| invalid())?;
    let size = usize::try_from(size).map_err(|_| invalid())?;
    let end = offset.checked_add(size).ok_or_else(invalid)?;
    let contents = file.get(offset..end).ok_or_else(invalid)?;
    let trailer = file.get(end..end + 5).ok_or_else(invalid)?;
    if trailer[0] != 0 {
        return Err(invalid());
    } // publisher uses uncompressed blocks
    let crc = crc32c::crc32c_append(crc32c::crc32c(contents), &trailer[..1]);
    let masked = crc.rotate_right(15).wrapping_add(0xa282ead8);
    if masked != u32::from_le_bytes(trailer[1..].try_into().unwrap()) {
        return Err(invalid());
    }
    Ok(contents)
}
fn entries(b: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let count = u32::from_le_bytes(
        b.get(b.len().checked_sub(4).ok_or_else(invalid)?..)
            .ok_or_else(invalid)?
            .try_into()
            .unwrap(),
    ) as usize;
    let end = b
        .len()
        .checked_sub((count + 1).checked_mul(4).ok_or_else(invalid)?)
        .ok_or_else(invalid)?;
    let mut p = 0;
    let mut previous = Vec::new();
    let mut result = Vec::new();
    while p < end {
        let shared = varint(b, &mut p)? as usize;
        let unshared = varint(b, &mut p)? as usize;
        let size = varint(b, &mut p)? as usize;
        if shared > previous.len() {
            return Err(invalid());
        }
        let key_end = p.checked_add(unshared).ok_or_else(invalid)?;
        let mut key = previous[..shared].to_vec();
        key.extend_from_slice(b.get(p..key_end).ok_or_else(invalid)?);
        p = key_end;
        let value_end = p.checked_add(size).ok_or_else(invalid)?;
        if value_end > end {
            return Err(invalid());
        }
        let value = b.get(p..value_end).ok_or_else(invalid)?.to_vec();
        p = value_end;
        previous = key.clone();
        result.push((key, value));
    }
    Ok(result)
}
fn table_entries(file: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    if file.len() < 48 || file[file.len() - 8..] != 0xdb4775248b80fb57u64.to_le_bytes() {
        return Err(invalid());
    }
    let footer = &file[file.len() - 48..file.len() - 8];
    let mut p = 0;
    let _meta_offset = varint(footer, &mut p)?;
    let _meta_size = varint(footer, &mut p)?;
    let offset = varint(footer, &mut p)?;
    let size = varint(footer, &mut p)?;
    let mut result = Vec::new();
    for (_, handle) in entries(block(file, offset, size)?)? {
        let mut p = 0;
        let offset = varint(&handle, &mut p)?;
        let size = varint(&handle, &mut p)?;
        result.extend(entries(block(file, offset, size)?)?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_bad_table_and_varints() {
        assert!(table_entries(&[]).is_err());
        assert!(varint(&[255; 11], &mut 0).is_err());
        assert!(fields(&[10, 255]).is_err());
    }
}
