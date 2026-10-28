//! Destructive pixel operations on layers (Image → Adjustments, flips, fills).

use photocraft_color::PixelFormat;
use photocraft_compose::{Buffer, adjust};
use photocraft_doc::{Adjustment, Document, Layer, LayerContent};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba, to_rgba};

/// Apply an adjustment destructively to a surface, weighted by an optional selection.
/// Applies `adj` to a surface (any colour model and depth, via straight RGBA) through the
/// selection; `mode` is the document's, for the tone transfer (e.g. Exposure in Grayscale).
pub fn adjust_surface(s: &mut Surface, adj: &Adjustment, selection: Option<&Surface>, mode: photocraft_color::ColorMode) {
    let r = s.content_bounds();
    if r.is_empty() {
        return;
    }
    let transfer = adjust::Transfer::for_mode(mode);
    match channel_tables(s.format(), adj, selection, transfer) {
        Some(tables) => adjust_by_tables(s, r, &tables),
        None => adjust_region(s, r, adj, selection, transfer),
    }
}

/// The general path: one tile at a time through straight float RGBA. Every adjustment is per pixel (the
/// positional ones read the buffer's own rect), so this gives the bytes one whole-layer buffer would, without
/// holding two float copies of the layer.
fn adjust_region(s: &mut Surface, r: Rect, adj: &Adjustment, selection: Option<&Surface>, transfer: adjust::Transfer) {
    let fmt = s.format();
    let n = fmt.channels();
    let (mut raw, mut out) = (Vec::new(), Vec::new());
    let mut buf = Buffer { rect: Rect::EMPTY, px: Vec::new() };
    let mut orig: Vec<[f32; 4]> = Vec::new();
    for tc in r.tiles() {
        let tr = tc.rect().intersect(&r);
        if tr.is_empty() {
            continue;
        }
        s.read_region_into(tr, &mut raw);
        buf.rect = tr;
        buf.px.clear();
        buf.px.extend(raw.chunks_exact(n).map(|p| to_rgba(&fmt, p)));
        orig.clear();
        orig.extend_from_slice(&buf.px);
        adjust::apply_with(adj, &mut buf, transfer);
        let w = tr.width() as usize;
        out.clear();
        for (i, (a, o)) in buf.px.iter().zip(&orig).enumerate() {
            let k = selection.map_or(1.0, |sel| sel.sample_channel(tr.x0 + (i % w) as i32, tr.y0 + (i / w) as i32, 0));
            let mixed: [f32; 4] = std::array::from_fn(|c| o[c] + (a[c] - o[c]) * k);
            let mut enc = [0.0f32; 8];
            let m = photocraft_raster::from_rgba_into(&fmt, mixed, &mut enc);
            out.extend_from_slice(&enc[..m]);
        }
        s.write_region(tr, &out);
    }
}

/// For an 8-bit RGB surface, no selection and an adjustment where each output channel depends only on the same
/// input channel: one byte table per channel. Built by running [`adjust_region`] on a 256-pixel ramp, so the
/// tables hold exactly the bytes the general path writes.
fn channel_tables(fmt: PixelFormat, adj: &Adjustment, selection: Option<&Surface>, transfer: adjust::Transfer) -> Option<[[u8; 256]; 3]> {
    use photocraft_doc::adjust::ToneSpace;
    let separable = match adj {
        Adjustment::Invert | Adjustment::Posterize { .. } | Adjustment::BrightnessContrast { .. } | Adjustment::Exposure { .. } => true,
        Adjustment::Levels { space, .. } | Adjustment::Curves { space, .. } => *space == ToneSpace::Rgb,
        _ => false,
    };
    if !separable || selection.is_some() || fmt.mode != photocraft_color::ColorMode::Rgb || fmt.sample != photocraft_color::SampleType::U8 {
        return None;
    }
    let n = fmt.channels();
    let ramp_rect = Rect::from_xywh(0, 0, 256, 1);
    let mut ramp = Surface::new(fmt);
    let mut px = Vec::with_capacity(256 * n);
    for v in 0..256 {
        let v = v as f32 / 255.0;
        px.extend_from_slice(&[v, v, v]);
        if fmt.alpha {
            px.push(1.0);
        }
    }
    ramp.write_region(ramp_rect, &px);
    adjust_region(&mut ramp, ramp_rect, adj, None, transfer);
    let tile = ramp.tile(photocraft_geom::TileCoord { tx: 0, ty: 0 })?;
    let mut tables = [[0u8; 256]; 3];
    for (v, p) in tile.bytes()[..256 * n].chunks_exact(n).enumerate() {
        for c in 0..3 {
            tables[c][v] = p[c];
        }
    }
    Some(tables)
}

