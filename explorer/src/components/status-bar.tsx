import { Component, type ReactNode } from "react";
import type { Status } from "../api";
import { plural, shortHash } from "../format";

interface StatusBarProps {
  /** Null until the first answer, or when the node stopped answering. */
  readonly status: Status | null;
  readonly unreachable: boolean;
}

/** Record and artifact counts and the current root, shown in the header. */
export class StatusBar extends Component<StatusBarProps> {
  override render(): ReactNode {
    const { status, unreachable } = this.props;
    if (unreachable) {
      return (
        <div className="status" aria-live="polite">
          <span className="status-item bad">Node not reachable</span>
        </div>
      );
    }
    if (status === null) return <div className="status" aria-live="polite" />;
    return (
      <div className="status" aria-live="polite">
        <span className="status-item">{plural(status.records, "record")}</span>
        <span className="status-item">{plural(status.artifacts, "artifact")}</span>
        <span className="status-item" title={status.root ? `Root of this node's own log: ${status.root}` : undefined}>
          root <code>{status.root ? shortHash(status.root) : "none"}</code>
        </span>
      </div>
    );
  }
}
