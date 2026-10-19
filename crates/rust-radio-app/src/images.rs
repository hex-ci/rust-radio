//! 台标与背景图：后台下载 + 磁盘缓存 + 纹理上传；另生成小尺寸模糊图给 Hero 背景用。

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use egui::{ColorImage, Context, TextureHandle, TextureOptions};

type Loaded = (String, Option<ColorImage>, Option<ColorImage>);

pub struct Images {
    dir: PathBuf,
    textures: HashMap<String, TextureHandle>,
    blurred: HashMap<String, TextureHandle>,
    pending: HashSet<String>,
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,
}

impl Images {
    pub fn new(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        let (tx, rx) = mpsc::channel();
        Self {
            dir,
            textures: HashMap::new(),
            blurred: HashMap::new(),
            pending: HashSet::new(),
            tx,
            rx,
        }
    }

    pub fn poll(&mut self, ctx: &Context) {
        let mut got = false;
        while let Ok((url, img, blur)) = self.rx.try_recv() {
            self.pending.remove(&url);
            if let Some(img) = img {
                let tex = ctx.load_texture(format!("img:{url}"), img, TextureOptions::LINEAR);
                self.textures.insert(url.clone(), tex);
            }
            if let Some(b) = blur {
                let tex = ctx.load_texture(format!("blur:{url}"), b, TextureOptions::LINEAR);
                self.blurred.insert(url, tex);
            }
            got = true;
        }
        if got {
            ctx.request_repaint();
        }
    }

    pub fn get(&mut self, ctx: &Context, url: &str) -> Option<TextureHandle> {
        if url.is_empty() {
            return None;
        }
        if let Some(t) = self.textures.get(url) {
            return Some(t.clone());
        }
        self.request(ctx, url);
        None
    }

    pub fn blurred(&mut self, ctx: &Context, url: &str) -> Option<TextureHandle> {
        if url.is_empty() {
            return None;
        }
        if let Some(t) = self.blurred.get(url) {
            return Some(t.clone());
        }
        self.request(ctx, url);
        None
    }

    fn request(&mut self, ctx: &Context, url: &str) {
        if self.pending.contains(url) {
            return;
        }
        self.pending.insert(url.to_string());
        let url = url.to_string();
        let dir = self.dir.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let bytes = load_bytes(&dir, &url);
            let (img, blur) = match bytes {
                Some(b) => decode(&b),
                None => (None, None),
            };
            let _ = tx.send((url, img, blur));
            ctx.request_repaint();
        });
    }
}

fn key(url: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn load_bytes(dir: &std::path::Path, url: &str) -> Option<Vec<u8>> {
    let path = dir.join(key(url));
    if let Ok(b) = std::fs::read(&path) {
        if !b.is_empty() {
            return Some(b);
        }
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) YuntingRadio/0.1")
        .build()
        .ok()?;
    let resp = client.get(url).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().ok()?.to_vec();
    let _ = std::fs::write(&path, &bytes);
    Some(bytes)
}

fn decode(bytes: &[u8]) -> (Option<ColorImage>, Option<ColorImage>) {
    let Ok(img) = image::load_from_memory(bytes) else {
        return (None, None);
    };
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let target = 512u32;
    let scaled = if w > target || h > target {
        image::imageops::resize(&rgba, target, target, image::imageops::FilterType::Triangle)
    } else {
        rgba
    };
    let (sw, sh) = scaled.dimensions();
    let main = ColorImage::from_rgba_unmultiplied([sw as usize, sh as usize], scaled.as_raw());

    // 模糊底图：先缩到 32px，再做两遍盒式模糊，绘制时拉伸 → 廉价的毛玻璃
    let mut small = image::imageops::resize(&scaled, 32, 32, image::imageops::FilterType::Triangle);
    box_blur(&mut small, 2);
    let blur = ColorImage::from_rgba_unmultiplied([32, 32], small.as_raw());
    (Some(main), Some(blur))
}

fn box_blur(img: &mut image::RgbaImage, radius: u32) {
    let (w, h) = img.dimensions();
    let src = img.clone();
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for dy in -(radius as i32)..=(radius as i32) {
                for dx in -(radius as i32)..=(radius as i32) {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let p = src.get_pixel(nx as u32, ny as u32);
                    for (a, v) in acc.iter_mut().zip(p.0) {
                        *a += v as u32;
                    }
                    n += 1;
                }
            }
            let mut out = [0u8; 4];
            for (o, a) in out.iter_mut().zip(acc) {
                *o = (a / n.max(1)) as u8;
            }
            img.put_pixel(x, y, image::Rgba(out));
        }
    }
}
