use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use gio::prelude::{AppInfoExt, FileExt, IconExt};
use gio::DesktopAppInfo;
use resvg::tiny_skia;
use resvg::usvg;
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

    texture_from_icon_path(renderer, &path, scale, desired_size)
}

fn texture_from_icon_path(
    renderer: &mut GlesRenderer,
    path: &Path,
    scale: f64,
    desired_size: i32,
) -> anyhow::Result<AppIconTextureBuffer> {
    let argb = match path.extension().and_then(|ext| ext.to_str()) {
        Some("svg") => argb_from_svg_path(path, desired_size)?,
        Some("png") => argb_from_png_path(path, desired_size)?,
        Some("xpm") => anyhow::bail!("unsupported icon format in {}", path.display()),
        _ => anyhow::bail!("unsupported icon format in {}", path.display()),
    };

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

fn argb_from_png_path(path: &Path, desired_size: i32) -> anyhow::Result<Vec<u8>> {
    let bytes = fs::read(path)?;
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info()?;

    let out_size = reader.output_buffer_size().ok_or_else(|| {
        anyhow::anyhow!("failed to determine PNG output size for {}", path.display())
    })?;
    let mut out = vec![0; out_size];
    let info = reader.next_frame(&mut out)?;
    let data = &out[..info.buffer_size()];

    let mut rgba = Vec::with_capacity((info.width * info.height * 4) as usize);
    match info.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(data),
        png::ColorType::Rgb => {
            for px in data.chunks_exact(3) {
                rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
        }
        _ => anyhow::bail!("unsupported icon color format in {}", path.display()),
    }

    resize_rgba_to_argb8888(
        &rgba,
        info.width as i32,
        info.height as i32,
        desired_size,
        desired_size,
    )
}

fn argb_from_svg_path(path: &Path, desired_size: i32) -> anyhow::Result<Vec<u8>> {
    let svg = fs::read(path)?;
    let tree = usvg::Tree::from_data(&svg, &usvg::Options::default())?;

    let svg_size = tree.size();
    let svg_width = f64::from(svg_size.width());
    let svg_height = f64::from(svg_size.height());
    anyhow::ensure!(svg_width > 0. && svg_height > 0.);

    let scale = f64::min(
        desired_size as f64 / svg_width,
        desired_size as f64 / svg_height,
    );
    let rendered_width = (svg_width * scale).round().max(1.) as u32;
    let rendered_height = (svg_height * scale).round().max(1.) as u32;

    let mut pixmap = tiny_skia::Pixmap::new(rendered_width, rendered_height)
        .ok_or_else(|| anyhow::anyhow!("failed to allocate svg pixmap"))?;

    let transform = tiny_skia::Transform::from_scale(scale as f32, scale as f32);
    let mut pixmap_mut = pixmap.as_mut();
    resvg::render(&tree, transform, &mut pixmap_mut);

    rgba_to_padded_argb8888(
        pixmap.data(),
        rendered_width as i32,
        rendered_height as i32,
        desired_size,
        desired_size,
    )
}

fn resize_rgba_to_argb8888(
    src: &[u8],
    src_width: i32,
    src_height: i32,
    dst_width: i32,
    dst_height: i32,
) -> anyhow::Result<Vec<u8>> {
    if src_width <= 0 || src_height <= 0 || dst_width <= 0 || dst_height <= 0 {
        anyhow::bail!("invalid icon dimensions");
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

            let r = u16::from(src[src_idx]);
            let g = u16::from(src[src_idx + 1]);
            let b = u16::from(src[src_idx + 2]);
            let a = u16::from(src[src_idx + 3]);

            let dst_x = x + x_offset;
            let dst_y = y + y_offset;
            let dst_idx = ((dst_y * dst_width + dst_x) * 4) as usize;

            dst[dst_idx] = ((b * a + 127) / 255) as u8;
            dst[dst_idx + 1] = ((g * a + 127) / 255) as u8;
            dst[dst_idx + 2] = ((r * a + 127) / 255) as u8;
            dst[dst_idx + 3] = a as u8;
        }
    }

    Ok(dst)
}

