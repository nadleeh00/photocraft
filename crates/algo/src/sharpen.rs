//! Unsharp Mask, Smart Sharpen (basic), High Pass.

use photocraft_geom::Rect;

use crate::Ctx;
use crate::blur::gaussian;
use crate::image::Image;

fn with_blur(src: &Image, out: Rect, ctx: &Ctx, radius: f32, mut f: impl FnMut(f32, f32) -> f32) -> Vec<f32> {
    let blurred = gaussian(src, out, ctx, radius);
    let mut res = src.crop(out);
    let n = src.ch;
    let cc = if ctx.alpha { n - 1 } else { n };
    for (px, bl) in res.chunks_exact_mut(n).zip(blurred.chunks_exact(n)) {
        if ctx.alpha && px[n - 1] <= 0.0 {
            continue;
        }
        for c in 0..cc {
            px[c] = f(px[c], bl[c]);
        }
    }
    res
}

pub(crate) fn unsharp(src: &Image, out: Rect, ctx: &Ctx, amount: f32, radius: f32, threshold: f32) -> Vec<f32> {
    let k = amount / 100.0;
    let t = threshold / 255.0;
    with_blur(src, out, ctx, radius, |o, b| {
        let d = o - b;
        if d.abs() + 1e-6 >= t { o + k * d } else { o }
    })
}

pub(crate) fn smart(src: &Image, out: Rect, ctx: &Ctx, amount: f32, radius: f32, reduce_noise: f32) -> Vec<f32> {
    let k = amount / 100.0;
    let nr = (reduce_noise / 100.0).clamp(0.0, 1.0);
    with_blur(src, out, ctx, radius, |o, b| {
        let d = o - b;
        // Suppress small (noise-level) differences.
        let keep = 1.0 - nr * (-(d.abs() * 40.0)).exp();
        o + k * d * keep
    })
}

pub(crate) fn high_pass(src: &Image, out: Rect, ctx: &Ctx, radius: f32) -> Vec<f32> {
    with_blur(src, out, ctx, radius, |o, b| 0.5 + (o - b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blur::tests::{test_ctx, textured};
    use crate::blur::{conv_sep, gaussian_kernel};

    /// The pre-fast-path result: the full exact kernel.
    fn unsharp_exact(src: &Image, out: Rect, ctx: &Ctx, amount: f32, radius: f32, threshold: f32) -> Vec<f32> {
        let k = gaussian_kernel(radius);
        let blurred = conv_sep(src, out, &k, &k, ctx.alpha);
        let mut res = src.crop(out);
        let (kk, t) = (amount / 100.0, threshold / 255.0);
        for (px, bl) in res.chunks_exact_mut(2).zip(blurred.chunks_exact(2)) {
            if px[1] <= 0.0 {
                continue;
            }
            let d = px[0] - bl[0];
            px[0] = if d.abs() + 1e-6 >= t { px[0] + kk * d } else { px[0] };
        }
        res
    }

    fn max_err(radius: f32) -> f32 {
        // Source reaches `radius` past `out` on every side, as the caller provides.
        let m = (radius * 3.0).ceil() as i32 + 2;
        let out = Rect::new(0, 0, 80, 60);
        let img = textured(Rect::new(-m, -m, 80 + m, 60 + m));
        let ctx = test_ctx(img.rect);
        let old = unsharp_exact(&img, out, &ctx, 100.0, radius, 0.0);
        let new = unsharp(&img, out, &ctx, 100.0, radius, 0.0);
        old.iter().zip(&new).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max)
    }

    #[test]
    fn unsharp_fast_path_matches_exact_kernel() {
        let (e8, e50) = (max_err(8.0), max_err(50.0));
        eprintln!("unsharp max error: r8 {e8}, r50 {e50}");
        assert!(e8 <= 0.02, "radius 8 max error {e8}");
        assert!(e50 <= 0.02, "radius 50 max error {e50}");
    }

    #[test]
    #[ignore = "timing; run once in release"]
    fn timing_radius_1000() {
        use crate::{FilterParams, apply, output_area};
        use photocraft_color::PixelFormat;
        use photocraft_raster::Surface;
        let n = 2000;
        let r = Rect::new(0, 0, n, n);
        let mut s = Surface::new(PixelFormat::RGBA8);
        let row: Vec<u8> = (0..n).flat_map(|x| [(x % 256) as u8, (x / 7 % 256) as u8, 128, 255]).collect();
        let bytes: Vec<u8> = (0..n).flat_map(|_| row.iter().copied()).collect();
        s.write_interleaved(r, &bytes);
        let time = |p: FilterParams| {
            let area = output_area(&p, s.content_bounds(), r, None).intersect(&r);
            let t = std::time::Instant::now();
            apply(&s, &p, area, r, None);
            t.elapsed().as_secs_f64()
        };
        let gauss = time(FilterParams::GaussianBlur { radius: 1000.0 });
        let unsharp = time(FilterParams::UnsharpMask { amount: 100.0, radius: 1000.0, threshold: 0.0 });
        let boxed = time(FilterParams::BoxBlur { radius: 1000.0 });
        eprintln!("r1000 on {n}x{n}: gaussianBlur {gauss:.2}s, unsharp {unsharp:.2}s, boxBlur {boxed:.2}s");
        assert!(unsharp <= 2.0 * gauss, "unsharp {unsharp}s vs gaussian {gauss}s");
        assert!(boxed <= 2.0 * gauss, "boxBlur {boxed}s vs gaussian {gauss}s");
    }
}
