use image::codecs::jpeg::JpegEncoder;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::BufWriter;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{self, AtomicU32, AtomicU64};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime};

const EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp"];
const MAX_DEPTH: usize = 3;
/// Thumbnail widths (16:9). 1280 is what 1.1.0 always used.
const THUMB_SIZES: [u32; 3] = [960, 1280, 1600];

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);
static THUMB_W: AtomicU32 = AtomicU32::new(THUMB_SIZES[0]);

/// Image sizes from earlier scans, keyed by path and checked against size + modified time,
/// so reopening doesn't open every file again just to read its header.
type Stamp = (u64, Option<SystemTime>);
static SIZES: LazyLock<Mutex<HashMap<PathBuf, (Stamp, (u32, u32))>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Smallest thumbnail that covers the selected card on the biggest screen.
/// The card is min(34vw, 720px, 42vh × 16/9) (ui::Ui::measure); the 720px cap is in
/// DPI-scaled pixels, which we don't know here, so it's left out: a 4K screen gets
/// 1280 (as in 1.1.0) even at 100%, 1080p and 1440p screens get 960.
pub fn thumb_width_for(screens: &[[i32; 4]]) -> u32 {
    let card = screens
        .iter()
        .map(|r| {
            let (w, h) = ((r[2] - r[0]).max(0) as f32, (r[3] - r[1]).max(0) as f32);
            (w * 0.34).min(h * 0.42 * 16.0 / 9.0)
        })
        .fold(0.0, f32::max);
    // up to 5% upscaling isn't visible
    THUMB_SIZES.into_iter().find(|&t| card <= t as f32 * 1.05).unwrap_or(THUMB_SIZES[THUMB_SIZES.len() - 1])
}

/// Picks the thumbnail size for these screens (physical pixels). Call before asking for thumbnails.
pub fn use_screens(screens: &[[i32; 4]]) {
    THUMB_W.store(thumb_width_for(screens), atomic::Ordering::Relaxed);
}

fn thumb_size() -> (u32, u32) {
    let w = THUMB_W.load(atomic::Ordering::Relaxed);
    (w, w * 9 / 16)
}

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

/// Lists the wallpapers under `root`, remembering image sizes for the next scan.
pub fn scan(root: &Path) -> Vec<Wallpaper> {
    scan_with(root, true)
}

/// Same, without keeping image sizes around (for the long-lived tray process).
pub fn scan_once(root: &Path) -> Vec<Wallpaper> {
    scan_with(root, false)
}

fn scan_with(root: &Path, remember: bool) -> Vec<Wallpaper> {
    let mut found = Vec::new();
    walk(root, root, 0, remember, &mut found);
    found.sort_by(|a, b| {
        a.folder
            .to_lowercase()
            .cmp(&b.folder.to_lowercase())
            .then_with(|| natural_cmp(&a.name.to_lowercase(), &b.name.to_lowercase()))
    });
    found
}

fn walk(root: &Path, dir: &Path, depth: usize, remember: bool, found: &mut Vec<Wallpaper>) {
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
                walk(root, &path, depth + 1, remember, found);
            }
            continue;
        }
        if !is_image(&path) {
            continue;
        }

        let (width, height) = if remember {
            image_size(&path, (meta.len(), meta.modified().ok()))
        } else {
            imagesize::size(&path).map(|s| (s.width as u32, s.height as u32)).unwrap_or((0, 0))
        };
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

fn image_size(path: &Path, stamp: Stamp) -> (u32, u32) {
    if let Some((seen, size)) = SIZES.lock().unwrap().get(path) {
        if *seen == stamp {
            return *size;
        }
    }
    let size = imagesize::size(path).map(|s| (s.width as u32, s.height as u32)).unwrap_or((0, 0));
    SIZES.lock().unwrap().insert(path.to_path_buf(), (stamp, size));
    size
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

fn cache_key(source: &Path, meta: &fs::Metadata, width: u32) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    meta.len().hash(&mut hasher);
    meta.modified().ok().hash(&mut hasher);
    width.hash(&mut hasher);
    hasher.finish()
}

