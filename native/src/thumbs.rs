//! Thumbnails: generated/decoded on 3 worker threads, uploaded to the GPU on demand,
//! and evicted when unused. Only the ambient colours survive closing the switcher.

use crate::library;
use crate::render::{self, Fx, Tex};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Condvar, Mutex};

const WORKERS: usize = 3;
/// ponytail: fixed cap; a row shows ~15 cards, a screen of grid tiles ~30.
const MAX_TEXTURES: usize = 48;

pub struct Decoded {
    pub path: String,
    pub result: Result<(Vec<(u32, u32, Vec<u8>)>, String), String>,
}

struct Queue {
    items: Mutex<(VecDeque<String>, bool)>,
    ready: Condvar,
}

pub enum State {
    Ready { tex: Tex, born: f64 },
    Error,
}

pub struct Thumbs {
    queue: Arc<Queue>,
    rx: mpsc::Receiver<Decoded>,
    queued: HashSet<String>,
    pub states: HashMap<String, State>,
    last_used: HashMap<String, u64>,
    frame: u64,
    /// Ambient colours, kept across opens (tiny).
    pub colors: HashMap<String, String>,
}

pub fn key(path: &str) -> String {
    path.replace('/', "\\").to_lowercase()
}

impl Thumbs {
    pub fn new(cache: PathBuf, wake: impl Fn() + Send + Sync + Clone + 'static) -> Self {
        let queue = Arc::new(Queue { items: Mutex::new((VecDeque::new(), false)), ready: Condvar::new() });
        let (tx, rx) = mpsc::channel();
        for i in 0..WORKERS {
            let queue = queue.clone();
            let tx = tx.clone();
            let cache = cache.clone();
            let wake = wake.clone();
            let _ = std::thread::Builder::new().name(format!("thumb-{i}")).spawn(move || loop {
                let path = {
                    let mut guard = queue.items.lock().unwrap();
                    loop {
                        if guard.1 {
                            return;
                        }
                        if let Some(p) = guard.0.pop_front() {
                            break p;
                        }
                        guard = queue.ready.wait(guard).unwrap();
                    }
                };
                let result = decode(&cache, &path);
                if tx.send(Decoded { path, result }).is_err() {
                    return;
                }
                wake();
            });
        }
        Self {
            queue,
            rx,
            queued: HashSet::new(),
            states: HashMap::new(),
            last_used: HashMap::new(),
            frame: 0,
            colors: HashMap::new(),
        }
    }

    pub fn want(&mut self, path: &str, urgent: bool) {
        let k = key(path);
        if self.states.contains_key(&k) {
            return;
        }
        let mut guard = self.queue.items.lock().unwrap();
        if !self.queued.contains(&k) && guard.0.iter().any(|p| key(p) == k) {
            self.queued.insert(k.clone()); // pushed early through a ThumbHandle
        }
        if self.queued.contains(&k) {
            if urgent {
                if let Some(i) = guard.0.iter().position(|p| key(p) == k) {
                    if i > 0 {
                        let p = guard.0.remove(i).unwrap();
                        guard.0.push_front(p);
                    }
                }
            }
            return;
        }
        self.queued.insert(k);
        if urgent {
            guard.0.push_front(path.to_string());
        } else {
            guard.0.push_back(path.to_string());
        }
        self.queue.ready.notify_one();
    }

    /// A Send handle that can queue thumbnails from other threads (used to start
    /// decoding the current wallpaper while the GPU is still initialising).
    pub fn handle(&self) -> ThumbHandle {
        ThumbHandle(self.queue.clone())
    }

    pub fn has(&self, path: &str) -> bool {
        self.states.contains_key(&key(path))
    }

    /// Texture for drawing; marks it used this frame.
    pub fn tex(&mut self, path: &str) -> Option<(Tex, f64)> {
        let k = key(path);
        match self.states.get(&k) {
            Some(State::Ready { tex, born, .. }) => {
                let r = (tex.clone(), *born);
                self.last_used.insert(k, self.frame);
                Some(r)
            }
            _ => None,
        }
    }

    pub fn is_error(&self, path: &str) -> bool {
        matches!(self.states.get(&key(path)), Some(State::Error))
    }


    /// Upload finished decodes. Returns the paths that just became available.
    pub fn pump(&mut self, fx: &Fx, now: f64) -> Vec<String> {
        let mut done = Vec::new();
        while let Ok(d) = self.rx.try_recv() {
            let k = key(&d.path);
            self.queued.remove(&k);
            let state = match d.result {
                Ok((levels, color)) => {
                    self.colors.insert(k.clone(), color.clone());
                    match fx.upload(&levels) {
                        Some(tex) => State::Ready { tex, born: now },
                        None => State::Error,
                    }
                }
                Err(_) => State::Error,
            };
            if let Some(State::Ready { tex, .. }) = self.states.insert(k.clone(), state) {
                tex.destroy();
            }
            self.last_used.insert(k, self.frame);
            done.push(d.path);
        }
        done
    }

    /// Call once per frame after drawing: frees the least recently drawn textures over the cap.
    pub fn end_frame(&mut self) {
        self.frame += 1;
        let ready: Vec<(String, u64)> = self
            .states
            .iter()
            .filter(|(_, s)| matches!(s, State::Ready { .. }))
            .map(|(k, _)| (k.clone(), *self.last_used.get(k).unwrap_or(&0)))
            .collect();
        if ready.len() <= MAX_TEXTURES {
            return;
        }
        let mut ready = ready;
        ready.sort_by_key(|(_, used)| *used);
        for (k, used) in ready.into_iter().take_while(|_| true) {
            if self.states.values().filter(|s| matches!(s, State::Ready { .. })).count() <= MAX_TEXTURES {
                break;
            }
            if used + 2 >= self.frame {
                break; // still on screen
            }
            if let Some(State::Ready { tex, .. }) = self.states.remove(&k) {
                tex.destroy();
            }
            self.last_used.remove(&k);
        }
    }

    /// Frees every texture (called when the window closes; the GL context goes away with it).
    pub fn release(&mut self) {
        for (_, s) in self.states.drain() {
            if let State::Ready { tex, .. } = s {
                tex.destroy();
            }
        }
        self.last_used.clear();
        let mut guard = self.queue.items.lock().unwrap();
        guard.0.clear();
        self.queued.clear();
        while self.rx.try_recv().is_ok() {}
    }
}

#[derive(Clone)]
pub struct ThumbHandle(Arc<Queue>);

impl ThumbHandle {
    pub fn want_first(&self, path: &str) {
        let mut guard = self.0.items.lock().unwrap();
        if !guard.0.iter().any(|p| key(p) == key(path)) {
            guard.0.push_front(path.to_string());
            self.0.ready.notify_one();
        }
    }
}

impl Drop for Thumbs {
    fn drop(&mut self) {
        self.queue.items.lock().unwrap().1 = true;
        self.queue.ready.notify_all();
    }
}

fn decode(cache: &Path, path: &str) -> Result<(Vec<(u32, u32, Vec<u8>)>, String), String> {
    let source = PathBuf::from(path);
    let transcoded = source.file_name().is_some_and(|n| n == "TranscodedWallpaper");
    if !library::is_image(&source) && !transcoded {
        return Err("Not a supported image".into());
    }
    let thumb = library::thumbnail(cache, &source)?;
    let img = image::ImageReader::open(&thumb.path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?
        .to_rgba8();
    Ok((render::mip_chain(img), thumb.color))
}
