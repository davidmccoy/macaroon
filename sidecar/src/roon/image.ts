/**
 * Image/artwork fetching module
 *
 * Fetches album artwork from Roon and converts to base64 data URLs
 */

import * as output from '../output.js';

export interface ImageService {
  get_image: (image_key: string, options: any, callback: (error: any, contentType: string, body: Buffer) => void) => void;
}

/**
 * Simple LRU (Least Recently Used) cache implementation
 * Uses Map's insertion order preservation to track access order
 */
class LRUCache<K, V> {
  private cache = new Map<K, V>();
  private maxSize: number;

  constructor(maxSize: number) {
    this.maxSize = maxSize;
  }

  get(key: K): V | undefined {
    const value = this.cache.get(key);
    if (value !== undefined) {
      // Move to end (most recently used) by re-inserting
      this.cache.delete(key);
      this.cache.set(key, value);
    }
    return value;
  }

  set(key: K, value: V): void {
    // If key exists, delete it first (will be re-added at end)
    if (this.cache.has(key)) {
      this.cache.delete(key);
    } else if (this.cache.size >= this.maxSize) {
      // Cache is full, delete oldest (first) entry
      const firstKey = this.cache.keys().next().value;
      if (firstKey !== undefined) {
        this.cache.delete(firstKey);
      }
    }
    this.cache.set(key, value);
  }

  has(key: K): boolean {
    return this.cache.has(key);
  }

  clear(): void {
    this.cache.clear();
  }

  get size(): number {
    return this.cache.size;
  }
}

// Maximum number of artwork entries to cache
// At ~100KB per image, 100 entries = ~10MB max memory usage
const MAX_CACHE_SIZE = 100;

/**
 * Manages artwork fetching and caching
 */
export class ImageManager {
  private imageService: ImageService | null = null;
  private cache = new LRUCache<string, string>(MAX_CACHE_SIZE);

  /**
   * Set the Roon image service (called after core connects)
   */
  setImageService(service: ImageService): void {
    this.imageService = service;
    output.debug('Image service initialized');
  }

  /**
   * Clear the image service and cache (called on disconnect)
   * Cache is cleared because image keys are specific to a Roon core
   */
  clearImageService(): void {
    this.imageService = null;
    this.clearCache();
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
    const cached = this.cache.get(imageKey);
    if (cached) {
      output.debug(`Using cached artwork for ${imageKey}`);
      return cached;
    }

    if (!this.imageService) {
      output.warn('Image service not available, cannot fetch artwork');
      return undefined;
    }

    try {
      const dataUrl = await this.getImageAsDataUrl(imageKey);

      // Cache the result (LRU will evict oldest if at capacity)
      this.cache.set(imageKey, dataUrl);

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
   */
  clearCache(): void {
    const cacheSize = this.cache.size;
    this.cache.clear();
    output.debug(`Cleared artwork cache (${cacheSize} items)`);
  }

  /**
   * Get current cache size
   */
  getCacheSize(): number {
    return this.cache.size;
  }
}