pub fn thumbnail(cache_dir: &Path, source: &Path) -> Result<Thumb, String> {
    let meta = fs::metadata(source).map_err(|e| format!("{}: {e}", source.display()))?;
    let (thumb_w, thumb_h) = thumb_size();
    let key = cache_key(source, &meta, thumb_w);

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
    let scale = (thumb_w as f64 / w as f64).max(thumb_h as f64 / h as f64).min(1.0);
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

/// Deletes cached thumbnails that no wallpaper in `items` uses at the current size and that
/// are over a week old: thumbnails of removed or edited wallpapers, other sizes (1.1.0 made
/// 1280px ones), and leftovers from interrupted writes. Runs at most once a day.
pub fn prune(cache_dir: &Path, items: &[Wallpaper]) {
    let dir = cache_dir.join("thumbs");
    let marker = dir.join(".pruned");
    let now = SystemTime::now();
    let age = |p: &Path| fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| now.duration_since(t).ok());
    if age(&marker).is_some_and(|a| a < Duration::from_secs(24 * 3600)) {
        return;
    }
    let width = thumb_size().0;
    let keep: HashSet<String> = items
        .iter()
        .filter_map(|item| {
            let source = Path::new(&item.path);
            fs::metadata(source).ok().map(|meta| format!("{:016x}", cache_key(source, &meta, width)))
        })
        .collect();
    let Ok(entries) = fs::read_dir(&dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let key = name.split('.').next().unwrap_or("");
        if key.len() != 16 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue; // not ours (or the marker)
        }
        let old = age(&path).is_some_and(|a| a > Duration::from_secs(7 * 24 * 3600));
        let stale_tmp = name.ends_with(".tmp") && age(&path).is_some_and(|a| a > Duration::from_secs(3600));
        if (old && !keep.contains(key)) || stale_tmp {
            let _ = fs::remove_file(&path);
        }
    }
    let _ = fs::write(&marker, b"");
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
        let (min_w, min_h) = thumb_size();
        assert!(tw >= min_w && th >= min_h && tw < 3840, "{tw}x{th}");
        assert_eq!(thumbnail(&cache, Path::new(&found[1].path)).unwrap().path, wide.path);

        let small = thumbnail(&cache, Path::new(&found[0].path)).unwrap();
        assert_eq!(image::image_dimensions(&small.path).unwrap(), (800, 600));
        assert_eq!(small.color, "#0a141e");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn thumbnail_size_follows_the_biggest_screen() {
        assert_eq!(thumb_width_for(&[]), 960);
        assert_eq!(thumb_width_for(&[[0, 0, 1920, 1080]]), 960);
        assert_eq!(thumb_width_for(&[[0, 0, 2560, 1440]]), 960);
        assert_eq!(thumb_width_for(&[[0, 0, 1920, 1080], [1920, 0, 5760, 2160]]), 1280);
        assert_eq!(thumb_width_for(&[[0, 0, 5120, 2880]]), 1600);
    }

    #[test]
    fn prune_keeps_current_thumbnails() {
        let dir = std::env::temp_dir().join(format!("wallswitch-prune-{}", std::process::id()));
        let thumbs = dir.join("cache").join("thumbs");
        fs::create_dir_all(&thumbs).unwrap();
        image::RgbImage::from_pixel(64, 36, image::Rgb([1, 2, 3])).save(dir.join("a.jpg")).unwrap();
        let items = scan(&dir);
        let kept = thumbnail(&dir.join("cache"), Path::new(&items[0].path)).unwrap();
        let week_old = SystemTime::now() - Duration::from_secs(8 * 24 * 3600);
        let orphan = thumbs.join("0123456789abcdef.jpg");
        let fresh_orphan = thumbs.join("fedcba9876543210.jpg");
        for f in [&orphan, &fresh_orphan] {
            fs::write(f, b"x").unwrap();
        }
        for f in [&orphan, Path::new(&kept.path)] {
            File::options().write(true).open(f).unwrap().set_modified(week_old).unwrap();
        }
        prune(&dir.join("cache"), &items);
        assert!(!orphan.exists());
        assert!(fresh_orphan.exists(), "recent files stay: they may belong to a wallpaper outside the folder");
        assert!(Path::new(&kept.path).exists());
        fs::remove_dir_all(&dir).ok();
    }
}
