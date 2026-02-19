export function trackEvent(event: string, data?: Record<string, unknown>): void {
  console.log(`[Analytics] ${event}`, data);
}

export function trackPageView(page: string): void {
  trackEvent("page_view", { page });
}

export function trackError(error: Error): void {
  trackEvent("error", { message: error.message, stack: error.stack });
}
