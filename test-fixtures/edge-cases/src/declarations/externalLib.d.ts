declare module "custom-analytics" {
  export function track(event: string, data?: Record<string, unknown>): void;
  export function identify(userId: string, traits?: Record<string, unknown>): void;
  export function page(name: string): void;
}
