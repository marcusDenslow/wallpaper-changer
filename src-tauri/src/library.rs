use image::codecs::jpeg::JpegEncoder;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{self, AtomicU64};

const EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp"];
const THUMB_W: u32 = 1280;
const THUMB_H: u32 = 720;
const MAX_DEPTH: usize = 3;

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Wallpaper {
    pub path: String,
    pub name: String,
    pub file_name: String,
    pub folder: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Serialize, Clone, Debug)]
pub struct Thumb {
    pub path: String,
    pub color: String,
}

pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

pub fn scan(root: &Path) -> Vec<Wallpaper> {
    let mut found = Vec::new();
    walk(root, root, 0, &mut found);
    found.sort_by(|a, b| {
        a.folder
            .to_lowercase()
            .cmp(&b.folder.to_lowercase())
            .then_with(|| natural_cmp(&a.name.to_lowercase(), &b.name.to_lowercase()))
    });
    found
}

fn walk(root: &Path, dir: &Path, depth: usize, found: &mut Vec<Wallpaper>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            if depth < MAX_DEPTH {
                walk(root, &path, depth + 1, found);
            }
            continue;
        }
        if !is_image(&path) {
            continue;
        }

        let (width, height) = imagesize::size(&path)
            .map(|size| (size.width as u32, size.height as u32))
            .unwrap_or((0, 0));
        let folder = path
            .parent()
            .and_then(|parent| parent.strip_prefix(root).ok())
            .map(|rel| rel.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();

        found.push(Wallpaper {
            name: path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            path: path.to_string_lossy().into_owned(),
            file_name: name,
            folder,
            width,
            height,
        });
    }
}

fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut left = a.chars().peekable();
    let mut right = b.chars().peekable();
    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut n1 = String::new();
                while let Some(c) = left.peek().copied().filter(char::is_ascii_digit) {
                    n1.push(c);
                    left.next();
                }
                let mut n2 = String::new();
                while let Some(c) = right.peek().copied().filter(char::is_ascii_digit) {
                    n2.push(c);
                    right.next();
                }
                let (n1, n2) = (n1.trim_start_matches('0'), n2.trim_start_matches('0'));
                let order = n1.len().cmp(&n2.len()).then_with(|| n1.cmp(n2));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                left.next();
                right.next();
            }
        }
    }
}

