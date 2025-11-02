/**
 * Image/artwork fetching module
 *
 * Fetches album artwork from Roon and converts to base64 data URLs
 */

import * as output from '../output.js';

export interface ImageService {
  get_image: (image_key: string, options: any, callback: (error: any, contentType: string, body: Buffer) => void) => void;
}

interface ArtworkCache {
  [imageKey: string]: string; // base64 data URL
}

/**
 * Manages artwork fetching and caching
 */
export class ImageManager {
  private imageService: ImageService | null = null;
  private cache: ArtworkCache = {};

  /**
   * Set the Roon image service (called after core connects)
   */
  setImageService(service: ImageService): void {
    this.imageService = service;
    output.debug('Image service initialized');
  }

  /**
   * Clear the image service (called on disconnect)
   */
  clearImageService(): void {
    this.imageService = null;
    output.debug('Image service cleared');
  }

  /**
   * Fetch artwork for a given image key and convert to base64 data URL
   * Returns cached version if available
   */
  async fetchArtwork(imageKey: string | null | undefined): Promise<string | undefined> {
    if (!imageKey) {
      output.debug('No image key provided');
      return undefined;
    }

    // Check cache first
    if (this.cache[imageKey]) {
      output.debug(`Using cached artwork for ${imageKey}`);
      return this.cache[imageKey];
    }

    if (!this.imageService) {
      output.warn('Image service not available, cannot fetch artwork');
      return undefined;
    }

    try {
      const dataUrl = await this.getImageAsDataUrl(imageKey);

      // Cache the result
      this.cache[imageKey] = dataUrl;

      output.debug(`Fetched and cached artwork for ${imageKey}`);
      return dataUrl;
    } catch (error) {
      output.error('Failed to fetch artwork:', error);
      return undefined;
    }
  }

  /**
   * Get image from Roon and convert to base64 data URL
   */
  private getImageAsDataUrl(imageKey: string): Promise<string> {
    return new Promise((resolve, reject) => {
      if (!this.imageService) {
        reject(new Error('Image service not available'));
        return;
      }

      // Request image with specific size and format
      // scale: 'fit' ensures the image fits within the specified dimensions
      // width/height: request reasonably sized image (not thumbnail, not huge)
      const options = {
        scale: 'fit',
        width: 300,
        height: 300,
        format: 'image/jpeg',
      };

      this.imageService.get_image(imageKey, options, (error, contentType, body) => {
        if (error) {
          reject(error);
          return;
        }

        try {
          // Convert buffer to base64
          const base64 = body.toString('base64');

          // Determine the content type (use provided or default to JPEG)
          const mimeType = contentType || 'image/jpeg';

          // Create data URL
          const dataUrl = `data:${mimeType};base64,${base64}`;

          resolve(dataUrl);
        } catch (err) {
          reject(err);
        }
      });
    });
  }

  /**
   * Clear the artwork cache
   * Useful to prevent memory leaks during long-running sessions
   */
  clearCache(): void {
    const cacheSize = Object.keys(this.cache).length;
    this.cache = {};
    output.debug(`Cleared artwork cache (${cacheSize} items)`);
  }

  /**
   * Get cache statistics
   */
  getCacheSize(): number {
    return Object.keys(this.cache).length;
  }
}
