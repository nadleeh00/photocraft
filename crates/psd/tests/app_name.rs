//! `set_app_name`: a per-process global, so this file holds one test (its own process).

use photocraft_psd::resources::{app_name, ids, set_app_name, version_info_resource};
use photocraft_psd::{LayerSpec, PixelData, PsdBuilder, PsdFile, ResourceData};

fn names(file: &PsdFile) -> (String, String) {
    let r = file.resources.iter().find(|r| r.id == ids::VERSION_INFO).expect("version info resource");
    match r.parsed() {
        Some(Ok(ResourceData::VersionInfo(v))) => (v.writer.to_string_lossy(), v.reader.to_string_lossy()),
        other => panic!("{other:?}"),
    }
}

fn built() -> PsdFile {
    let mut b = PsdBuilder::new(2, 1);
    b.push_layer(LayerSpec::new("L", 0, 0, 2, 1, PixelData::Rgba8(vec![255, 0, 0, 255, 0, 255, 0, 255])));
    b.composite(PixelData::Rgba8(vec![255, 0, 0, 255, 0, 255, 0, 255]));
    PsdFile::from_bytes(&b.to_bytes().unwrap()).unwrap()
}

#[test]
fn the_app_name_replaces_writer_and_reader_once_and_unset_keeps_upstream() {
    assert_eq!(app_name(), None);
    let upstream = version_info_resource(true);
    let (w, r) = names(&built());
    assert_eq!((w.trim_end_matches('\0'), r.trim_end_matches('\0')), ("Adobe Photoshop", "Adobe Photoshop CS6"));

    assert!(set_app_name("Zz Test App"));
    assert!(!set_app_name("Another"), "a second name is refused");
    assert_eq!(app_name(), Some("Zz Test App"));
    assert_ne!(version_info_resource(true).data, upstream.data);
    let (w, r) = names(&built());
    assert_eq!((w.trim_end_matches('\0'), r.trim_end_matches('\0')), ("Zz Test App", "Zz Test App"));
}
