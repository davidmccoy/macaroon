use anyhow::{Context, Result};
use image::{Rgba, RgbaImage};
use imageproc::drawing::draw_text_mut;
use ab_glyph::{FontRef, PxScale};

/// Detect if macOS is in dark mode using defaults command (safer than Cocoa APIs)
#[cfg(target_os = "macos")]
fn is_dark_mode() -> bool {
    use std::process::Command;

    // Use the `defaults read` command to check system appearance
    // This is safer than calling Cocoa APIs directly
    match Command::new("defaults")
        .args(&["read", "-g", "AppleInterfaceStyle"])
        .output()
    {
        Ok(output) => {
            let result = String::from_utf8_lossy(&output.stdout);
            let is_dark = result.trim() == "Dark";
            log::debug!("Dark mode detection (via defaults): {}", is_dark);
            is_dark
        }
        Err(e) => {
            // If the command fails (e.g., key doesn't exist in light mode), assume light mode
            log::debug!("Dark mode detection failed: {}, assuming light mode", e);
            false
        }
    }
}

/// Default to light mode on non-macOS platforms
#[cfg(not(target_os = "macos"))]
fn is_dark_mode() -> bool {
    false
}

/// Get appropriate text color based on system appearance
fn get_text_color() -> Rgba<u8> {
    if is_dark_mode() {
        Rgba([255, 255, 255, 255]) // White text for dark mode
    } else {
        Rgba([0, 0, 0, 255]) // Black text for light mode
    }
}

pub struct Compositor {
    font: Vec<u8>,
}

impl Compositor {
    pub fn new() -> Result<Self> {
        // Load SF Pro Text (macOS native system font) from system font directory
        // SF Pro is the standard system font for macOS since macOS 10.11
        let font_path = "/System/Library/Fonts/SFNS.ttf";

        let font_data = std::fs::read(font_path)
            .context("Failed to load SF Pro system font. Ensure running on macOS.")?;

        Ok(Self { font: font_data })
    }

    /// Create a menu bar icon with album art and text
    /// Returns PNG bytes
    pub fn create_menu_bar_icon(
        &self,
        album_art_base64: Option<&str>,
        title: &str,
        artist: &str,
    ) -> Result<Vec<u8>> {
        // Render at 3x resolution for Retina displays for sharper text
        const SCALE_FACTOR: u32 = 3;
        const MAX_CANVAS_WIDTH: u32 = 500 * SCALE_FACTOR;
        const MIN_CANVAS_WIDTH: u32 = 22 * SCALE_FACTOR;  // Just artwork
        const CANVAS_HEIGHT: u32 = 22 * SCALE_FACTOR;
        const ALBUM_ART_SIZE: u32 = 22 * SCALE_FACTOR;
        const TEXT_X_OFFSET: i32 = 28 * SCALE_FACTOR as i32;
        const RIGHT_PADDING: u32 = 3 * SCALE_FACTOR;  // Small buffer for glyph overhang

        // Calculate dynamic canvas width based on text length
        let canvas_width = if !title.is_empty() || !artist.is_empty() {
            let text = format!("{} - {}", title, artist);
            let scale = PxScale::from(63.0);
            let text_width = self.measure_text_width(&text, scale);

            // Width = album art + spacing + text + padding
            // Use ceiling to ensure we have enough space for the full measured width
            let required_width = ALBUM_ART_SIZE + (TEXT_X_OFFSET as u32 - ALBUM_ART_SIZE) + text_width.ceil() as u32 + RIGHT_PADDING;

            // Cap at maximum width
            let final_width = required_width.min(MAX_CANVAS_WIDTH);

            log::debug!(
                "Canvas sizing: text='{}', text_width={:.1}px, required={}, max={}, final={}",
                text, text_width, required_width, MAX_CANVAS_WIDTH, final_width
            );

            final_width
        } else {
            // No text - just show artwork
            MIN_CANVAS_WIDTH
        };

        // Create transparent canvas with dynamic width
        let mut canvas = RgbaImage::from_pixel(
            canvas_width,
            CANVAS_HEIGHT,
            Rgba([0, 0, 0, 0])
        );

        // Draw album art or placeholder
        if let Some(artwork_data) = album_art_base64 {
            if let Ok(art_image) = self.decode_and_resize_artwork(artwork_data, ALBUM_ART_SIZE) {
                self.overlay_image(&mut canvas, &art_image, 0, 0);
            } else {
                // Fallback to colored square if artwork fails
                self.draw_placeholder_art(&mut canvas, ALBUM_ART_SIZE);
            }
        } else {
            // No artwork provided - draw placeholder
            self.draw_placeholder_art(&mut canvas, ALBUM_ART_SIZE);
        }

        // Only draw text if we have title or artist
        if !title.is_empty() || !artist.is_empty() {
            // Prepare text: "Title - Artist"
            let text = format!("{} - {}", title, artist);
            let available_width = (canvas_width - TEXT_X_OFFSET as u32 - RIGHT_PADDING) as i32;
            let display_text = self.truncate_text(&text, available_width);

            // Draw text at 3x scale for Retina
            // 63px at 3x = 21px at 1x - matching original Helvetica Neue size
            let scale = PxScale::from(63.0);

            // Get text color based on macOS appearance (dark/light mode)
            let text_color = get_text_color();

            // Load font for rendering
            let font = FontRef::try_from_slice(&self.font)
                .context("Failed to parse font data")?;

            // Position text vertically - scaled for 3x resolution
            // At 3x: 3px offset = 1px at 1x (matching original positioning)
            let text_y = 3;

            draw_text_mut(
                &mut canvas,
                text_color,
                TEXT_X_OFFSET,
                text_y,
                scale,
                &font,
                &display_text,
            );
        }

        // Encode as PNG
        self.encode_png(&canvas)
    }

