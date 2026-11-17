/// <reference path="./fyx-ui-types.d.ts" />
declare module 'fyx:ui/host@0.1.0' {
  /**
   * Extension-initiated update (timers, background work). The host coalesces pushes and
   * applies them on the next frame; it never calls back into the extension while rendering.
   */
  export function pushUpdate(surfaceId: string, update: Update): void;
  export function log(level: LogLevel, message: string): void;
  export type Update = import('fyx:ui/types@0.1.0').Update;
  /**
   * # Variants
   * 
   * ## `"trace"`
   * 
   * ## `"debug"`
   * 
   * ## `"info"`
   * 
   * ## `"warn"`
   * 
   * ## `"error"`
   */
  export type LogLevel = "trace" | "debug" | "info" | "warn" | "error";
}
