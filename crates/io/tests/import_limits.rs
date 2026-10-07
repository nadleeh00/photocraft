//! `import_with_limits`: every decoder path runs under the caller's limits.

use photocraft_codecs::{ChannelLayout, CodecError, EncodeOptions, Format, Image, Limits};
use photocraft_io::{IoError, import, import_with_limits};
use photocraft_psd::{LayerSpec, PixelData, PsdBuilder, PsdError, PsdFile};
use photocraft_raw::RawError;
use photocraft_raw::testgen::{DngSpec, TiffBuilder, Val, mosaic, scene};

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = Image::from_u8(w, h, ChannelLayout::Rgb, vec![90; (w * h * 3) as usize]).unwrap();
    photocraft_codecs::encode(&img, Format::Png, &EncodeOptions::default()).unwrap()
}

fn psd(w: u32, h: u32, layers: usize) -> Vec<u8> {
    let mut b = PsdBuilder::new(w, h);
    for i in 0..layers {
        b.push_layer(LayerSpec::new(format!("L{i}"), 0, 0, w, h, PixelData::Rgba8(vec![200; (w * h * 4) as usize])));
    }
    b.composite(PixelData::Rgba8(vec![200; (w * h * 4) as usize]));
    b.to_bytes().unwrap()
}

fn limits(max_width: u32, max_height: u32, max_pixels: u64, max_alloc: u64) -> Limits {
    Limits { max_width, max_height, max_pixels, max_alloc }
}

fn wide() -> Limits {
    Limits::default()
}

#[test]
fn flat_decode_takes_the_limits() {
    let bytes = png(50, 10);
    assert!(import_with_limits("a.png", &bytes, &wide()).is_ok());
    for l in [limits(49, 1000, 1 << 20, 1 << 30), limits(1000, 9, 1 << 20, 1 << 30), limits(1000, 1000, 499, 1 << 30)] {
        let err = import_with_limits("a.png", &bytes, &l).unwrap_err();
        assert!(matches!(err, IoError::Codec(CodecError::LimitExceeded(_))), "{l:?}: {err:?}");
    }
}

#[test]
fn psd_canvas_over_the_limits_is_refused() {
    let bytes = psd(40, 30, 1);
    for l in [limits(39, 1000, 1 << 20, 1 << 30), limits(1000, 29, 1 << 20, 1 << 30), limits(1000, 1000, 1199, 1 << 30)] {
        let err = import_with_limits("a.psd", &bytes, &l).unwrap_err();
        assert!(matches!(err, IoError::Psd(PsdError::LimitExceeded(m)) if m.contains("canvas")), "{l:?}: {err:?}");
    }
}

#[test]
fn psd_layer_data_over_max_alloc_is_refused_before_decoding() {
    let bytes = psd(40, 30, 3);
    let need = PsdFile::from_bytes(&bytes).unwrap().decoded_bytes().unwrap();
    let err = import_with_limits("a.psd", &bytes, &limits(1000, 1000, 1 << 20, need - 1)).unwrap_err();
    assert!(matches!(err, IoError::Psd(PsdError::LimitExceeded(m)) if m.contains("decoded layer data")), "{err:?}");
    let ok = import_with_limits("a.psd", &bytes, &limits(1000, 1000, 1 << 20, need)).unwrap();
    assert_eq!(ok.document.layers.len(), import("a.psd", &bytes).unwrap().document.layers.len());
}

#[test]
fn raw_develop_takes_the_limits() {
    let (w, h) = (40, 24);
    let bytes = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 65535)).build();
    assert!(import_with_limits("shot.dng", &bytes, &wide()).is_ok());
    let err = import_with_limits("shot.dng", &bytes, &limits(39, 1000, 1 << 20, 1 << 30)).unwrap_err();
    assert!(matches!(err, IoError::Raw(RawError::LimitExceeded(_))), "{err:?}");
}

/// A raw variant that is not developed falls back to its embedded JPEG preview; that decode
/// must run under the same limits, not the defaults.
#[test]
fn raw_preview_fallback_takes_the_limits() {
    let img = Image::from_u8(32, 20, ChannelLayout::Rgb, vec![180; 32 * 20 * 3]).unwrap();
    let jpeg = photocraft_codecs::encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
    let mut t = TiffBuilder::default();
    let strip = t.blob(vec![0; 64]);
    let preview = t.blob(jpeg.clone());
    let raw = t.ifd(vec![
        (256, Val::Long(vec![8])),
        (257, Val::Long(vec![8])),
        (258, Val::Short(vec![12])),
        (259, Val::Short(vec![34713])),
        (262, Val::Short(vec![32803])),
        (273, Val::Blobs(vec![strip])),
        (279, Val::Long(vec![64])),
        (33421, Val::Short(vec![2, 2])),
        (33422, Val::Byte(vec![0, 1, 1, 2])),
    ]);
    let ifd0 = t.ifd(vec![
        (271, Val::Ascii("NIKON CORPORATION".into())),
        (330, Val::Ifds(vec![raw])),
        (513, Val::Blobs(vec![preview])),
        (514, Val::Long(vec![jpeg.len() as u32])),
    ]);
    t.chain = vec![ifd0];
    let bytes = t.build();
    let opened = import_with_limits("DSC_0001.NEF", &bytes, &wide()).unwrap();
    assert_eq!((opened.document.size.width, opened.document.size.height), (32, 20));
    let err = import_with_limits("DSC_0001.NEF", &bytes, &limits(31, 1000, 1 << 20, 1 << 30)).unwrap_err();
    assert!(matches!(err, IoError::Codec(CodecError::LimitExceeded(_))), "{err:?}");
}

#[test]
fn native_bundles_are_refused() {
    let err = import_with_limits("doc.pcraft", b"anything", &wide()).unwrap_err();
    assert!(matches!(err, IoError::Unsupported(_)), "{err:?}");
}

#[test]
fn under_wide_limits_flat_and_psd_open_like_import() {
    for (name, bytes) in [("a.png", png(8, 6)), ("a.psd", psd(8, 6, 2))] {
        let a = import(name, &bytes).unwrap().document;
        let b = import_with_limits(name, &bytes, &wide()).unwrap().document;
        assert_eq!((a.size, a.layers.len(), a.mode, a.depth), (b.size, b.layers.len(), b.mode, b.depth), "{name}");
    }
}