/// Applies per-channel byte tables over `r` of an 8-bit RGB surface. Fully transparent pixels stay as they are,
/// as the general path leaves them.
fn adjust_by_tables(s: &mut Surface, r: Rect, tables: &[[u8; 256]; 3]) {
    let n = s.format().channels();
    let alpha = s.format().alpha;
    let ts = photocraft_geom::TILE_SIZE;
    for tc in r.tiles() {
        let tr = tc.rect().intersect(&r);
        if tr.is_empty() {
            continue;
        }
        let data = s.tile_mut(tc).bytes_mut();
        for y in tr.y0..tr.y1 {
            let row = ((y - tc.ty * ts) * ts + (tr.x0 - tc.tx * ts)) as usize * n;
            for p in data[row..row + tr.width() as usize * n].chunks_exact_mut(n) {
                if alpha && p[3] == 0 {
                    continue;
                }
                p[0] = tables[0][p[0] as usize];
                p[1] = tables[1][p[1] as usize];
                p[2] = tables[2][p[2] as usize];
            }
        }
    }
}

/// Fill (respecting selection coverage) with a straight RGBA colour.
pub fn fill_surface(s: &mut Surface, area: Rect, color: [f32; 4], selection: Option<&Surface>, lock_transparency: bool) {
    let fmt = s.format();
    let n = fmt.channels();
    let mut region = s.read_region(area);
    let w = area.width() as usize;
    for (i, px) in region.chunks_exact_mut(n).enumerate() {
        let x = area.x0 + (i % w) as i32;
        let y = area.y0 + (i / w) as i32;
        let k = selection.map_or(1.0, |sel| sel.sample_channel(x, y, 0));
        if k <= 0.0 {
            continue;
        }
        let d = to_rgba(&fmt, px);
        let sa = color[3] * k;
        let oa = sa + d[3] * (1.0 - sa);
        let mut o = [0.0f32; 4];
        if oa > 0.0 {
            for c in 0..3 {
                o[c] = (color[c] * sa + d[c] * d[3] * (1.0 - sa)) / oa;
            }
        }
        o[3] = if lock_transparency { d[3] } else { oa };
        let mut enc = [0.0f32; 8];
        photocraft_raster::from_rgba_into(&fmt, o, &mut enc);
        px.copy_from_slice(&enc[..n]);
    }
    s.write_region(area, &region);
}

/// Clear pixels (make transparent) within the selection.
pub fn clear_surface(s: &mut Surface, area: Rect, selection: Option<&Surface>) {
    let fmt = s.format();
    let n = fmt.channels();
    if !fmt.alpha {
        return;
    }
    let mut region = s.read_region(area);
    let w = area.width() as usize;
    for (i, px) in region.chunks_exact_mut(n).enumerate() {
        let k = selection.map_or(1.0, |sel| sel.sample_channel(area.x0 + (i % w) as i32, area.y0 + (i / w) as i32, 0));
        px[n - 1] *= 1.0 - k;
        // Fully cleared pixels become the empty pixel, so bounds and tile pruning see them as gone.
        if px[n - 1] <= 0.0 {
            px.fill(0.0);
        }
    }
    s.write_region(area, &region);
    s.prune();
}

/// Remap a surface through a pixel-coordinate mapping (used for flips/rotations).
pub fn remap_surface(s: &Surface, map: impl Fn(i32, i32) -> (i32, i32)) -> Surface {
    let r = s.content_bounds();
    let mut out = Surface::with_default(s.format(), &s.default_pixel());
    let n = s.channels();
    let raw = s.read_region(r);
    let w = r.width() as usize;
    for (i, px) in raw.chunks_exact(n).enumerate() {
        let (x, y) = (r.x0 + (i % w) as i32, r.y0 + (i / w) as i32);
        let (nx, ny) = map(x, y);
        out.write_pixel(nx, ny, px);
    }
    out
}

