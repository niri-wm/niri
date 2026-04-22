use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use gio::prelude::{AppInfoExt, IconExt};
use gio::DesktopAppInfo;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::utils::Transform;

use crate::render_helpers::texture::TextureBuffer;
use crate::utils::round_logical_in_physical;

pub const APP_ICON_SIZE: f64 = 20.;

pub type AppIconTextureBuffer = TextureBuffer<GlesTexture>;

#[derive(Debug, Default)]
pub struct AppIconTexture {
    app_id: Option<String>,
    scale: f64,
    texture: Option<Option<AppIconTextureBuffer>>,
}

impl AppIconTexture {
    pub fn get(
        &mut self,
        renderer: &mut GlesRenderer,
        app_id: Option<&str>,
        scale: f64,
    ) -> Option<AppIconTextureBuffer> {
        let app_id_owned = app_id.map(str::to_owned);

        if self.app_id != app_id_owned || self.scale != scale {
            self.texture = None;
            self.app_id = app_id_owned;
            self.scale = scale;
        }

        self.texture
            .get_or_insert_with(|| {
                app_id.and_then(|id| generate_app_icon_texture(renderer, id, scale).ok())
            })
            .clone()
    }

    pub fn get_stale(&self) -> Option<&AppIconTextureBuffer> {
        if let Some(Some(texture)) = &self.texture {
            Some(texture)
        } else {
            None
        }
    }
}

fn generate_app_icon_texture(
    renderer: &mut GlesRenderer,
    app_id: &str,
    scale: f64,
) -> anyhow::Result<AppIconTextureBuffer> {
    let _span = tracy_client::span!("mru::generate_app_icon_texture");

    let desired_size = round_logical_in_physical(scale, APP_ICON_SIZE).round() as i32;
    let desired_size = desired_size.max(1);

    let path = find_icon_path_for_app_id(app_id, desired_size)
        .ok_or_else(|| anyhow::anyhow!("icon not found for app_id {app_id}"))?;

    texture_from_png_path(renderer, &path, scale, desired_size)
}

fn texture_from_png_path(
    renderer: &mut GlesRenderer,
    path: &Path,
    scale: f64,
    desired_size: i32,
) -> anyhow::Result<AppIconTextureBuffer> {
    let bytes = fs::read(path)?;
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info()?;

    let out_size = reader.output_buffer_size().ok_or_else(|| {
        anyhow::anyhow!("failed to determine PNG output size for {}", path.display())
    })?;
    let mut out = vec![0; out_size];
    let info = reader.next_frame(&mut out)?;
    let data = &out[..info.buffer_size()];

    let mut argb = Vec::with_capacity((info.width * info.height * 4) as usize);
    match info.color_type {
        png::ColorType::Rgba => {
            for px in data.chunks_exact(4) {
                let r = u16::from(px[0]);
                let g = u16::from(px[1]);
                let b = u16::from(px[2]);
                let a = u16::from(px[3]);

                let r = ((r * a + 127) / 255) as u8;
                let g = ((g * a + 127) / 255) as u8;
                let b = ((b * a + 127) / 255) as u8;

                argb.push(b);
                argb.push(g);
                argb.push(r);
                argb.push(a as u8);
            }
        }
        png::ColorType::Rgb => {
            for px in data.chunks_exact(3) {
                argb.push(px[2]);
                argb.push(px[1]);
                argb.push(px[0]);
                argb.push(255);
            }
        }
        _ => anyhow::bail!("unsupported icon color format in {}", path.display()),
    }

    let argb = resize_icon_argb8888(
        &argb,
        info.width as i32,
        info.height as i32,
        desired_size,
        desired_size,
    );

    let buffer = TextureBuffer::from_memory(
        renderer,
        &argb,
        Fourcc::Argb8888,
        (desired_size, desired_size),
        false,
        scale,
        Transform::Normal,
        Vec::new(),
    )?;

    Ok(buffer)
}

fn resize_icon_argb8888(
    src: &[u8],
    src_width: i32,
    src_height: i32,
    dst_width: i32,
    dst_height: i32,
) -> Vec<u8> {
    if src_width <= 0 || src_height <= 0 || dst_width <= 0 || dst_height <= 0 {
        return Vec::new();
    }

    if src_width == dst_width && src_height == dst_height {
        return src.to_vec();
    }

    let scale = f64::min(
        dst_width as f64 / src_width as f64,
        dst_height as f64 / src_height as f64,
    );
    let scaled_width = ((src_width as f64 * scale).round() as i32).max(1);
    let scaled_height = ((src_height as f64 * scale).round() as i32).max(1);
    let x_offset = (dst_width - scaled_width) / 2;
    let y_offset = (dst_height - scaled_height) / 2;

    let mut dst = vec![0; (dst_width * dst_height * 4) as usize];

    for y in 0..scaled_height {
        let src_y = ((y as i64 * src_height as i64) / scaled_height as i64) as i32;
        for x in 0..scaled_width {
            let src_x = ((x as i64 * src_width as i64) / scaled_width as i64) as i32;

            let src_idx = ((src_y * src_width + src_x) * 4) as usize;
            let dst_x = x + x_offset;
            let dst_y = y + y_offset;
            let dst_idx = ((dst_y * dst_width + dst_x) * 4) as usize;

            dst[dst_idx..dst_idx + 4].copy_from_slice(&src[src_idx..src_idx + 4]);
        }
    }

    dst
}

