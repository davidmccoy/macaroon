use anyhow::{anyhow, Context, Result};
use image::{Rgba, RgbaImage};
use imageproc::drawing::draw_text_mut;
use ab_glyph::{Font, FontVec, PxScale, ScaleFont};

/// Maximum dimensions for decoded images (prevent OOM attacks)
const MAX_IMAGE_DIMENSION: u32 = 4096;

/// Get appropriate text color based on system appearance
/// Uses the dark-light crate which properly caches and uses native APIs
fn get_text_color() -> Rgba<u8> {
    match dark_light::detect() {
        dark_light::Mode::Dark => Rgba([255, 255, 255, 255]), // White text for dark mode
        dark_light::Mode::Light | dark_light::Mode::Default => Rgba([0, 0, 0, 255]), // Black text for light mode
    }
}

pub struct Compositor {
    font: FontVec,
}

impl Compositor {
    pub fn new() -> Result<Self> {
        // Load SF Pro Text (macOS native system font) from system font directory
        // SF Pro is the standard system font for macOS since macOS 10.11
        let font_path = "/System/Library/Fonts/SFNS.ttf";

        let font_data = std::fs::read(font_path)
            .context("Failed to load SF Pro system font. Ensure running on macOS.")?;

        // Parse font once at construction time, cache for reuse
        let font = FontVec::try_from_vec(font_data)
            .map_err(|_| anyhow!("Failed to parse SF Pro font data"))?;

        Ok(Self { font })
    }

    /// Extract the primary (first) artist from a potentially multi-artist string
    /// Roon sends multiple artists separated by " / ", but we only want to show the first
    fn get_primary_artist(artist: &str) -> &str {
        let first = artist.split(" / ").next().unwrap_or(artist);
        // Handle edge case where artist is " / Something" or just " / "
        let trimmed = first.trim();
        if trimmed.is_empty() {
            // Try to get the second part if first was empty
            artist.split(" / ").nth(1).map(str::trim).filter(|s| !s.is_empty()).unwrap_or(artist.trim())
        } else {
            trimmed
        }
    }

    /// Format display text from title and artist, handling empty cases
    fn format_display_text(title: &str, artist: &str) -> String {
        let primary_artist = Self::get_primary_artist(artist);
        let title_trimmed = title.trim();
        let artist_trimmed = primary_artist.trim();

        match (title_trimmed.is_empty(), artist_trimmed.is_empty()) {
            (true, true) => String::new(),
            (true, false) => artist_trimmed.to_string(),
            (false, true) => title_trimmed.to_string(),
            (false, false) => format!("{} - {}", title_trimmed, artist_trimmed),
        }
    }