/// Merge `upper` onto `lower` producing a raster layer (Layer → Merge Down).
pub fn merge_down(doc_bounds: Rect, lower: &Layer, upper: &Layer, format: photocraft_color::PixelFormat) -> Layer {
    let area = [lower, upper].iter().map(|l| l.surface().map(|s| s.content_bounds()).unwrap_or(doc_bounds)).fold(Rect::EMPTY, |a, b| a.union(&b));
    let stack = vec![lower.clone(), upper.clone()];
    let mut tmp = Document::new("merge", doc_bounds.size(), format.mode, format.sample);
    tmp.layers = stack;
    let buf = photocraft_compose::render(&tmp, area);
    let mut s = Surface::new(format);
    let data: Vec<f32> = buf.px.iter().flat_map(|p| from_rgba(&format, *p)).collect();
    if !area.is_empty() {
        s.write_region(area, &data);
        s.prune();
    }
    let mut merged = Layer::new(lower.name.clone(), LayerContent::Raster(s));
    merged.id = lower.id;
    merged.blend = lower.blend;
    merged.opacity = 1.0;
    merged
}

#[cfg(test)]
mod table_tests {
    use super::*;
    use photocraft_doc::adjust::{CurvePoint, LevelsChannel, ToneSpace};

    /// A 300x260 RGBA8 surface (spans tile edges) with varied colours and some transparent and partly
    /// transparent pixels.
    fn sample() -> Surface {
        let mut s = Surface::new(PixelFormat::RGBA8);
        let r = Rect::from_xywh(3, 5, 300, 260);
        let mut seed = 7u32;
        let mut px = Vec::with_capacity(300 * 260 * 4);
        for _ in 0..300 * 260 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let b = |k: u32| ((seed >> k) & 255) as f32 / 255.0;
            let a = match (seed >> 3) % 7 {
                0 => 0.0,
                1 => 0.5,
                _ => 1.0,
            };
            px.extend_from_slice(&[b(8), b(16), b(24), a]);
        }
        s.write_region(r, &px);
        s
    }

    fn adjustments() -> Vec<Adjustment> {
        let lv = LevelsChannel::default();
        vec![
            Adjustment::Invert,
            Adjustment::Posterize { levels: 4 },
            Adjustment::BrightnessContrast { brightness: 37.0, contrast: -12.0, legacy: false },
            Adjustment::BrightnessContrast { brightness: -80.0, contrast: 60.0, legacy: true },
            Adjustment::Exposure { exposure: 0.7, offset: 0.01, gamma: 1.2 },
            Adjustment::Levels {
                master: LevelsChannel { gamma: 1.4, ..lv.clone() },
                per_channel: [lv.clone(), LevelsChannel { in_black: 0.1, ..lv.clone() }, lv.clone()],
                space: ToneSpace::Rgb,
                black: lv.clone(),
            },
            Adjustment::Curves {
                master: vec![CurvePoint { input: 0.0, output: 0.0 }, CurvePoint { input: 0.4, output: 0.6 }, CurvePoint { input: 1.0, output: 1.0 }],
                per_channel: [vec![], vec![], vec![]],
                space: ToneSpace::Rgb,
                black: vec![],
            },
        ]
    }

    #[test]
    fn tables_give_the_bytes_of_the_general_path() {
        let transfer = adjust::Transfer::for_mode(photocraft_color::ColorMode::Rgb);
        for adj in adjustments() {
            let mut general = sample();
            let r = general.content_bounds();
            adjust_region(&mut general, r, &adj, None, transfer);
            let mut fast = sample();
            assert!(channel_tables(fast.format(), &adj, None, transfer).is_some(), "{adj:?} takes the table path");
            adjust_surface(&mut fast, &adj, None, photocraft_color::ColorMode::Rgb);
            assert!(general == fast, "{adj:?}: table path differs from the general path");
        }
    }
}
