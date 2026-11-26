#![cfg(feature="images")]
use std::io::Cursor;
use image::{DynamicImage,ImageFormat,Rgb,RgbImage};
use mesh_connect::{ConnectInvite,Error,QrTheme,decode_qr_image,decode_qr_image_at,qr_png,qr_svg};
fn invite()->ConnectInvite{
    let f:serde_json::Value=serde_json::from_str(include_str!("fixtures/invite-v1.json")).unwrap();
    serde_json::from_value(f["invite"].clone()).unwrap()
}
fn png(img:RgbImage)->Vec<u8>{
    let mut bytes=Cursor::new(Vec::new());DynamicImage::ImageRgb8(img).write_to(&mut bytes,ImageFormat::Png).unwrap();bytes.into_inner()
}
#[test]
fn bluefin_saved_image_jpeg_screenshot_rotation_and_crop_roundtrip(){
    let invite=invite();let theme=QrTheme::bluefin();
    let bytes=qr_png(&invite,&theme).unwrap();
    assert_eq!(bytes,qr_png(&invite,&theme).unwrap());
    assert_eq!(decode_qr_image(&bytes).unwrap(),invite);
    assert!(matches!(decode_qr_image_at(&bytes,invite.expires_at_unix_ms),Err(Error::Expired)));
    let original=image::load_from_memory(&bytes).unwrap().to_rgb8();
    let mut jpeg=Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(original.clone()).write_to(&mut jpeg,ImageFormat::Jpeg).unwrap();
    assert_eq!(decode_qr_image(jpeg.get_ref()).unwrap(),invite);
    let mut screen=RgbImage::from_pixel(original.width()+180,original.height()+250,Rgb([225,230,240]));
    image::imageops::replace(&mut screen,&original,70,100);
    for rotated in [screen.clone(),image::imageops::rotate90(&screen),image::imageops::rotate180(&screen),image::imageops::rotate270(&screen)]{
        assert_eq!(decode_qr_image(&png(rotated)).unwrap(),invite);
    }
    let cropped=image::imageops::crop_imm(&screen,45,70,original.width()+50,original.height()+60).to_image();
    assert_eq!(decode_qr_image(&png(cropped)).unwrap(),invite);
    let svg=qr_svg(&invite,&theme).unwrap();
    assert!(svg.contains("BLUEFIN"));assert!(svg.contains("shape-rendering=\"crispEdges\""));
    assert!(!svg.contains(&invite.to_code().unwrap()));assert!(!svg.contains("<script"));
}
#[test]
fn two_distinct_valid_codes_are_never_silently_selected(){
    let a=invite();let mut b=a.clone();b.invitation_id=[9;32];b.secret=[10;32];
    let one=image::load_from_memory(&qr_png(&a,&QrTheme::bluefin()).unwrap()).unwrap().to_rgb8();
    let two=image::load_from_memory(&qr_png(&b,&QrTheme::bluefin()).unwrap()).unwrap().to_rgb8();
    let mut screenshot=RgbImage::from_pixel(one.width()+two.width()+80,one.height().max(two.height())+40,Rgb([255,255,255]));
    image::imageops::replace(&mut screenshot,&one,20,20);
    image::imageops::replace(&mut screenshot,&two,i64::from(one.width()+60),20);
    assert!(matches!(decode_qr_image(&png(screenshot)),Err(Error::AmbiguousImage)));
}
#[test]
fn image_and_theme_resource_bounds_apply_before_qr_detection(){
    let invite=invite();let mut theme=QrTheme::bluefin();theme.foreground=[220,220,220];
    assert!(matches!(qr_png(&invite,&theme),Err(Error::InvalidTheme)));
    theme=QrTheme::bluefin();theme.quiet_zone_modules=3;assert!(qr_svg(&invite,&theme).is_err());
    assert!(matches!(decode_qr_image(&vec![0;8*1024*1024+1]),Err(Error::TooLarge)));
    let oversized=png(RgbImage::from_pixel(4097,1,Rgb([255,255,255])));
    assert!(matches!(decode_qr_image(&oversized),Err(Error::TooLarge)));
    assert!(decode_qr_image(b"<svg/>").is_err());
}
