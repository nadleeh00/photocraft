//! `PsdFile::decoded_bytes`: the decode size, computed from stored rectangles.

use photocraft_psd::{LayerSpec, PixelData, PsdBuilder, PsdFile};

#[test]
fn decoded_bytes_counts_every_layer_channel_and_the_merged_image() {
    let (w, h) = (6u32, 4u32);
    let mut b = PsdBuilder::new(w, h);
    b.push_layer(LayerSpec::new("full", 0, 0, w, h, PixelData::Rgba8(vec![9; (w * h * 4) as usize])));
    b.push_layer(LayerSpec::new("small", 1, 1, 2, 3, PixelData::Rgba8(vec![9; 2 * 3 * 4])));
    b.composite(PixelData::Rgba8(vec![9; (w * h * 4) as usize]));
    let file = PsdFile::from_bytes(&b.to_bytes().unwrap()).unwrap();

    let merged = u64::from(file.header.channels) * u64::from(w * h);
    let layers: u64 = file
        .layer_info
        .as_ref()
        .unwrap()
        .layers
        .iter()
        .map(|l| l.channels.iter().map(|c| { let (cw, ch) = l.channel_rect(c.id).size().unwrap(); (cw * ch) as u64 }).sum::<u64>())
        .sum();
    // 8-bit: one byte per sample. Full layer: 4 channels × 24; small: 4 × 6.
    assert_eq!(layers, 4 * 24 + 4 * 6);
    assert_eq!(file.decoded_bytes().unwrap(), merged + layers);
    // Decoding actually yields that many bytes.
    let decoded: u64 = file
        .layer_info
        .as_ref()
        .unwrap()
        .layers
        .iter()
        .map(|l| l.channels.iter().map(|c| l.decode_channel(c.id, file.header.depth, file.header.version).unwrap().len() as u64).sum::<u64>())
        .sum::<u64>()
        + file.image_data.decode(&file.header).unwrap().len() as u64;
    assert_eq!(file.decoded_bytes().unwrap(), decoded);
}
