import { Component, type ReactNode } from "react";
import type { Api, ArtifactStatus, VerifyReport } from "../api";
import { bytes, shortId } from "../format";

interface AttachmentTableProps {
  readonly api: Api;
  readonly artifacts: readonly ArtifactStatus[];
  /** Results of the last check, when one has run. */
  readonly report: VerifyReport | null;
}

interface Result {
  readonly className: string;
  readonly text: string;
  readonly detail?: string;
}

/** A record's attachments. The last column fills in once checks run. */
export class AttachmentTable extends Component<AttachmentTableProps> {
  override render(): ReactNode {
    const { api, artifacts } = this.props;
    if (artifacts.length === 0) return <p className="muted">No attachments.</p>;
    return (
      <div className="table-wrap">
        <table>
          <thead>
            <tr>
              {["Name", "Size", "Type", "Content id", "Bytes", ""].map((heading, column) => (
                <th key={column} scope="col">
                  {heading}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {artifacts.map(({ reference, available }) => {
              const result = this.result(reference.name, available);
              return (
                <tr key={reference.name}>
                  <td>{reference.name}</td>
                  <td className="num nowrap">{bytes(reference.size)}</td>
                  <td className="kind">{reference.media ?? "unspecified"}</td>
                  <td>
                    <code className="id" title={reference.blob}>
                      {shortId(reference.blob)}
                    </code>
                  </td>
                  <td className={result.className} title={result.detail}>
                    {result.text}
                  </td>
                  <td>
                    {available ? (
                      <a href={api.artifactUrl(reference.blob)} download={reference.name}>
                        Download
                      </a>
                    ) : null}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    );
  }

  private result(name: string, available: boolean): Result {
    const { report } = this.props;
    if (report === null) {
      return { className: "muted", text: available ? "on this node, not checked" : "not on this node" };
    }
    if (report.verified.includes(name)) return { className: "good", text: "match the content id" };
    const corrupt = report.corrupt.find(([corruptName]) => corruptName === name);
    if (corrupt) return { className: "bad", text: "do not match", detail: corrupt[1] };
    return { className: "warn", text: "not on this node" };
  }
}