fn rgba_to_padded_argb8888(
    src: &[u8],
    src_width: i32,
    src_height: i32,
    dst_width: i32,
    dst_height: i32,
) -> anyhow::Result<Vec<u8>> {
    if src_width <= 0 || src_height <= 0 || dst_width <= 0 || dst_height <= 0 {
        anyhow::bail!("invalid icon dimensions");
    }

    let x_offset = (dst_width - src_width) / 2;
    let y_offset = (dst_height - src_height) / 2;
    let mut dst = vec![0; (dst_width * dst_height * 4) as usize];

    for y in 0..src_height {
        for x in 0..src_width {
            let src_idx = ((y * src_width + x) * 4) as usize;
            let r = src[src_idx + 2] as u16;
            let g = src[src_idx + 1] as u16;
            let b = src[src_idx] as u16;
            let a = src[src_idx + 3] as u16;

            let dst_x = x + x_offset;
            let dst_y = y + y_offset;
            let dst_idx = ((dst_y * dst_width + dst_x) * 4) as usize;

            dst[dst_idx] = b as u8;
            dst[dst_idx + 1] = g as u8;
            dst[dst_idx + 2] = r as u8;
            dst[dst_idx + 3] = a as u8;
        }
    }

    Ok(dst)
}

static APP_ICON_NAMES_CACHE: LazyLock<Mutex<HashMap<String, Vec<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static ICON_PATH_CACHE: LazyLock<Mutex<HashMap<(String, i32), Option<PathBuf>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static INSTALLED_APP_ICONS: LazyLock<Vec<(String, Vec<String>)>> = LazyLock::new(|| {
    gio::AppInfo::all()
        .into_iter()
        .filter_map(|app_info| {
            let app_id = app_info.id()?.to_string();
            let icon_names = icon_names_from_app_info(&app_info);
            Some((app_id, icon_names))
        })
        .collect()
});

fn find_icon_path_for_app_id(app_id: &str, desired_size: i32) -> Option<PathBuf> {
    let icon_names = app_icon_names_for_app_id(app_id);
    icon_names
        .into_iter()
        .find_map(|icon_name| find_icon_path_for_name(&icon_name, desired_size))
}

fn app_icon_names_for_app_id(app_id: &str) -> Vec<String> {
    if let Some(cached) = APP_ICON_NAMES_CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.get(app_id).cloned())
    {
        return cached;
    }

    let icon_names = app_icon_names_for_app_id_uncached(app_id);

    if let Ok(mut cache) = APP_ICON_NAMES_CACHE.lock() {
        cache.insert(app_id.to_owned(), icon_names.clone());
    }

    icon_names
}

fn app_icon_names_for_app_id_uncached(app_id: &str) -> Vec<String> {
    // Mirror niri-switch's lookup approach: try exact desktop IDs from the AppInfo cache first.
    let direct_ids = [format!("{app_id}.desktop"), app_id.to_owned()];
    for requested in direct_ids {
        if let Some(icon_names) = installed_app_icon_names_for_desktop_id(&requested) {
            return icon_names;
        }
    }

    let matches = DesktopAppInfo::search(app_id);
    let Some(best_matches) = matches.first() else {
        return fallback_icon_names_from_app_id(app_id);
    };
    let Some(best_match) = best_matches.iter().min_by_key(|id| id.len()) else {
        return fallback_icon_names_from_app_id(app_id);
    };

    if let Some(icon_names) = installed_app_icon_names_for_desktop_id(best_match.as_str()) {
        return icon_names;
    }

    let Some(info) = DesktopAppInfo::new(best_match.as_str()) else {
        return fallback_icon_names_from_app_id(app_id);
    };

    let mut names = icon_names_from_desktop_app_info(&info);
    if names.is_empty() {
        names = fallback_icon_names_from_app_id(app_id);
    }
    names
}

fn fallback_icon_names_from_app_id(app_id: &str) -> Vec<String> {
    let mut names = Vec::new();

    let app_id = app_id.trim();
    if app_id.is_empty() {
        return names;
    }

    names.push(app_id.to_owned());

    if let Some(stripped) = app_id.strip_suffix(".desktop") {
        names.push(stripped.to_owned());
    }

    if let Some(last) = app_id.split('.').next_back() {
        if !last.is_empty() {
            names.push(last.to_owned());
        }
    }

    names.push(app_id.replace('.', "-"));
    names.push(app_id.replace('.', "_"));

    names.sort();
    names.dedup();
    names
}

fn installed_app_icon_names_for_desktop_id(desktop_id: &str) -> Option<Vec<String>> {
    INSTALLED_APP_ICONS
        .iter()
        .find(|(app_id, _)| app_id == desktop_id)
        .map(|(_, names)| names.clone())
}

