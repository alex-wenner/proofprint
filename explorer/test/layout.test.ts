import { describe, expect, it } from "vitest";
import { LineageLayout, NODE, type Layout } from "../src/graph/layout";
import { id, summary, tree } from "./fixtures";

function layerOf(layout: Layout, name: string): number {
  const node = layout.nodes.find((candidate) => candidate.id === id(name));
  if (!node) throw new Error(`${name} is not in the layout`);
  return node.layer;
}

function row(layout: Layout, layer: number): string[] {
  return layout.nodes
    .filter((node) => node.layer === layer)
    .sort((a, b) => a.x - b.x)
    .map((node) => node.summary?.label ?? "missing");
}

describe("LineageLayout", () => {
  const base = summary("b", 0);
  const left = summary("l", 1, ["b"]);
  const right = summary("r", 2, ["b"]);
  const merge = summary("m", 3, ["l", "r"]);

  it("puts parents above children, one layer below the lowest parent", () => {
    const layout = new LineageLayout(tree(merge, [left, right, base])).build();
    expect(layerOf(layout, "b")).toBe(0);
    expect(layerOf(layout, "l")).toBe(1);
    expect(layerOf(layout, "r")).toBe(1);
    expect(layerOf(layout, "m")).toBe(2);
    expect(row(layout, 1)).toEqual(["l", "r"]);
    expect(layout.edges).toHaveLength(4);
    expect(layout.edges.filter((edge) => edge.near)).toHaveLength(2);
  });

  it("draws only edges whose ends are both in the tree", () => {
    // From "l": its ancestor is "b", its descendant is "m". "r" is neither.
    const layout = new LineageLayout(tree(left, [base], [merge])).build();
    expect(layout.nodes.map((node) => node.summary?.label)).toEqual(["b", "l", "m"]);
    expect(layout.edges.map((edge) => [edge.from, edge.to])).toEqual([
      [id("b"), id("l")],
      [id("l"), id("m")],
    ]);
    expect(layout.nodes.find((node) => node.focus)?.id).toBe(id("l"));
  });

  it("places a layer under the average of its parents, not in log order", () => {
    const a = summary("a", 0);
    const b = summary("b", 1);
    const x = summary("x", 2, ["b"]);
    const y = summary("y", 3, ["a"]);
    const z = summary("z", 4, ["x", "y"]);
    const layout = new LineageLayout(tree(z, [x, y, a, b])).build();
    expect(row(layout, 0)).toEqual(["a", "b"]);
    expect(row(layout, 1)).toEqual(["y", "x"]);
  });

  it("draws parents the log does not hold as placeholders", () => {
    const orphan = summary("o", 0, ["g"]);
    const layout = new LineageLayout(tree(orphan, [], [], ["g"])).build();
    const ghost = layout.nodes.find((node) => node.id === id("g"));
    expect(ghost?.summary).toBeNull();
    expect(ghost?.layer).toBe(0);
    expect(layerOf(layout, "o")).toBe(1);
    expect(layout.edges).toEqual([{ from: id("g"), to: id("o"), near: true }]);
  });

  it("keeps the records closest to the focus and counts the rest", () => {
    const chain = Array.from({ length: 101 }, (_, step) =>
      summary(`s${step}`, step, step === 0 ? [] : [`s${step - 1}`]),
    );
    const focus = chain[60]!;
    const ancestors = chain.slice(0, 60).reverse();
    const descendants = chain.slice(61);
    const layout = new LineageLayout(tree(focus, ancestors, descendants)).build(11);
    expect(layout.nodes).toHaveLength(11);
    expect(layout.hidden.earlier + layout.hidden.later).toBe(90);
    const kept = layout.nodes.map((node) => node.summary!.position);
    expect(Math.min(...kept)).toBeGreaterThanOrEqual(55);
    expect(Math.max(...kept)).toBeLessThanOrEqual(65);
  });

  it("sizes the drawing to its widest layer and its depth", () => {
    const layout = new LineageLayout(tree(merge, [left, right, base])).build();
    expect(layout.width).toBe(2 * 12 + 2 * NODE.width + 24);
    expect(layout.height).toBe(2 * 12 + 3 * NODE.height + 2 * 44);
    for (const node of layout.nodes) {
      expect(node.x).toBeGreaterThanOrEqual(12);
      expect(node.x + NODE.width).toBeLessThanOrEqual(layout.width - 12);
    }
  });

  it("handles a record with no connections", () => {
    const alone = summary("a", 0);
    const layout = new LineageLayout(tree(alone)).build();
    expect(layout.nodes).toHaveLength(1);
    expect(layout.edges).toEqual([]);
    expect(layout.hidden).toEqual({ earlier: 0, later: 0 });
  });
});
