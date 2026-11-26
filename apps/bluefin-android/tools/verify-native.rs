//! Check ELF PT_LOAD alignment for each supplied shared library before packaging.
use std::{env, fs};
fn number(bytes: &[u8], offset: usize, count: usize) -> Result<u64, &'static str> {
    let mut value = 0;
    for (i, byte) in bytes
        .get(offset..offset.checked_add(count).ok_or("overflow")?)
        .ok_or("truncated ELF")?
        .iter()
        .enumerate()
    {
        value |= u64::from(*byte) << (8 * i);
    }
    Ok(value)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths: Vec<_> = env::args().skip(1).collect();
    if paths.is_empty() {
        return Err("pass ELF shared libraries".into());
    }
    for path in paths {
        let b = fs::read(&path)?;
        if b.get(..4) != Some(b"\x7fELF") || b.get(5) != Some(&1) {
            return Err("expected little-endian ELF".into());
        }
        let wide = match b.get(4) {
            Some(1) => false,
            Some(2) => true,
            _ => return Err("invalid ELF class".into()),
        };
        let (phoff, entsize, count) = if wide {
            (number(&b, 32, 8)?, number(&b, 54, 2)?, number(&b, 56, 2)?)
        } else {
            (number(&b, 28, 4)?, number(&b, 42, 2)?, number(&b, 44, 2)?)
        };
        if entsize < if wide { 56 } else { 32 } {
            return Err("invalid program header size".into());
        }
        let mut loads = 0;
        for i in 0..count {
            let off = usize::try_from(
                phoff
                    .checked_add(i.checked_mul(entsize).ok_or("overflow")?)
                    .ok_or("overflow")?,
            )?;
            if number(&b, off, 4)? != 1 {
                continue;
            }
            loads += 1;
            let (file, virtual_address, alignment) = if wide {
                (
                    number(&b, off + 8, 8)?,
                    number(&b, off + 16, 8)?,
                    number(&b, off + 48, 8)?,
                )
            } else {
                (
                    number(&b, off + 4, 4)?,
                    number(&b, off + 8, 4)?,
                    number(&b, off + 28, 4)?,
                )
            };
            if alignment < 16384
                || !alignment.is_power_of_two()
                || file % 16384 != virtual_address % 16384
            {
                return Err(format!("{path}: PT_LOAD incompatible with 16 KiB pages").into());
            }
        }
        if loads == 0 {
            return Err("ELF has no load segments".into());
        }
        println!("{path}: {loads} PT_LOAD segments aligned for 16 KiB pages");
    }
    Ok(())
}
