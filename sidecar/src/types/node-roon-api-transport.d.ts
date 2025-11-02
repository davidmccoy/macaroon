/**
 * Type definitions for node-roon-api-transport
 */

declare module 'node-roon-api-transport' {
  interface RoonApiTransport {
    subscribe_zones(callback: (response: string, data: any) => void): void;
    control(zone_or_output_id: string, control: string, options?: any): void;
    seek(zone_or_output_id: string, how: string, seconds?: number): void;
    change_settings(zone_or_output_id: string, settings: any, callback?: (error: any) => void): void;
    change_volume(output_id: string, how: string, value?: number, callback?: (error: any) => void): void;
    mute(output_id: string, how: string, callback?: (error: any) => void): void;
    mute_all(zone_or_output_id: string, how: string, callback?: (error: any) => void): void;
  }

  const transport: RoonApiTransport;
  export = transport;
}
