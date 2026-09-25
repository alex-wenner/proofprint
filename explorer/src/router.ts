export type Route =
  | { readonly name: "history"; readonly query: URLSearchParams }
  | { readonly name: "record"; readonly id: string }
  | { readonly name: "unknown"; readonly path: string };

/**
 * History-API routing. Same-origin links are followed without a page load;
 * links into /api/, downloads, and modified clicks are left to the browser.
 */
export class Router {
  private listener: ((route: Route) => void) | null = null;
  private readonly onPopState = (): void => this.emit();
  private readonly onClick = (event: MouseEvent): void => this.follow(event);

  static parse(pathname: string, search: string): Route {
    if (pathname === "/" || pathname === "") {
      return { name: "history", query: new URLSearchParams(search) };
    }
    const match = /^\/records\/([^/]+)\/?$/.exec(pathname);
    if (match?.[1] !== undefined) {
      try {
        return { name: "record", id: decodeURIComponent(match[1]) };
      } catch {
        return { name: "unknown", path: pathname };
      }
    }
    return { name: "unknown", path: pathname };
  }

  static current(): Route {
    return Router.parse(location.pathname, location.search);
  }

  /** Report every later navigation to `listener`. The current route is not reported. */
  start(listener: (route: Route) => void): void {
    this.stop();
    this.listener = listener;
    window.addEventListener("popstate", this.onPopState);
    document.addEventListener("click", this.onClick);
  }

  stop(): void {
    this.listener = null;
    window.removeEventListener("popstate", this.onPopState);
    document.removeEventListener("click", this.onClick);
  }

  navigate(path: string): void {
    if (path !== location.pathname + location.search) history.pushState(null, "", path);
    this.emit();
  }

  private emit(): void {
    this.listener?.(Router.current());
  }

  private follow(event: MouseEvent): void {
    if (event.defaultPrevented || event.button !== 0) return;
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    // `closest("a")` also matches SVG <a> elements, which the lineage graph uses.
    const anchor = event.target instanceof Element ? event.target.closest("a") : null;
    const href = anchor?.getAttribute("href");
    if (!anchor || !href || anchor.hasAttribute("download") || anchor.getAttribute("target")) return;
    const url = new URL(href, location.href);
    if (url.origin !== location.origin || url.pathname.startsWith("/api/")) return;
    event.preventDefault();
    this.navigate(url.pathname + url.search);
  }
}
