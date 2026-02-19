// Router that uses dynamic imports for lazy loading
type RouteHandler = () => Promise<void>;

export class Router {
  private routes: Map<string, () => Promise<RouteHandler>> = new Map();

  constructor() {
    // Lazy route registration using dynamic imports
    this.routes.set("/users", async () => {
      const mod = await import("./lazy/usersPage");
      return mod.handleUsers;
    });

    this.routes.set("/posts", async () => {
      const mod = await import("./lazy/postsPage");
      return mod.handlePosts;
    });

    this.routes.set("/settings", async () => {
      const mod = await import("./lazy/settingsPage");
      return mod.handleSettings;
    });
  }

  async getHandler(path: string): Promise<RouteHandler | null> {
    const loader = this.routes.get(path);
    if (!loader) return null;
    return loader();
  }
}
