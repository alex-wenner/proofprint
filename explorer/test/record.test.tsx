// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { Api, type Proof, type RecordDetail, type Tree, type VerifyReport } from "../src/api";
import { Parties } from "../src/parties";
import { RecordPage } from "../src/views/record";
import { id, summary, tree } from "./fixtures";
import { Mounted } from "./render";

const own = `ed25519:${"a".repeat(64)}`;
const record = summary("r", 1, ["b"]);

const detail: RecordDetail = {
  id: record.id,
  position: 1,
  signed: {
    record: {
      v: 1,
      kind: record.kind,
      parents: [id("b")],
      payload: { title: "r", count: 9007199254740992 },
      blobs: [
        { name: "weights", blob: `ppb1:${"1".repeat(64)}`, size: 2048 },
        { name: "report", blob: `ppb1:${"2".repeat(64)}`, size: 10, media: "application/json" },
      ],
      created: record.created,
      signer: own,
    },
    sig: "5".repeat(128),
  },
  canonical: `{"count":9007199254740993,"title":"r"}`,
  artifacts: [
    { reference: { name: "weights", blob: `ppb1:${"1".repeat(64)}`, size: 2048 }, available: true },
    { reference: { name: "report", blob: `ppb1:${"2".repeat(64)}`, size: 10, media: "application/json" }, available: false },
  ],
  children: [],
};

const lineage: Tree = tree(record, [summary("b", 0)]);

const proof: Proof = {
  record: record.id,
  leaf_index: 1,
  tree_size: 2,
  leaf_hash: "c".repeat(64),
  path: ["d".repeat(64)],
  root: "e".repeat(64),
  valid: true,
  scope: "local tree",
};

const report: VerifyReport = {
  signature: true,
  inclusion: true,
  complete: false,
  consistent: true,
  verified: ["weights"],
  missing: ["report"],
  corrupt: [],
  verified_bytes: 2048,
  root: "e".repeat(64),
  tree_size: 2,
};

/** An Api whose verify call answers with `answer`, and records that it was asked. */
function api(answer: Response): { api: Api; calls: string[] } {
  const calls: string[] = [];
  const fetcher = async (url: string, init?: RequestInit): Promise<Response> => {
    calls.push(`${init?.method ?? "GET"} ${url}`);
    return answer;
  };
  return { api: new Api("", fetcher), calls };
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
}

describe("RecordPage", () => {
  let mounted: Mounted | null = null;

  afterEach(async () => {
    await mounted?.unmount();
    mounted = null;
  });

  async function render(answer: Response): Promise<{ mounted: Mounted; calls: string[] }> {
    const parties = new Parties();
    parties.claim(own);
    const fake = api(answer);
    mounted = await Mounted.render(
      <RecordPage api={fake.api} parties={parties} detail={detail} tree={lineage} proof={proof} />,
    );
    return { mounted, calls: fake.calls };
  }

  it("shows what was signed and by whom", async () => {
    const { mounted } = await render(json(report));
    expect(mounted.find("h1").textContent).toBe("r");
    expect(mounted.container.textContent).toContain("this node's key");
    expect(mounted.container.textContent).toContain("#1 of 2 in this log");
    expect(mounted.all(".graph-node")).toHaveLength(2);
    expect(mounted.find("details pre").textContent).toBe(detail.canonical);
    expect(document.title).toBe("r · ProofPrint");
  });

  it("offers downloads only for attachments this node holds", async () => {
    const { mounted } = await render(json(report));
    const rows = mounted.all<HTMLTableRowElement>(".panel tbody tr");
    expect(rows[0]!.querySelector("a")?.getAttribute("href")).toBe(`/api/artifacts/ppb1:${"1".repeat(64)}`);
    expect(rows[0]!.querySelector("a")?.getAttribute("download")).toBe("weights");
    expect(rows[1]!.querySelector("a")).toBeNull();
    expect(rows.map((row) => row.cells[4]!.textContent)).toEqual(["on this node, not checked", "not on this node"]);
  });

  it("runs checks on request and marks each attachment", async () => {
    const { mounted, calls } = await render(json(report));
    expect(calls).toEqual([]);
    await mounted.press("Run checks");
    expect(calls).toEqual([`POST /api/records/${record.id}/verify`]);
    const cells = mounted.all<HTMLTableRowElement>(".panel tbody tr").map((row) => row.cells[4]!);
    expect(cells.map((cell) => [cell.className, cell.textContent])).toEqual([
      ["good", "match the content id"],
      ["warn", "not on this node"],
    ]);
    expect(mounted.find(".results").textContent).toContain("1 of 2 match (2.0 KiB), 1 not on this node");
  });

  it("reports a refused check without losing the page", async () => {
    const { mounted } = await render(json({ error: "signature verification failed" }, 400));
    await mounted.press("Run checks");
    expect(mounted.find(".check-output").textContent).toBe("Check failed: signature verification failed");
    expect(mounted.find("h1").textContent).toBe("r");
  });
});