fn icon_names_from_desktop_app_info(app_info: &DesktopAppInfo) -> Vec<String> {
    let Some(icon) = app_info.icon() else {
        return Vec::new();
    };

    icon_names_from_icon_string(icon.to_string().as_deref())
}

fn icon_names_from_app_info(app_info: &gio::AppInfo) -> Vec<String> {
    let Some(icon) = app_info.icon() else {
        return Vec::new();
    };

    icon_names_from_icon_string(icon.to_string().as_deref())
}

fn icon_names_from_icon_string(icon_name: Option<&str>) -> Vec<String> {
    let Some(icon_name) = icon_name else {
        return Vec::new();
    };
    let icon_name = icon_name.trim();
    if icon_name.is_empty() {
        return Vec::new();
    }

    let mut names = Vec::new();

    if let Some(path) = icon_name_path(icon_name) {
        names.push(path.to_string_lossy().to_string());
    }

    names.push(icon_name.to_owned());
    for part in icon_name.split([' ', ',']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        names.push(part.to_owned());
    }

    names.sort();
    names.dedup();
    names
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
    if let Some(path) = icon_name_path(icon_name) {
        if path.exists() {
            return Some(path);
        }
    }

    let dirs = xdg_data_dirs();
    let mut candidates = Vec::new();

    for dir in &dirs {
        add_pixmap_candidates(&mut candidates, dir, icon_name);
    }

    let preferred_themes = preferred_icon_themes();
    for base in &dirs {
        for theme in &preferred_themes {
            add_theme_candidates(&mut candidates, base, theme, icon_name);
        }

        add_theme_candidates(&mut candidates, base, "hicolor", icon_name);

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
                add_theme_candidates(&mut candidates, base, &name, icon_name);
            }
        }
    }

    candidates.sort_by_key(|path| icon_candidate_score(path, desired_size));
    candidates.into_iter().find(|path| path.exists())
}

fn icon_name_path(icon_name: &str) -> Option<PathBuf> {
    let path_like = Path::new(icon_name);
    if path_like.is_absolute() {
        return Some(path_like.to_path_buf());
    }

    if icon_name.starts_with("file://") {
        return gio::File::for_uri(icon_name).path();
    }

    None
}

fn add_pixmap_candidates(candidates: &mut Vec<PathBuf>, base: &Path, icon_name: &str) {
    let pixmaps_dir = base.join("pixmaps");
    for ext in ["png", "svg", "xpm"] {
        candidates.push(pixmaps_dir.join(format!("{icon_name}.{ext}")));
    }
}

fn add_theme_candidates(candidates: &mut Vec<PathBuf>, base: &Path, theme: &str, icon_name: &str) {
    let theme_dir = base.join("icons").join(theme);
    collect_icon_candidates(&theme_dir, icon_name, 0, candidates);
}

fn collect_icon_candidates(
    dir: &Path,
    icon_name: &str,
    depth: u8,
    candidates: &mut Vec<PathBuf>,
) {
    if depth > 4 {
        return;
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        if file_type.is_dir() {
            collect_icon_candidates(&path, icon_name, depth + 1, candidates);
            continue;
        }

        if !(file_type.is_file() || file_type.is_symlink()) {
            continue;
        }

        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if stem != icon_name {
            continue;
        }

        let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
            continue;
        };
        if !matches!(ext, "png" | "svg" | "xpm") {
            continue;
        }

        candidates.push(path);
    }
}

fn icon_candidate_score(path: &Path, desired_size: i32) -> (i32, i32, i32, usize) {
    let category_rank = if path.components().any(|component| component.as_os_str() == "apps") {
        0
    } else {
        1
    };

    let size_rank = match path_size_hint(path) {
        Some(size) => (size - desired_size).abs(),
        None if path.components().any(|component| component.as_os_str() == "scalable") => 8,
        None => 512,
    };

    let format_rank = match path.extension().and_then(|ext| ext.to_str()) {
        Some("png") => 0,
        Some("svg") => 1,
        Some("xpm") => 2,
        _ => 3,
    };

    (category_rank, size_rank, format_rank, path.as_os_str().len())
}

fn path_size_hint(path: &Path) -> Option<i32> {
    for component in path.components() {
        let component = component.as_os_str().to_str()?;
        let component = component.strip_suffix("@2x").unwrap_or(component);
        let (width, height) = component.split_once('x')?;
        let width = width.parse::<i32>().ok()?;
        let height = height.parse::<i32>().ok()?;
        if width == height {
            return Some(width);
        }
    }

    None
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