static APP_ICON_NAME_CACHE: LazyLock<Mutex<HashMap<String, Option<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static ICON_PATH_CACHE: LazyLock<Mutex<HashMap<(String, i32), Option<PathBuf>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn find_icon_path_for_app_id(app_id: &str, desired_size: i32) -> Option<PathBuf> {
    let icon_name = desktop_icon_name_for_app_id(app_id)?;
    find_icon_path_for_name(&icon_name, desired_size)
}

fn desktop_icon_name_for_app_id(app_id: &str) -> Option<String> {
    if let Some(cached) = APP_ICON_NAME_CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.get(app_id).cloned())
    {
        return cached;
    }

    let app_info = desktop_app_info_for_app_id(app_id);
    let icon_name = app_info.and_then(|info| {
        let icon = info.icon()?;
        icon.to_string().map(|name| name.to_string())
    });

    if let Ok(mut cache) = APP_ICON_NAME_CACHE.lock() {
        cache.insert(app_id.to_owned(), icon_name.clone());
    }

    icon_name
}

fn desktop_app_info_for_app_id(app_id: &str) -> Option<DesktopAppInfo> {
    let requested = format!("{app_id}.desktop");
    if let Some(info) = DesktopAppInfo::new(&requested) {
        return Some(info);
    }

    let matches = DesktopAppInfo::search(app_id);
    let best_matches = matches.first()?;
    let best_match = best_matches.iter().min_by_key(|id| id.len())?;
    DesktopAppInfo::new(best_match.as_str())
}

fn find_icon_path_for_name(icon_name: &str, desired_size: i32) -> Option<PathBuf> {
    let key = (icon_name.to_owned(), desired_size);
    if let Some(cached) = ICON_PATH_CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.get(&key).cloned())
    {
        return cached;
    }

    let found = find_icon_path_for_name_uncached(icon_name, desired_size);
    if let Ok(mut cache) = ICON_PATH_CACHE.lock() {
        cache.insert(key, found.clone());
    }

    found
}

fn find_icon_path_for_name_uncached(icon_name: &str, desired_size: i32) -> Option<PathBuf> {
    let path_like = Path::new(icon_name);
    if path_like.is_absolute() && path_like.exists() {
        return Some(path_like.to_path_buf());
    }

    let dirs = xdg_data_dirs();
    let mut candidates = Vec::new();

    for dir in &dirs {
        candidates.push(dir.join("pixmaps").join(format!("{icon_name}.png")));
    }

    let preferred_themes = preferred_icon_themes();
    for base in &dirs {
        for theme in &preferred_themes {
            add_theme_candidates(&mut candidates, base, theme, icon_name, desired_size);
        }

        add_theme_candidates(&mut candidates, base, "hicolor", icon_name, desired_size);

        let icons_dir = base.join("icons");
        if let Ok(themes) = fs::read_dir(icons_dir) {
            for theme in themes.flatten() {
                let Ok(ft) = theme.file_type() else {
                    continue;
                };
                if !ft.is_dir() {
                    continue;
                }
                let name = theme.file_name();
                let name = name.to_string_lossy();
                if preferred_themes.iter().any(|x| x == &name) || name == "hicolor" {
                    continue;
                }
                add_theme_candidates(&mut candidates, base, &name, icon_name, desired_size);
            }
        }
    }

    candidates.into_iter().find(|path| path.exists())
}

fn add_theme_candidates(
    candidates: &mut Vec<PathBuf>,
    base: &Path,
    theme: &str,
    icon_name: &str,
    desired_size: i32,
) {
    for size in icon_size_candidates(desired_size) {
        let dir = format!("{size}x{size}");
        candidates.push(
            base.join("icons")
                .join(theme)
                .join(&dir)
                .join("apps")
                .join(format!("{icon_name}.png")),
        );
    }
}

fn icon_size_candidates(desired_size: i32) -> Vec<i32> {
    let mut sizes = vec![16, 22, 24, 32, 48, 64, 96, 128, 192, 256];
    sizes.sort_by_key(|size| (desired_size - size).abs());
    sizes
}

fn preferred_icon_themes() -> Vec<String> {
    let mut rv = Vec::new();

    if let Ok(theme) = env::var("XDG_ICON_THEME") {
        if !theme.is_empty() {
            rv.push(theme);
        }
    }

    if let Ok(theme) = env::var("GTK_THEME") {
        let theme = theme.split(':').next().unwrap_or_default().to_owned();
        if !theme.is_empty() && !rv.iter().any(|x| x == &theme) {
            rv.push(theme);
        }
    }

    rv
}

fn xdg_data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(path) = env::var_os("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(path));
    } else if let Some(home) = env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share"));
    }

    let data_dirs = env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());

    dirs.extend(data_dirs.split(':').map(PathBuf::from));
    dirs
}