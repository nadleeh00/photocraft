//! The app name set through `photocraft_psd::resources::set_app_name` reaches the EPS and LUT
//! writers. One test: the name is a per-process global.

use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image};
use photocraft_engine::{file_cmds, print_cmds};

#[test]
fn eps_and_lut_creator_lines_follow_the_app_name() {
    let img = Image::from_u8(4, 4, ChannelLayout::Rgb, vec![120; 4 * 4 * 3]).unwrap();
    let png = photocraft_codecs::encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
    let doc = photocraft_io::import("a.png", &png).unwrap().document;

    assert!(print_cmds::paths_to_ai(&doc, "all").unwrap().0.contains("%%Creator: PhotoCraft\n"));
    assert!(file_cmds::bake_cube(&doc, 2, "t").contains("# Created by Photocraft\n"));

    assert!(photocraft_psd::resources::set_app_name("Zz Test App"));
    assert!(print_cmds::paths_to_ai(&doc, "all").unwrap().0.contains("%%Creator: Zz Test App\n"));
    assert!(file_cmds::bake_cube(&doc, 2, "t").contains("# Created by Zz Test App\n"));
}
