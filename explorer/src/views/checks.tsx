import { Component, type ReactNode } from "react";
import type { VerifyReport } from "../api";
import { bytes, plural, shortHash } from "../format";

export type CheckState =
  | { readonly phase: "idle" }
  | { readonly phase: "running" }
  | { readonly phase: "done"; readonly report: VerifyReport }
  | { readonly phase: "failed"; readonly message: string };

interface CheckPanelProps {
  readonly check: CheckState;
  readonly onRun: () => void;
}

/** Runs the node's checks on request; re-hashing large attachments takes time. */
export class CheckPanel extends Component<CheckPanelProps> {
  override render(): ReactNode {
    const { check, onRun } = this.props;
    return (
      <>
        <p className="muted">
          Re-verifies the signature, recomputes the inclusion proof, and re-hashes every attachment stored on this
          node.
        </p>
        <button type="button" className="button" disabled={check.phase === "running"} onClick={onRun}>
          Run checks
        </button>
        <div className="check-output" aria-live="polite">
          {this.output()}
        </div>
      </>
    );
  }

  private output(): ReactNode {
    const { check } = this.props;
    switch (check.phase) {
      case "idle":
        return null;
      case "running":
        return <p className="muted">Checking…</p>;
      case "failed":
        return <p className="bad">{`Check failed: ${check.message}`}</p>;
      case "done":
        return <CheckResults report={check.report} />;
    }
  }
}

class CheckResults extends Component<{ readonly report: VerifyReport }> {
  override render(): ReactNode {
    const { report } = this.props;
    return (
      <>
        <dl className="results">
          <dt>Signature</dt>
          <dd className="good">valid for the stated signing key</dd>
          <dt>Inclusion</dt>
          <dd className="good">{`in a tree of ${plural(report.tree_size, "record")} with root ${shortHash(report.root)}`}</dd>
          <dt>Attachments</dt>
          <dd className={attachmentOutcome(report)}>{attachmentText(report)}</dd>
        </dl>
        <p className="muted">
          Passing means the stated key signed these exact bytes and this node's log contains them. It does not mean the
          claims in the payload are true.
        </p>
      </>
    );
  }
}

function attachmentOutcome(report: VerifyReport): "good" | "warn" | "bad" {
  if (report.corrupt.length > 0) return "bad";
  if (report.missing.length > 0) return "warn";
  return "good";
}

function attachmentText(report: VerifyReport): string {
  const total = report.verified.length + report.missing.length + report.corrupt.length;
  if (total === 0) return "none";
  const parts = [`${report.verified.length} of ${total} match (${bytes(report.verified_bytes)})`];
  if (report.missing.length > 0) parts.push(`${report.missing.length} not on this node`);
  if (report.corrupt.length > 0) parts.push(`${report.corrupt.length} do not match`);
  return parts.join(", ");
}