fn ambient_color(img: &image::RgbImage) -> String {
    let small = image::imageops::thumbnail(img, 24, 14);
    let (mut r, mut g, mut b, mut total) = (0f64, 0f64, 0f64, 0f64);
    for pixel in small.pixels() {
        let [pr, pg, pb] = pixel.0.map(|c| c as f64 / 255.0);
        let max = pr.max(pg).max(pb);
        let min = pr.min(pg).min(pb);
        let saturation = if max > 0.0 { (max - min) / max } else { 0.0 };
        let weight = 0.08 + saturation * saturation * (0.25 + max);
        r += pr * weight;
        g += pg * weight;
        b += pb * weight;
        total += weight;
    }
    let channel = |v: f64| ((v / total).clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", channel(r), channel(g), channel(b))
}

fn write_atomic(dir: &Path, key: u64, target: &Path, write: impl FnOnce(&Path) -> Result<(), String>) -> Result<(), String> {
    let n = TMP_COUNTER.fetch_add(1, atomic::Ordering::Relaxed);
    let tmp = dir.join(format!("{key:016x}.{}.{n}.tmp", std::process::id()));
    write(&tmp)?;
    fs::rename(&tmp, target).or_else(|e| {
        let _ = fs::remove_file(&tmp);
        if target.is_file() {
            Ok(())
        } else {
            Err(e.to_string())
        }
    })
}

pub fn thumbnail(cache_dir: &Path, source: &Path) -> Result<Thumb, String> {
    let meta = fs::metadata(source).map_err(|e| format!("{}: {e}", source.display()))?;
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    meta.len().hash(&mut hasher);
    meta.modified().ok().hash(&mut hasher);
    THUMB_W.hash(&mut hasher);
    let key = hasher.finish();

    let dir = cache_dir.join("thumbs");
    let jpg = dir.join(format!("{key:016x}.jpg"));
    let color_file = dir.join(format!("{key:016x}.color"));
    if jpg.is_file() {
        if let Ok(color) = fs::read_to_string(&color_file) {
            if color.len() == 7 {
                return Ok(Thumb { path: jpg.to_string_lossy().into_owned(), color });
            }
        }
    }
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let img = image::ImageReader::open(source)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| format!("Couldn't read {}: {e}", source.display()))?;

    let (w, h) = (img.width().max(1), img.height().max(1));
    let scale = (THUMB_W as f64 / w as f64).max(THUMB_H as f64 / h as f64).min(1.0);
    let small = if scale < 1.0 {
        let tw = ((w as f64 * scale).round() as u32).max(1);
        let th = ((h as f64 * scale).round() as u32).max(1);
        img.thumbnail_exact(tw, th)
    } else {
        img
    };
    let rgb = small.to_rgb8();
    let color = ambient_color(&rgb);

    write_atomic(&dir, key, &jpg, |tmp| {
        let mut out = BufWriter::new(File::create(tmp).map_err(|e| e.to_string())?);
        JpegEncoder::new_with_quality(&mut out, 86)
            .encode_image(&rgb)
            .map_err(|e| e.to_string())
    })?;
    write_atomic(&dir, key, &color_file, |tmp| fs::write(tmp, &color).map_err(|e| e.to_string()))?;
    Ok(Thumb { path: jpg.to_string_lossy().into_owned(), color })
}

pub fn prewarm(cache_dir: PathBuf, items: Vec<Wallpaper>) {
    let _ = std::thread::Builder::new().name("thumbnails".into()).spawn(move || {
        for item in items {
            let _ = thumbnail(&cache_dir, Path::new(&item.path));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_numbers_naturally() {
        let mut names = vec!["wall10", "wall2", "wall1", "aurora", "wall02b"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, vec!["aurora", "wall1", "wall2", "wall02b", "wall10"]);
    }

    #[test]
    fn scans_and_caches_thumbnails() {
        let dir = std::env::temp_dir().join(format!("wallswitch-test-{}", std::process::id()));
        let sub = dir.join("Nature");
        fs::create_dir_all(&sub).unwrap();
        image::RgbImage::from_fn(3840, 1600, |x, y| image::Rgb([(x % 255) as u8, (y % 255) as u8, 90]))
            .save(sub.join("wide.png"))
            .unwrap();
        image::RgbImage::from_pixel(800, 600, image::Rgb([10, 20, 30]))
            .save(dir.join("small.jpg"))
            .unwrap();
        fs::write(dir.join("notes.txt"), "x").unwrap();

        let found = scan(&dir);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].file_name, "small.jpg");
        assert_eq!(found[1].folder, "Nature");
        assert_eq!((found[1].width, found[1].height), (3840, 1600));

        let cache = dir.join("cache");
        let wide = thumbnail(&cache, Path::new(&found[1].path)).unwrap();
        assert!(wide.color.starts_with('#') && wide.color.len() == 7);
        let (tw, th) = image::image_dimensions(&wide.path).unwrap();
        assert!(tw >= THUMB_W && th >= THUMB_H && tw < 3840, "{tw}x{th}");
        assert_eq!(thumbnail(&cache, Path::new(&found[1].path)).unwrap().path, wide.path);

        let small = thumbnail(&cache, Path::new(&found[0].path)).unwrap();
        assert_eq!(image::image_dimensions(&small.path).unwrap(), (800, 600));
        assert_eq!(small.color, "#0a141e");
        fs::remove_dir_all(&dir).ok();
    }
}
