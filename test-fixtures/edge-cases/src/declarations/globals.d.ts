declare module "*.css" {
  const content: Record<string, string>;
  export default content;
}

declare module "*.svg" {
  const content: string;
  export default content;
}

declare global {
  interface Window {
    __APP_VERSION__: string;
  }

  function __DEV_LOG__(message: string): void;
}

export {};
