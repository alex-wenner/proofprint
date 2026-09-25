import { Component, type ReactNode } from "react";
import { Title } from "../components/parts";

export class MissingView extends Component<{ readonly path: string }> {
  override render(): ReactNode {
    return (
      <div className="view">
        <Title text="Not found" />
        <h1>Nothing here</h1>
        <p className="muted">
          No page at <code>{this.props.path}</code>. <a href="/">Back to the history</a>.
        </p>
      </div>
    );
  }
}
