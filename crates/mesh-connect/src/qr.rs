use std::io::Cursor;
use image::{ImageEncoder, ImageFormat, ImageReader, Rgb, RgbImage};
use qrcode::{Color, EcLevel, QrCode};
use crate::{ConnectInvite, Error, Result};

const MAX_IMAGE_INPUT: usize = 8 * 1024 * 1024;
const MAX_IMAGE_SIDE: u32 = 4096;
const MAX_IMAGE_PIXELS: u64 = 8 * 1024 * 1024;
const MAX_GRIDS: usize = 32;

/// Branding stays outside the four-module quiet zone and never replaces data.
#[derive(Clone, Debug)]
pub struct QrTheme {
    pub foreground: [u8;3],
    pub background: [u8;3],
    pub accent: [u8;3],
    pub module_pixels: u32,
    pub quiet_zone_modules: u32,
    pub bluefin_branding: bool,
}
impl QrTheme {
    pub fn bluefin() -> Self {
        Self { foreground:[8,36,64], background:[255,255,255],
            accent:[0,144,168], module_pixels:6, quiet_zone_modules:4,
            bluefin_branding:true }
    }
    fn validate(&self) -> Result<()> {
        let light=luminance(self.background);
        let dark=luminance(self.foreground);
        if self.quiet_zone_modules<4 || self.quiet_zone_modules>16
            || !(3..=12).contains(&self.module_pixels)
            || light<0.85 || dark>0.10 || (light+0.05)/(dark+0.05)<7.0 {
            return Err(Error::InvalidTheme);
        }
        Ok(())
    }
}
impl Default for QrTheme { fn default()->Self {Self::bluefin()} }
fn luminance(color:[u8;3])->f64 {
    let linear=|v:u8|{let s=f64::from(v)/255.0;if s<=0.04045{s/12.92}else{((s+0.055)/1.055).powf(2.4)}};
    0.2126*linear(color[0])+0.7152*linear(color[1])+0.0722*linear(color[2])
}
fn code(invite:&ConnectInvite)->Result<QrCode> {
    // Ungrouped ASCII gives QR alphanumeric mode more room. It is the identical
    // canonical capsule, checksum and capability as the human grouped code.
    let text=invite.to_code()?.replace('-',"");
    QrCode::with_error_correction_level(text.as_bytes(),EcLevel::H)
        .map_err(|_|Error::TooLarge)
}
fn hex(c:[u8;3])->String{format!("#{:02x}{:02x}{:02x}",c[0],c[1],c[2])}
/// Deterministic SVG without external resources, embedded scripts or secrets
/// in metadata. The code itself is secret-bearing and must be handled as such.
pub fn qr_svg(invite:&ConnectInvite,theme:&QrTheme)->Result<String>{
    theme.validate()?;let qr=code(invite)?;
    let q=theme.quiet_zone_modules as usize;
    let side=qr.width()+2*q;
    let footer=if theme.bluefin_branding {12}else{0};
    let mut svg=format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {side} {}\" role=\"img\" aria-label=\"Bluefin one-time connection code\"><rect width=\"100%\" height=\"100%\" fill=\"{}\"/><path fill=\"{}\" shape-rendering=\"crispEdges\" d=\"",
        side*theme.module_pixels as usize,(side+footer)*theme.module_pixels as usize,side+footer,
        hex(theme.background),hex(theme.foreground));
    for y in 0..qr.width(){for x in 0..qr.width(){
        if qr[(x,y)]==Color::Dark{svg.push_str(&format!("M{} {}h1v1h-1z",x+q,y+q));}
    }}
    svg.push_str("\"/>");
    if theme.bluefin_branding {
        // Footer begins after the complete bottom quiet zone.
        let y=side+2;
        svg.push_str(&format!("<path fill=\"{}\" d=\"M8 {}l10-7-3 7 3 3z\"/><text x=\"23\" y=\"{}\" fill=\"{}\" font-family=\"sans-serif\" font-size=\"5\" font-weight=\"700\">BLUEFIN</text>",
            hex(theme.accent),y+6,y+6,hex(theme.foreground)));
    }
    svg.push_str("</svg>");Ok(svg)
}
/// Lossless PNG, including a small fin below the full quiet zone.
pub fn qr_png(invite:&ConnectInvite,theme:&QrTheme)->Result<Vec<u8>>{
    theme.validate()?;let qr=code(invite)?;
    let q=theme.quiet_zone_modules;
    let scale=theme.module_pixels;
    let side=(qr.width() as u32+2*q)*scale;
    let footer=if theme.bluefin_branding{12*scale}else{0};
    let mut img=RgbImage::from_pixel(side,side+footer,Rgb(theme.background));
    for y in 0..qr.width(){for x in 0..qr.width(){
        if qr[(x,y)]==Color::Dark{
            let x0=(x as u32+q)*scale;let y0=(y as u32+q)*scale;
            for dy in 0..scale{for dx in 0..scale{img.put_pixel(x0+dx,y0+dy,Rgb(theme.foreground));}}
        }
    }}
    if theme.bluefin_branding {
        // A solid triangular fin; all branding pixels lie below the QR canvas.
        for dy in 0..7*scale {
            let width=dy*10/7;
            for dx in 0..=width {
                let x=18*scale-dx;let y=side+scale+dy;
                if x<side {img.put_pixel(x,y,Rgb(theme.accent));}
            }
        }
    }
    let mut bytes=Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(img.as_raw(),img.width(),img.height(),image::ExtendedColorType::Rgb8)
        .map_err(|_|Error::InvalidImage)?;
    Ok(bytes)
}
/// Decode saved PNG/JPEG images and screenshots locally. Every detected valid
/// capsule must agree, so a screenshot cannot silently choose a different
/// account/server. Invalid/expired imports never require an external upload.
pub fn decode_qr_image(input:&[u8])->Result<ConnectInvite>{
    if input.is_empty() || input.len()>MAX_IMAGE_INPUT{return Err(Error::TooLarge);}
    let format=image::guess_format(input).map_err(|_|Error::InvalidImage)?;
    if !matches!(format,ImageFormat::Png|ImageFormat::Jpeg){return Err(Error::InvalidImage);}
    let (width,height)=ImageReader::with_format(Cursor::new(input),format)
        .into_dimensions().map_err(|_|Error::InvalidImage)?;
    if width==0 || height==0 || width>MAX_IMAGE_SIDE || height>MAX_IMAGE_SIDE
        || u64::from(width)*u64::from(height)>MAX_IMAGE_PIXELS{return Err(Error::TooLarge);}
    let mut limits=image::Limits::default();
    limits.max_image_width=Some(MAX_IMAGE_SIDE);
    limits.max_image_height=Some(MAX_IMAGE_SIDE);
    limits.max_alloc=Some(64*1024*1024);
    let mut reader=ImageReader::with_format(Cursor::new(input),format);
    reader.limits(limits);
    let gray=reader.decode().map_err(|_|Error::InvalidImage)?.to_luma8();
    let mut prepared=rqrr::PreparedImage::prepare_from_greyscale(
        width as usize,height as usize,|x,y|gray.get_pixel(x as u32,y as u32).0[0]);
    let grids=prepared.detect_grids();
    if grids.len()>MAX_GRIDS{return Err(Error::TooLarge);}
    let mut found:Option<ConnectInvite>=None;
    for grid in grids{
        let Ok((_,text))=grid.decode() else{continue};
        let invite = match ConnectInvite::parse(&text) {
            Ok(invite) => invite,
            Err(error) if looks_like_invitation(&text) => return Err(error),
            Err(_) => continue,
        };
        if let Some(old)=&found {
            if old!=&invite{return Err(Error::AmbiguousImage);}
        }else{found=Some(invite);}
    }
    found.ok_or(Error::NoCode)
}
pub fn decode_qr_image_at(input:&[u8],now_unix_ms:u64)->Result<ConnectInvite>{
    let invite=decode_qr_image(input)?;invite.validate_at(now_unix_ms)?;Ok(invite)
}

fn looks_like_invitation(text: &str) -> bool {
    let text = text.trim_ascii().to_ascii_lowercase();
    text.starts_with("mesh-connect://invite/") || text.starts_with("bluefin://connect/")
        || (text.starts_with("mc") && text.as_bytes().get(2).is_some_and(u8::is_ascii_digit))
}
