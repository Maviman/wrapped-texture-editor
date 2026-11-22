root = r"E:\claude_programs\wrapped_texture_editor\target\round4\work\merged\crates\wte_core\src"
p = root + r"\text\raster.rs"
s = open(p, encoding="utf-8", newline="").read()
probe = r'''

/// Gate stage 3 (round 4): the classic painter on the three baseline cases B1-B3 and the font timing B4 of
/// `target\round4\perf\text_baseline.txt`, measured exactly as P0's probe did on the round-3 tree (rasterize_text and
/// PremulMips::build, median of 5 after one warm-up; TEXT 8.4 rows 1 and "B4"). Run with
/// `cargo.cmd test -p wte_core --release --lib gate_classic_painter -- --ignored --nocapture --test-threads=1`.
#[cfg(test)]
mod gate_probe {
    use std::sync::Arc;
    use std::time::Instant;

    use crate::blob::{BlobKind, BlobStore};
    use crate::pixel_ops::PremulMips;
    use crate::text::{FontCache, FontRef, Outline, TextAlign, TextFx, TextSpec, measure_text, rasterize_text};

    fn base(content: &str, px: f32) -> TextSpec {
        TextSpec {
            content: content.to_owned(),
            font: FontRef { blob: crate::ids::BlobId(0), face_index: 0, family: "Ubuntu".to_owned(), style: "Light".to_owned() },
            size_px: px,
            color: [220, 30, 40, 255],
            align: TextAlign::Left,
            line_spacing: 1.0,
            letter_spacing: 0.0,
            outline: None,
            fx: TextFx::default(),
        }
    }

    fn ubuntu(px: f32) -> Arc<fontdue::Font> {
        FontCache::default().fallback(px)
    }

    fn median(mut v: Vec<f64>) -> f64 {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    }

    fn time5(name: &str, mut f: impl FnMut() -> f64) {
        f();
        let v: Vec<f64> = (0..5).map(|_| f()).collect();
        let r: Vec<f64> = v.iter().map(|x| (x * 1000.0).round() / 1000.0).collect();
        eprintln!("gate time {name}: median {:.3} ms (5 runs after one warm-up: {r:?})", median(v));
    }

    #[test]
    #[ignore = "timing probe of the gate"]
    fn gate_classic_painter() {
        let raster_mips = |spec: &TextSpec, mips: bool| {
            let u = ubuntu(spec.size_px);
            let t = Instant::now();
            let img = rasterize_text(&[&*u], spec).unwrap();
            if mips {
                std::hint::black_box(PremulMips::build(&img));
            }
            std::hint::black_box(&img);
            t.elapsed().as_secs_f64() * 1e3
        };
        let b1 = base("WTE 2026", 256.0);
        time5("B1 rasterize_text + PremulMips::build, \"WTE 2026\" at 256 px per em", || raster_mips(&b1, true));
        time5("B1 rasterize_text only", || raster_mips(&b1, false));
        let mut b2 = base("ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789", 128.0);
        b2.content.truncate(60);
        b2.outline = Some(Outline { width_em: 0.06, color: [0, 0, 0, 255] });
        time5("B2 rasterize_text + PremulMips::build, 60 letters at 128 px per em, 6 % outline", || raster_mips(&b2, true));
        time5("B2 rasterize_text only", || raster_mips(&b2, false));
        let u512 = ubuntu(512.0);
        let pattern: Vec<char> = "WTE 2026 ".chars().collect();
        let mut content = String::new();
        let mut i = 0;
        loop {
            content.push(pattern[i % pattern.len()]);
            i += 1;
            let [w, _] = measure_text(&[&*u512], &base(&content, 512.0)).unwrap();
            if w as f32 >= 16.0 * 512.0 * 0.98 || i > 200 {
                break;
            }
        }
        let b3 = base(&content, 512.0);
        let img = rasterize_text(&[&*u512], &b3).unwrap();
        eprintln!("gate B3 content {:?} ({} chars), picture {}x{}", content, content.chars().count(), img.width, img.height);
        time5("B3 rasterize_text + PremulMips::build, one line 16 em at 512 px per em", || raster_mips(&b3, true));
        time5("B3 rasterize_text only", || raster_mips(&b3, false));
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        let mut dirs = vec![std::path::PathBuf::from(r"C:\Windows\Fonts")];
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(std::path::Path::new(&local).join(r"Microsoft\Windows\Fonts"));
        }
        for d in &dirs {
            if let Ok(rd) = std::fs::read_dir(d) {
                for e in rd.flatten() {
                    let p = e.path();
                    let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
                    if ["ttf", "otf", "ttc"].contains(&ext.as_str()) {
                        files.push(p);
                    }
                }
            }
        }
        let largest = files.iter().max_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)).cloned();
        let arial = std::path::PathBuf::from(r"C:\Windows\Fonts\arial.ttf");
        eprintln!("gate B4 font files found: {}", files.len());
        for (name, path) in [("Arial", Some(arial)), ("the largest installed font file", largest)] {
            let Some(path) = path else { continue };
            let Ok(bytes) = std::fs::read(&path) else {
                eprintln!("gate B4 {name}: cannot read {}", path.display());
                continue;
            };
            eprintln!("gate B4 {name}: {} ({} bytes)", path.display(), bytes.len());
            let bytes: Arc<[u8]> = Arc::from(bytes);
            let mut blobs = BlobStore::default();
            let blob = blobs.insert(BlobKind::Font, "f", Arc::clone(&bytes));
            let font = FontRef { blob, face_index: 0, family: name.to_owned(), style: "Regular".to_owned() };
            time5(&format!("B4 first FontCache::get of {name} at the 128 bucket"), || {
                let mut cache = FontCache::default();
                let t = Instant::now();
                std::hint::black_box(cache.get(&blobs, &font, 128.0).unwrap());
                t.elapsed().as_secs_f64() * 1e3
            });
        }
        time5("font scan at start (FontCatalog::bundled + load_system)", || {
            let t = Instant::now();
            let mut c = crate::text::FontCatalog::bundled();
            c.load_system();
            let ms = t.elapsed().as_secs_f64() * 1e3;
            std::hint::black_box(c.families().len());
            ms
        });
    }
}
'''
if "mod gate_probe" not in s:
    if not s.endswith("\r\n"):
        s += "\r\n"
    s += probe.lstrip("\n").replace("\n", "\r\n")
    open(p, "w", encoding="utf-8", newline="").write(s)
print("ok")
