import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** A mounted React tree in a detached container, for tests running under jsdom. */
export class Mounted {
  readonly container: HTMLElement = document.createElement("div");
  private readonly root: Root = createRoot(this.container);

  static async render(node: ReactNode): Promise<Mounted> {
    const mounted = new Mounted();
    document.body.append(mounted.container);
    await act(async () => mounted.root.render(node));
    return mounted;
  }

  find(selector: string): HTMLElement {
    const found = this.container.querySelector<HTMLElement>(selector);
    if (!found) throw new Error(`nothing matches ${selector}`);
    return found;
  }

  all<E extends Element = HTMLElement>(selector: string): E[] {
    return [...this.container.querySelectorAll<E>(selector)];
  }

  /** Click the button whose text is exactly `label`. */
  async press(label: string): Promise<void> {
    const button = this.all("button").find((candidate) => candidate.textContent === label);
    if (!button) throw new Error(`no button labelled ${label}`);
    await act(async () => button.click());
  }

  async submit(selector: string): Promise<void> {
    await act(async () => this.find(selector).dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  }

  async unmount(): Promise<void> {
    await act(async () => this.root.unmount());
    this.container.remove();
  }
}
