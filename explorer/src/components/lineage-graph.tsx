import { Component, createRef, type ReactNode } from "react";
import { recordPath, shortId, truncate } from "../format";
import { NODE, type GraphEdge, type GraphNode, type Layout } from "../graph/layout";
import type { Parties } from "../parties";

const ARROW_ID = "lineage-arrow";

interface LineageGraphProps {
  readonly layout: Layout;
  readonly parties: Parties;
}

/** Draws a lineage layout as SVG. Each stored record links to its own page. */
export class LineageGraph extends Component<LineageGraphProps> {
  private readonly frame = createRef<HTMLDivElement>();

  /** Scroll the focused record into the middle of the frame. */
  override componentDidMount(): void {
    const frame = this.frame.current;
    const focus = this.props.layout.nodes.find((node) => node.focus);
    if (!frame || !focus) return;
    frame.scrollTop = Math.max(0, focus.y + NODE.height / 2 - frame.clientHeight / 2);
    frame.scrollLeft = Math.max(0, focus.x + NODE.width / 2 - frame.clientWidth / 2);
  }

  override render(): ReactNode {
    const { layout, parties } = this.props;
    const positions = new Map(layout.nodes.map((node) => [node.id, node]));
    return (
      <div className="graph" ref={this.frame}>
        <svg
          className="graph-drawing"
          width={layout.width}
          height={layout.height}
          viewBox={`0 0 ${layout.width} ${layout.height}`}
          role="group"
          aria-label="Records connected to this one. Parents are above children."
        >
          <defs>
            <marker id={ARROW_ID} viewBox="0 0 8 8" refX={7} refY={4} markerWidth={7} markerHeight={7} orient="auto">
              <path d="M0,0 L8,4 L0,8 z" className="graph-arrow" />
            </marker>
          </defs>
          {layout.edges.map((edge) => (
            <EdgePath key={`${edge.from}>${edge.to}`} edge={edge} positions={positions} />
          ))}
          {layout.nodes.map((node) => (
            <NodeBox key={node.id} node={node} parties={parties} />
          ))}
        </svg>
      </div>
    );
  }
}

interface EdgePathProps {
  readonly edge: GraphEdge;
  readonly positions: ReadonlyMap<string, GraphNode>;
}

/** A curve from the bottom of the parent to the top of the child. */
class EdgePath extends Component<EdgePathProps> {
  override render(): ReactNode {
    const { edge, positions } = this.props;
    const from = positions.get(edge.from);
    const to = positions.get(edge.to);
    if (!from || !to) return null;
    const x1 = from.x + NODE.width / 2;
    const y1 = from.y + NODE.height;
    const x2 = to.x + NODE.width / 2;
    const y2 = to.y - 2;
    const bend = (y2 - y1) / 2;
    return (
      <path
        d={`M${x1},${y1} C${x1},${y1 + bend} ${x2},${y2 - bend} ${x2},${y2}`}
        className={edge.near ? "graph-edge near" : "graph-edge"}
        markerEnd={`url(#${ARROW_ID})`}
      />
    );
  }
}

interface NodeBoxProps {
  readonly node: GraphNode;
  readonly parties: Parties;
}

class NodeBox extends Component<NodeBoxProps> {
  override render(): ReactNode {
    const { node, parties } = this.props;
    const { x, y } = node;
    const box = <rect x={x} y={y} width={NODE.width} height={NODE.height} rx={6} className="graph-box" />;
    if (node.summary === null) {
      return (
        <g className="graph-node missing">
          <title>{`${node.id}\nNamed as a parent but not stored in this log.`}</title>
          {box}
          <text x={x + 12} y={y + 22} className="graph-label">
            Not in this log
          </text>
          <text x={x + 12} y={y + 40} className="graph-kind">
            {shortId(node.id)}
          </text>
        </g>
      );
    }
    const { id, label, kind, signer } = node.summary;
    return (
      <a
        href={recordPath(id)}
        className={node.focus ? "graph-node focus" : "graph-node"}
        aria-current={node.focus ? "page" : undefined}
      >
        <title>{`${label}\n${kind}\n${id}`}</title>
        {box}
        <text x={x + 12} y={y + 22} className="graph-label">
          {truncate(label, 24)}
        </text>
        <circle cx={x + 16} cy={y + 36} r={4} className={`graph-dot ${parties.className(signer)}`} />
        <text x={x + 26} y={y + 40} className="graph-kind">
          {truncate(kind, 26)}
        </text>
      </a>
    );
  }
}