    /// Decode base64 artwork and resize to target size
    fn decode_and_resize_artwork(&self, artwork_data: &str, size: u32) -> Result<RgbaImage> {
        // Strip data URL prefix if present
        let base64_data = if artwork_data.starts_with("data:") {
            artwork_data
                .split(',')
                .nth(1)
                .context("Invalid data URL format")?
        } else {
            artwork_data
        };

        // Decode base64
        use base64::Engine;
        let image_bytes = base64::engine::general_purpose::STANDARD
            .decode(base64_data)
            .context("Failed to decode base64 artwork")?;

        // Load and resize image
        let img = image::load_from_memory(&image_bytes)
            .context("Failed to load image from memory")?;

        let resized = img.resize_exact(size, size, image::imageops::FilterType::Lanczos3);

        Ok(resized.to_rgba8())
    }

    /// Overlay one image onto another at specified position
    fn overlay_image(&self, canvas: &mut RgbaImage, overlay: &RgbaImage, x: i64, y: i64) {
        image::imageops::overlay(canvas, overlay, x, y);
    }

    /// Draw a placeholder colored square when no artwork is available
    fn draw_placeholder_art(&self, canvas: &mut RgbaImage, size: u32) {
        // Draw a purple square as placeholder
        let placeholder_color = Rgba([147, 51, 234, 255]); // Purple

        for py in 0..size {
            for px in 0..size {
                canvas.put_pixel(px, py, placeholder_color);
            }
        }
    }

    /// Truncate text to fit within available width
    fn truncate_text(&self, text: &str, max_width: i32) -> String {
        // Use same scale as rendering (63px at 3x = 21px at 1x)
        let scale = PxScale::from(63.0);

        // Measure full text width
        let full_width = self.measure_text_width(text, scale);

        if full_width <= max_width as f32 {
            return text.to_string();
        }

        // Truncate with ellipsis
        let ellipsis = "...";
        let ellipsis_width = self.measure_text_width(ellipsis, scale);
        let available_for_text = max_width as f32 - ellipsis_width;

        let mut truncated = String::new();
        for ch in text.chars() {
            let test_str = format!("{}{}", truncated, ch);
            let width = self.measure_text_width(&test_str, scale);

            if width > available_for_text {
                break;
            }
            truncated.push(ch);
        }

        format!("{}{}", truncated, ellipsis)
    }

    /// Measure the width of text in pixels
    fn measure_text_width(&self, text: &str, scale: PxScale) -> f32 {
        use ab_glyph::{Font, ScaleFont};

        // Parse font for measurement
        let font = match FontRef::try_from_slice(&self.font) {
            Ok(f) => f,
            Err(_) => return 0.0,
        };

        let scaled_font = font.as_scaled(scale);
        let mut width = 0.0;

        for ch in text.chars() {
            let glyph_id = font.glyph_id(ch);
            width += scaled_font.h_advance(glyph_id);
        }

        width
    }

    /// Encode image as PNG bytes
    fn encode_png(&self, image: &RgbaImage) -> Result<Vec<u8>> {
        use image::codecs::png::PngEncoder;
        use image::ImageEncoder;

        let mut buffer = Vec::new();
        let encoder = PngEncoder::new(&mut buffer);

        encoder
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgba8,
            )
            .context("Failed to encode PNG")?;

        Ok(buffer)
    }
}

/// Create a test icon with fake data (for Phase 0 testing)
pub fn create_test_icon() -> Result<Vec<u8>> {
    let compositor = Compositor::new()?;

    compositor.create_menu_bar_icon(
        None, // No artwork for now - will show purple square
        "Test Song Title",
        "Test Artist Name",
    )
}
