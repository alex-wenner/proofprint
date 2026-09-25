// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import type { RecordPage } from "../src/api";
import { Parties } from "../src/parties";
import { FilterForm, HistoryPage, historyPath, readFilter } from "../src/views/history";
import { id, summary } from "./fixtures";
import { Mounted } from "./render";

function page(records: RecordPage["records"], total = records.length, offset = 0): RecordPage {
  return { records, total, offset, root: "ab".repeat(32) };
}

describe("history filters", () => {
  it("round-trips through the URL and drops empty fields", () => {
    const filter = readFilter(new URLSearchParams("kind=ml.dataset%2Fv1&offset=-4"));
    expect(filter).toEqual({ kind: "ml.dataset/v1", signer: "", offset: 0 });
    expect(historyPath(filter)).toBe("/?kind=ml.dataset%2Fv1");
    expect(historyPath({ kind: "", signer: "", offset: 0 })).toBe("/");
    expect(historyPath({ kind: "", signer: "ed25519:ab", offset: 50 })).toBe("/?signer=ed25519%3Aab&offset=50");
  });
});

describe("HistoryPage", () => {
  let mounted: Mounted | null = null;
  const navigations: string[] = [];
  const navigate = (path: string): void => {
    navigations.push(path);
  };

  afterEach(async () => {
    await mounted?.unmount();
    mounted = null;
    navigations.length = 0;
  });

  it("lists records with links to each record and to narrower filters", async () => {
    const records = [summary("a", 0), { ...summary("b", 1, ["a"]), kind: "ml.dataset/v1" }];
    const filter = { kind: "", signer: "", offset: 0 };
    mounted = await Mounted.render(<HistoryPage page={page(records)} filter={filter} parties={new Parties()} navigate={navigate} />);

    const rows = mounted.all("tbody tr");
    expect(rows).toHaveLength(2);
    expect(rows[1]!.querySelector(".record-link")?.getAttribute("href")).toBe(`/records/${id("b")}`);
    expect(rows[1]!.querySelector("a.kind")?.getAttribute("href")).toBe("/?kind=ml.dataset%2Fv1");
    expect(rows[1]!.querySelector("a.chip")?.getAttribute("href")).toBe(`/?signer=${encodeURIComponent(records[1]!.signer)}`);
    expect(mounted.container.textContent).toContain("2 records in append order.");
    expect(mounted.all(".pager")).toHaveLength(0);
    expect(document.title).toBe("History · ProofPrint");
  });

  it("pages through long logs", async () => {
    const filter = { kind: "example.note/v1", signer: "", offset: 50 };
    const records = Array.from({ length: 50 }, (_, index) => summary(`r${index}`, 50 + index));
    mounted = await Mounted.render(<HistoryPage page={page(records, 120, 50)} filter={filter} parties={new Parties()} navigate={navigate} />);
    const links = mounted.all(".pager a").map((link) => link.getAttribute("href"));
    expect(links).toEqual(["/?kind=example.note%2Fv1", "/?kind=example.note%2Fv1&offset=100"]);
    expect(mounted.find(".pager span").textContent).toBe("51–100 of 120");
  });

  it("explains an empty log and an empty filter differently", async () => {
    const empty = { kind: "", signer: "", offset: 0 };
    mounted = await Mounted.render(<HistoryPage page={page([])} filter={empty} parties={new Parties()} navigate={navigate} />);
    expect(mounted.find(".empty").textContent).toContain("This log is empty.");
    await mounted.unmount();

    const filtered = { kind: "nope/v1", signer: "", offset: 0 };
    mounted = await Mounted.render(<HistoryPage page={page([])} filter={filtered} parties={new Parties()} navigate={navigate} />);
    expect(mounted.find(".empty").textContent).toBe("No records match this filter.");
  });

  it("submits the filter from the first page", async () => {
    const filter = { kind: " ml.dataset/v1 ", signer: "", offset: 100 };
    mounted = await Mounted.render(<FilterForm filter={filter} kinds={[]} signers={[]} navigate={navigate} />);
    await mounted.submit("form");
    expect(navigations).toEqual(["/?kind=ml.dataset%2Fv1"]);
  });
});
