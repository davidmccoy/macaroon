/**
 * Type definitions for node-roon-api
 */

declare module 'node-roon-api' {
  interface RoonApiOptions {
    extension_id: string;
    display_name: string;
    display_version: string;
    publisher: string;
    email: string;
    website: string;
    core_paired?: (core: any) => void;
    core_unpaired?: (core: any) => void;
    log_level?: 'none' | 'all';
  }

  interface RoonApiServiceOptions {
    required_services?: any[];
    provided_services?: any[];
    optional_services?: any[];
  }

  interface WsConnectOptions {
    host: string;
    port: number;
    onclose?: () => void;
  }

  class RoonApi {
    constructor(options: RoonApiOptions);
    init_services(options: RoonApiServiceOptions): void;
    start_discovery(): void;
    ws_connect(options: WsConnectOptions): void;
    save_config(key: string, value: any): void;
    load_config(key: string): any;
  }

  export = RoonApi;
}
