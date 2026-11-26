# Bluefin moonlit appearance

A focused visual refresh using deep navy, mist gray, moon silver, muted lavender, and smooth backgrounds. Media artwork and the established fin-and-wave identity are retained.

## Palette

| Role | Light | Dark |
| --- | --- | --- |
| Background | `#A3ACB1` | `#061727` |
| Raised panel | `#CCD0D3` | `#102435` |
| Hover | `#BDC5C9` | `#182D3D` |
| Text | `#061727` | `#CCD0D3` |
| Secondary text | `#2B4050` | `#A3ACB1` |
| Readable accent | `#333B5A` | `#ADB5D1` |
| Decorative lavender | `#8289A5` | `#8289A5` |
| Primary fill | `#061727` | `#8289A5` |
| Primary label | `#CCD0D3` | `#061727` |
| Control border | `#465966` | `#637889` |

The original lavender is decorative in light mode because its contrast against mist is too low for small labels. Native icon geometry remains the established curved fin above two waves. Web and Android marks now share those paths. Backgrounds are smooth. Glass is reserved for floating navigation and player controls; fields, artwork, lyrics, and QR modules remain clear. QR paper stays white with navy modules and lavender footer detailing.

## Client coverage

- Universal web and connection pages follow the system light/dark preference.
- Native Rust desktop registers both palettes without forcing a theme. This client uses flat colors without engraving.
- Apple main clients retain their existing appearance setting. Shared backgrounds, buttons, sign-in mark, and generated platform assets use the palette. iOS includes light and dark app-icon variants.
- Watch retains its existing dark presentation. Full-color widgets use the matching light/dark palette and preserve system rendering in other modes.
- Android and Android TV follow the system preference. Vector resources and native window colors have matching variants.
- Browser TV, Tizen, and webOS use light/dark CSS. Vega uses its existing React Native system-color hook.
- Roku retains its existing dark-only presentation.

## Initial palette verification on 2026-11-26

- Existing progressive web and packaging suites: 46 passed, zero failed.
- JavaScript syntax check and Git whitespace checks passed.
- All changed Swift sources parsed. Shared style and native mark typechecked at iOS 15 and tvOS 15.
- Watch background APIs typechecked at the project's watchOS 26 minimum. Widget views and supporting files typechecked at the project's iOS 26 minimum.
- Both native icon generators completed successfully. Original fin and wave coordinates were retained. Generated asset catalogs parsed successfully.
- Nine Android/Roku XML resources parsed. Material3 color parameters were checked against the pinned 1.3.2 source.
- Rust theme module passed rustfmt checking; the edited UI file parsed through rustfmt.

Full application builds, installed-device acceptance, and browser rendering are not qualified by these checks. Available local browser launchers had no installed browser binary. The native Rust compile was not run because the existing Cargo source cache target was unavailable. The supplied visual concept illustrates the intended appearance.

Source base: `45ead646a880996e89f6869c18724ac2e8e943b6`. This theme work is isolated on `work/bluefin-moonlit-theme-20261126`.

## Music and materials update

The subsequent [music playback and materials update](music-playback-and-materials.md) removes the repeating line texture, adopts public Liquid Glass controls on supported Apple systems, and adds media-clock lyric rendering. Its release qualification is separate from the initial palette checks above.
