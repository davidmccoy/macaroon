/**
 * Type definitions for node-roon-api-image
 */

declare module 'node-roon-api-image' {
  interface ImageOptions {
    scale?: 'fit' | 'fill' | 'stretch';
    width?: number;
    height?: number;
    format?: string;
  }

  interface RoonApiImage {
    get_image(
      image_key: string,
      options: ImageOptions,
      callback: (error: any, contentType: string, body: Buffer) => void
    ): void;
  }

  const image: RoonApiImage;
  export = image;
}