    /// Create a menu bar icon with album art and text
    /// Returns PNG bytes
    pub fn create_menu_bar_icon(
        &self,
        album_art_base64: Option<&str>,
        title: &str,
        artist: &str,
    ) -> Result<Vec<u8>> {
        // Render at 3x resolution for sharp Retina text
        const SCALE_FACTOR: u32 = 3;
        // Menu bar height is 22pt
        const MENU_BAR_HEIGHT_PT: u32 = 22;
        const MAX_CANVAS_WIDTH: u32 = 500 * SCALE_FACTOR;
        const MIN_CANVAS_WIDTH: u32 = MENU_BAR_HEIGHT_PT * SCALE_FACTOR;
        const CANVAS_HEIGHT: u32 = MENU_BAR_HEIGHT_PT * SCALE_FACTOR;
        const ALBUM_ART_SIZE: u32 = MENU_BAR_HEIGHT_PT * SCALE_FACTOR;
        // Gap between album art and text: 10pt
        const TEXT_GAP_PT: u32 = 10;
        const TEXT_X_OFFSET: i32 = ((MENU_BAR_HEIGHT_PT + TEXT_GAP_PT) * SCALE_FACTOR) as i32;
        const RIGHT_PADDING: u32 = 3 * SCALE_FACTOR; // Small buffer for glyph overhang
        // Font size: 21pt at 3x = 63px (original size)
        const FONT_SIZE_PX: f32 = 63.0;

        // Format display text handling empty title/artist properly
        let text = Self::format_display_text(title, artist);

        // Calculate dynamic canvas width based on text length
        let canvas_width = if !text.is_empty() {
            let scale = PxScale::from(FONT_SIZE_PX);
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

        // Only draw text if we have something to display
        if !text.is_empty() {
            let available_width = (canvas_width - TEXT_X_OFFSET as u32 - RIGHT_PADDING) as i32;
            let scale = PxScale::from(FONT_SIZE_PX);
            let display_text = self.truncate_text(&text, available_width, scale);

            // Get text color based on macOS appearance (dark/light mode)
            let text_color = get_text_color();

            // Position text vertically - small offset from top at 3x scale
            const TEXT_Y_OFFSET: i32 = 3;

            draw_text_mut(
                &mut canvas,
                text_color,
                TEXT_X_OFFSET,
                TEXT_Y_OFFSET,
                scale,
                &self.font,
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

        // Check for empty base64 data
        if base64_data.trim().is_empty() {
            return Err(anyhow!("Empty base64 data in artwork"));
        }

        // Decode base64
        use base64::Engine;
        let image_bytes = base64::engine::general_purpose::STANDARD
            .decode(base64_data)
            .context("Failed to decode base64 artwork")?;

        // Load image
        let img = image::load_from_memory(&image_bytes)
            .context("Failed to load image from memory")?;

        // Validate image dimensions to prevent OOM attacks
        if img.width() > MAX_IMAGE_DIMENSION || img.height() > MAX_IMAGE_DIMENSION {
            return Err(anyhow!(
                "Image dimensions {}x{} exceed maximum allowed {}x{}",
                img.width(), img.height(), MAX_IMAGE_DIMENSION, MAX_IMAGE_DIMENSION
            ));
        }

        // Resize using Triangle filter (bilinear) - faster than Lanczos3 and
        // quality difference is imperceptible at 22x22 target size
        let resized = img.resize_exact(size, size, image::imageops::FilterType::Triangle);

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
    /// Uses O(n) algorithm by measuring individual glyph advances
    fn truncate_text(&self, text: &str, max_width: i32, scale: PxScale) -> String {
        let scaled_font = self.font.as_scaled(scale);

        // First pass: measure full text width
        let full_width: f32 = text.chars()
            .map(|ch| scaled_font.h_advance(self.font.glyph_id(ch)))
            .sum();

        if full_width <= max_width as f32 {
            return text.to_string();
        }

        // Calculate ellipsis width
        let ellipsis = "...";
        let ellipsis_width: f32 = ellipsis.chars()
            .map(|ch| scaled_font.h_advance(self.font.glyph_id(ch)))
            .sum();

        // Add small safety margin for glyph overhang
        let available_for_text = (max_width as f32 - ellipsis_width - 2.0).max(0.0);

        // If even ellipsis doesn't fit, return empty string
        if available_for_text <= 0.0 {
            return String::new();
        }

        // Second pass: truncate incrementally (O(n) - measure each char once)
        let mut truncated = String::new();
        let mut current_width = 0.0;

        for ch in text.chars() {
            let glyph_id = self.font.glyph_id(ch);
            let char_width = scaled_font.h_advance(glyph_id);

            if current_width + char_width > available_for_text {
                break;
            }

            current_width += char_width;
            truncated.push(ch);
        }

        // Only add ellipsis if we actually truncated something
        if truncated.len() < text.len() && !truncated.is_empty() {
            format!("{}{}", truncated, ellipsis)
        } else if truncated.is_empty() {
            ellipsis.to_string()
        } else {
            truncated
        }
    }

    /// Measure the width of text in pixels
    fn measure_text_width(&self, text: &str, scale: PxScale) -> f32 {
        let scaled_font = self.font.as_scaled(scale);

        text.chars()
            .map(|ch| scaled_font.h_advance(self.font.glyph_id(ch)))
            .sum()
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

/// Create a test icon with fake data (for development testing)
#[allow(dead_code)]
pub fn create_test_icon() -> Result<Vec<u8>> {
    let compositor = Compositor::new()?;

    compositor.create_menu_bar_icon(
        None, // No artwork - will show purple placeholder
        "Test Song Title",
        "Test Artist Name",
    )
}
