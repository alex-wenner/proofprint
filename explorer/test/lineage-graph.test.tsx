// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { LineageGraph } from "../src/components/lineage-graph";
import { LineageLayout } from "../src/graph/layout";
import { Parties } from "../src/parties";
import { id, summary, tree } from "./fixtures";
import { Mounted } from "./render";

describe("LineageGraph", () => {
  const base = summary("b", 0);
  const record = summary("r", 1, ["b", "g"]);
  const child = { ...summary("c", 2, ["r"]), signer: `ed25519:${"c".repeat(64)}` };
  const layout = new LineageLayout(tree(record, [base], [child], ["g"])).build();
  let mounted: Mounted | null = null;

  afterEach(async () => {
    await mounted?.unmount();
    mounted = null;
  });

  it("links every stored record to its page and marks the focus", async () => {
    mounted = await Mounted.render(<LineageGraph layout={layout} parties={new Parties()} />);
    const links = mounted.all("a").map((link) => link.getAttribute("href"));
    expect(links).toHaveLength(3);
    expect(links).toEqual(expect.arrayContaining([`/records/${id("b")}`, `/records/${id("r")}`, `/records/${id("c")}`]));
    const focus = mounted.find(".graph-node.focus");
    expect(focus.getAttribute("href")).toBe(`/records/${id("r")}`);
    expect(focus.getAttribute("aria-current")).toBe("page");
    expect(focus.namespaceURI).toBe("http://www.w3.org/2000/svg");
  });

  it("draws absent parents without a link", async () => {
    mounted = await Mounted.render(<LineageGraph layout={layout} parties={new Parties()} />);
    const missing = mounted.find(".graph-node.missing");
    expect(missing.tagName).toBe("g");
    expect(missing.textContent).toContain("Not in this log");
  });

  it("draws one path per edge and colours signers apart", async () => {
    mounted = await Mounted.render(<LineageGraph layout={layout} parties={new Parties()} />);
    expect(mounted.all("path.graph-edge")).toHaveLength(3);
    expect(mounted.all("path.graph-edge.near")).toHaveLength(3);
    const dots = mounted.all("circle.graph-dot").map((dot) => dot.getAttribute("class"));
    expect(new Set(dots).size).toBe(2);
  });

  it("shortens long labels and keeps the full text in a title", async () => {
    const long = { ...summary("x", 0), label: "A label that is much too long to fit in one box" };
    mounted = await Mounted.render(
      <LineageGraph layout={new LineageLayout(tree(long)).build()} parties={new Parties()} />,
    );
    expect(mounted.find(".graph-label").textContent).toBe("A label that is much to…");
    expect(mounted.find("title").textContent).toContain(long.label);
  });
});
