import type { Summary, Tree } from "../api";

export interface GraphNode {
  readonly id: string;
  /** Null when a record names this id as a parent but the log does not hold it. */
  readonly summary: Summary | null;
  readonly focus: boolean;
  readonly layer: number;
  readonly x: number;
  readonly y: number;
}

export interface GraphEdge {
  readonly from: string;
  readonly to: string;
  /** Touches the focused record. */
  readonly near: boolean;
}

export interface Layout {
  readonly nodes: readonly GraphNode[];
  readonly edges: readonly GraphEdge[];
  readonly width: number;
  readonly height: number;
  /** Connected records left out to keep the drawing readable. */
  readonly hidden: { readonly earlier: number; readonly later: number };
}

export const NODE = { width: 200, height: 52 } as const;
const GAP = { column: 24, row: 44 } as const;
const MARGIN = 12;
const DEFAULT_LIMIT = 40;

/**
 * Places a record, its ancestors, and its descendants in layers: parents
 * above children, each node one layer below its lowest parent. Within a
 * layer, nodes sit under the average position of their parents.
 */
export class LineageLayout {
  private readonly focus: string;
  private readonly summaries = new Map<string, Summary | null>();
  private readonly parents = new Map<string, readonly string[]>();
  private readonly children = new Map<string, string[]>();
  private readonly earlierTotal: number;
  private readonly laterTotal: number;

  constructor(tree: Tree) {
    this.focus = tree.record.id;
    for (const id of tree.missing) this.summaries.set(id, null);
    for (const summary of [tree.record, ...tree.ancestors, ...tree.descendants]) {
      this.summaries.set(summary.id, summary);
    }
    for (const [id, summary] of this.summaries) {
      const parents = (summary?.parents ?? []).filter((parent) => this.summaries.has(parent));
      this.parents.set(id, parents);
      for (const parent of parents) this.childrenOf(parent).push(id);
    }
    this.earlierTotal = tree.ancestors.length + tree.missing.length;
    this.laterTotal = tree.descendants.length;
  }

  build(limit = DEFAULT_LIMIT): Layout {
    const { kept, earlier, later } = this.nearest(Math.max(1, limit));
    const rows = this.layers(kept);
    const widest = Math.max(...rows.map((row) => row.length));
    const width = 2 * MARGIN + span(widest, NODE.width, GAP.column);
    const height = 2 * MARGIN + span(rows.length, NODE.height, GAP.row);

    const centres = new Map<string, number>();
    const nodes: GraphNode[] = [];
    rows.forEach((row, layer) => {
      const left = (width - span(row.length, NODE.width, GAP.column)) / 2;
      const ordered = this.order(row, kept, centres);
      ordered.forEach((id, column) => {
        const x = left + column * (NODE.width + GAP.column);
        centres.set(id, x + NODE.width / 2);
        nodes.push({
          id,
          summary: this.summaries.get(id) ?? null,
          focus: id === this.focus,
          layer,
          x,
          y: MARGIN + layer * (NODE.height + GAP.row),
        });
      });
    });

    const edges: GraphEdge[] = [];
    for (const id of kept) {
      for (const parent of this.keptParents(id, kept)) {
        edges.push({ from: parent, to: id, near: parent === this.focus || id === this.focus });
      }
    }
    return {
      nodes,
      edges,
      width,
      height,
      hidden: { earlier: this.earlierTotal - earlier, later: this.laterTotal - later },
    };
  }

  /** Breadth-first from the focus: up through parents, down through children. */
  private nearest(limit: number): { kept: Set<string>; earlier: number; later: number } {
    const kept = new Set([this.focus]);
    const queue: [string, "up" | "down"][] = [
      [this.focus, "up"],
      [this.focus, "down"],
    ];
    let earlier = 0;
    let later = 0;
    for (let head = 0; head < queue.length && kept.size < limit; head += 1) {
      const [id, direction] = queue[head]!;
      const next = direction === "up" ? this.parents.get(id) ?? [] : this.children.get(id) ?? [];
      for (const other of next) {
        if (kept.size >= limit) break;
        if (kept.has(other)) continue;
        kept.add(other);
        if (direction === "up") earlier += 1;
        else later += 1;
        queue.push([other, direction]);
      }
    }
    return { kept, earlier, later };
  }

  /** Longest-path layering, resolved in dependency order. */
  private layers(kept: ReadonlySet<string>): string[][] {
    const waiting = new Map<string, number>();
    const ready: string[] = [];
    for (const id of kept) {
      const count = this.keptParents(id, kept).length;
      waiting.set(id, count);
      if (count === 0) ready.push(id);
    }
    const layerOf = new Map<string, number>();
    for (let head = 0; head < ready.length; head += 1) {
      const id = ready[head]!;
      const parentLayers = this.keptParents(id, kept).map((parent) => layerOf.get(parent) ?? 0);
      layerOf.set(id, parentLayers.length === 0 ? 0 : Math.max(...parentLayers) + 1);
      for (const child of this.children.get(id) ?? []) {
        if (!kept.has(child)) continue;
        const left = (waiting.get(child) ?? 1) - 1;
        waiting.set(child, left);
        if (left === 0) ready.push(child);
      }
    }
    const rows: string[][] = [];
    for (const id of kept) {
      // Ids are content hashes of records that include their parents' ids, so
      // a cycle cannot be built. Anything unresolved still gets drawn.
      const layer = layerOf.get(id) ?? 0;
      (rows[layer] ??= []).push(id);
    }
    return rows.filter((row) => row.length > 0);
  }

  /** Sort a row under its parents' average x, then by log position. */
  private order(row: readonly string[], kept: ReadonlySet<string>, centres: ReadonlyMap<string, number>): string[] {
    const keyed = row.map((id) => {
      const placed = this.keptParents(id, kept)
        .map((parent) => centres.get(parent))
        .filter((centre): centre is number => centre !== undefined);
      const centre = placed.length === 0 ? -1 : placed.reduce((sum, value) => sum + value, 0) / placed.length;
      return { id, centre, position: this.summaries.get(id)?.position ?? -1 };
    });
    keyed.sort((a, b) => a.centre - b.centre || a.position - b.position);
    return keyed.map((entry) => entry.id);
  }

  private keptParents(id: string, kept: ReadonlySet<string>): string[] {
    return (this.parents.get(id) ?? []).filter((parent) => kept.has(parent));
  }

  private childrenOf(id: string): string[] {
    let list = this.children.get(id);
    if (list === undefined) {
      list = [];
      this.children.set(id, list);
    }
    return list;
  }
}

function span(count: number, size: number, gap: number): number {
  return count === 0 ? 0 : count * size + (count - 1) * gap;
}
