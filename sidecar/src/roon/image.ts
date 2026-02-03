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
 * Cache entry with timestamp for TTL support
 */
interface CacheEntry<V> {
  value: V;
  timestamp: number;
}

/**
 * LRU (Least Recently Used) cache with TTL support
 * Uses Map's insertion order preservation to track access order
 */
class LRUCache<K, V> {
  private cache = new Map<K, CacheEntry<V>>();
  private maxSize: number;
  private ttlMs: number;

  /**
   * @param maxSize Maximum number of entries
   * @param ttlMs Time-to-live in milliseconds (default: 1 hour)
   */
  constructor(maxSize: number, ttlMs: number = 60 * 60 * 1000) {
    this.maxSize = maxSize;
    this.ttlMs = ttlMs;
  }

  get(key: K): V | undefined {
    const entry = this.cache.get(key);
    if (entry !== undefined) {
      // Check if entry has expired
      if (Date.now() - entry.timestamp > this.ttlMs) {
        this.cache.delete(key);
        return undefined;
      }
      // Move to end (most recently used) by re-inserting
      this.cache.delete(key);
      this.cache.set(key, entry);
      return entry.value;
    }
    return undefined;
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
    this.cache.set(key, { value, timestamp: Date.now() });
  }

  has(key: K): boolean {
    const entry = this.cache.get(key);
    if (entry === undefined) return false;
    // Check TTL
    if (Date.now() - entry.timestamp > this.ttlMs) {
      this.cache.delete(key);
      return false;
    }
    return true;
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

// Time-to-live for cached artwork (1 hour)
// Allows artwork to refresh if user updates metadata in Roon
const CACHE_TTL_MS = 60 * 60 * 1000;

// Timeout for image fetch requests (10 seconds)
const IMAGE_FETCH_TIMEOUT_MS = 10000;

/**
 * Manages artwork fetching and caching
 */
export class ImageManager {
  private imageService: ImageService | null = null;
  private cache = new LRUCache<string, string>(MAX_CACHE_SIZE, CACHE_TTL_MS);

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

      // Set up timeout to prevent hanging requests
      let timeoutId: NodeJS.Timeout | null = setTimeout(() => {
        timeoutId = null;
        reject(new Error(`Image fetch timeout after ${IMAGE_FETCH_TIMEOUT_MS}ms for key: ${imageKey}`));
      }, IMAGE_FETCH_TIMEOUT_MS);

      // Request image with specific size and format
      // scale: 'fit' ensures the image fits within the specified dimensions
      // 64x64 is sufficient for 22pt menu bar icon at 2x Retina (44px actual)
      // Slightly larger to allow for quality during resize
      const options = {
        scale: 'fit',
        width: 64,
        height: 64,
        format: 'image/jpeg',
      };

      this.imageService.get_image(imageKey, options, (error, contentType, body) => {
        // Clear timeout if it hasn't fired yet
        if (timeoutId) {
          clearTimeout(timeoutId);
          timeoutId = null;
        } else {
          // Timeout already fired, ignore this response
          return;
        }

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
