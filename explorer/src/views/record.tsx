import { Component, type ReactNode } from "react";
import { messageOf, type Api, type Proof, type RecordDetail, type Tree } from "../api";
import { LineageGraph } from "../components/lineage-graph";
import { Loader } from "../components/loader";
import { CopyButton, Fact, Section, SignerChip, Title } from "../components/parts";
import { plural, timestamp } from "../format";
import { LineageLayout } from "../graph/layout";
import type { Parties } from "../parties";
import { AttachmentTable } from "./attachments";
import { CheckPanel, type CheckState } from "./checks";

interface RecordViewProps {
  readonly api: Api;
  readonly parties: Parties;
  readonly id: string;
}

interface Loaded {
  readonly detail: RecordDetail;
  readonly tree: Tree;
  readonly proof: Proof;
}

/** One record: what it states, how it connects, and what can be checked. */
export class RecordView extends Component<RecordViewProps> {
  override render(): ReactNode {
    const { api, parties } = this.props;
    return (
      <div className="view">
        <Loader load={this.load}>{(loaded) => <RecordPage api={api} parties={parties} {...loaded} />}</Loader>
      </div>
    );
  }

  private readonly load = async (): Promise<Loaded> => {
    const { api, id } = this.props;
    const [detail, tree, proof] = await Promise.all([api.record(id), api.tree(id), api.proof(id)]);
    return { detail, tree, proof };
  };
}

interface RecordPageProps extends Loaded {
  readonly api: Api;
  readonly parties: Parties;
}

interface RecordPageState {
  readonly check: CheckState;
}

export class RecordPage extends Component<RecordPageProps, RecordPageState> {
  override state: RecordPageState = { check: { phase: "idle" } };
  private mounted = false;

  override componentDidMount(): void {
    this.mounted = true;
  }

  override componentWillUnmount(): void {
    this.mounted = false;
  }

  override render(): ReactNode {
    const { api, parties, detail, tree, proof } = this.props;
    const { check } = this.state;
    return (
      <>
        <Title text={tree.record.label} />
        <header className="record-head">
          <p className="crumbs">
            <a href="/">History</a>
            {` / #${detail.position}`}
          </p>
          <h1>{tree.record.label}</h1>
          <p className="kind">{detail.signed.record.kind}</p>
        </header>
        <RecordFacts detail={detail} tree={tree} proof={proof} parties={parties} />
        <LineageSection tree={tree} parties={parties} />
        <Section heading="Payload">
          <pre className="json">{JSON.stringify(detail.signed.record.payload, null, 2)}</pre>
        </Section>
        <Section heading="Attachments">
          <AttachmentTable api={api} artifacts={detail.artifacts} report={check.phase === "done" ? check.report : null} />
        </Section>
        <Section heading="Checks">
          <CheckPanel check={check} onRun={this.runChecks} />
        </Section>
        <ProofSection proof={proof} />
        <SignedBytes detail={detail} />
      </>
    );
  }

  private readonly runChecks = async (): Promise<void> => {
    this.setState({ check: { phase: "running" } });
    let check: CheckState;
    try {
      check = { phase: "done", report: await this.props.api.verify(this.props.detail.id) };
    } catch (error) {
      check = { phase: "failed", message: messageOf(error) };
    }
    if (this.mounted) this.setState({ check });
  };
}

interface RecordFactsProps {
  readonly detail: RecordDetail;
  readonly tree: Tree;
  readonly proof: Proof;
  readonly parties: Parties;
}

class RecordFacts extends Component<RecordFactsProps> {
  override render(): ReactNode {
    const { detail, tree, proof, parties } = this.props;
    const record = detail.signed.record;
    const parents = record.parents ?? [];
    const absent = parents.filter((parent) => tree.missing.includes(parent)).length;
    return (
      <dl className="facts panel">
        <Fact term="Record id">
          <code className="id wrap">{detail.id}</code> <CopyButton text={detail.id} />
        </Fact>
        <Fact term="Signed by">
          <SignerChip parties={parties} signer={record.signer} />{" "}
          <span className="muted">{parties.isOwn(record.signer) ? "this node's key" : "not this node's key"}</span>
        </Fact>
        <Fact term="Created">
          {timestamp(record.created)} <span className="muted">as stated by the signer</span>
        </Fact>
        <Fact term="Position">{`#${detail.position} of ${proof.tree_size} in this log`}</Fact>
        <Fact term="Parents">{absent > 0 ? `${parents.length} (${absent} not in this log)` : parents.length}</Fact>
        <Fact term="Children">{detail.children.length}</Fact>
      </dl>
    );
  }
}

interface LineageSectionProps {
  readonly tree: Tree;
  readonly parties: Parties;
}

class LineageSection extends Component<LineageSectionProps> {
  override render(): ReactNode {
    const { tree, parties } = this.props;
    if (tree.ancestors.length + tree.descendants.length + tree.missing.length === 0) {
      return (
        <Section heading="Lineage">
          <p className="muted">No parents or children in this log.</p>
        </Section>
      );
    }
    const layout = new LineageLayout(tree).build();
    const { earlier, later } = layout.hidden;
    return (
      <Section heading="Lineage">
        <p className="muted">
          {`${plural(tree.ancestors.length, "earlier record")} and ${plural(tree.descendants.length, "later record")} connect to this one. Parents are drawn above children.`}
        </p>
        <LineageGraph layout={layout} parties={parties} />
        {earlier + later > 0 ? (
          <p className="muted">{`Showing the ${layout.nodes.length} closest. ${earlier} earlier and ${later} later records are not drawn.`}</p>
        ) : null}
      </Section>
    );
  }
}

class ProofSection extends Component<{ readonly proof: Proof }> {
  override render(): ReactNode {
    const { proof } = this.props;
    return (
      <Section heading="Inclusion proof">
        <p className="muted">
          Recomputed against this node's current root. It shows this log contains the record. It cannot show the log
          was never rewritten until someone outside this machine keeps a copy of the root.
        </p>
        <dl className="facts">
          <Fact term="Leaf">{`${proof.leaf_index} of ${proof.tree_size}`}</Fact>
          <Fact term="Leaf hash">
            <code className="hash">{proof.leaf_hash}</code>
          </Fact>
          <Fact term="Root">
            <code className="hash">{proof.root}</code>
          </Fact>
          <Fact term="Path">
            {proof.path.length === 0 ? (
              "empty; this is the only record"
            ) : (
              <ol className="path">
                {proof.path.map((hash, index) => (
                  <li key={index}>
                    <code className="hash">{hash}</code>
                  </li>
                ))}
              </ol>
            )}
          </Fact>
        </dl>
      </Section>
    );
  }
}

/** The canonical text, not a re-serialization: JSON.parse rounds integers above 2^53. */
class SignedBytes extends Component<{ readonly detail: RecordDetail }> {
  override render(): ReactNode {
    const { detail } = this.props;
    const envelope = `{"record":${detail.canonical},"sig":"${detail.signed.sig}"}`;
    return (
      <Section heading="Signed bytes">
        <p className="muted">
          The exact text the signature covers. The envelope adds the signature; another node accepts it unchanged at
          POST /api/records.
        </p>
        <details>
          <summary>Show canonical record</summary>
          <pre className="json wrap">{detail.canonical}</pre>
        </details>
        <div className="actions">
          <CopyButton text={detail.canonical} label="Copy canonical record" />
          <CopyButton text={envelope} label="Copy envelope" />
        </div>
      </Section>
    );
  }
}
