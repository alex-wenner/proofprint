import { describe, expect, it } from "vitest";
import { Api, ApiError } from "../src/api";

class FakeNode {
  readonly calls: { url: string; method: string }[] = [];
  private readonly responses: Response[];

  constructor(...responses: Response[]) {
    this.responses = responses;
  }

  readonly fetch = async (url: string, init?: RequestInit): Promise<Response> => {
    this.calls.push({ url, method: init?.method ?? "GET" });
    const next = this.responses.shift();
    if (!next) throw new TypeError("connection refused");
    return next;
  };
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
}

describe("Api", () => {
  it("leaves out empty filters and escapes the rest", async () => {
    const node = new FakeNode(json({ records: [], total: 0, offset: 0, root: null }), json({}));
    const api = new Api("", node.fetch);
    await api.records({ kind: "ml.dataset/v1", signer: "", offset: 0 });
    await api.records();
    expect(node.calls.map((call) => call.url)).toEqual([
      "/api/records?kind=ml.dataset%2Fv1&offset=0",
      "/api/records",
    ]);
  });

  it("keeps the colon in ids readable and escapes anything else", async () => {
    const node = new FakeNode(json({}), json({}));
    const api = new Api("http://127.0.0.1:4780/", node.fetch);
    await api.record("ppr1:abc");
    await api.tree("ppr1:a/b?c");
    expect(node.calls.map((call) => call.url)).toEqual([
      "http://127.0.0.1:4780/api/records/ppr1:abc",
      "http://127.0.0.1:4780/api/records/ppr1:a%2Fb%3Fc/tree",
    ]);
    expect(api.artifactUrl("ppb1:ff")).toBe("http://127.0.0.1:4780/api/artifacts/ppb1:ff");
  });

  it("posts to verify", async () => {
    const node = new FakeNode(json({}));
    await new Api("", node.fetch).verify("ppr1:abc");
    expect(node.calls[0]).toEqual({ url: "/api/records/ppr1:abc/verify", method: "POST" });
  });

  it("turns the node's error body into an ApiError", async () => {
    const node = new FakeNode(json({ error: "record not found: ppr1:abc" }, 404));
    const failure = await new Api("", node.fetch).record("ppr1:abc").catch((error: unknown) => error);
    expect(failure).toBeInstanceOf(ApiError);
    expect(failure).toMatchObject({ status: 404, message: "record not found: ppr1:abc" });
  });

  it("falls back to the status line when the body is not JSON", async () => {
    const node = new FakeNode(new Response("<html>", { status: 502, statusText: "Bad Gateway" }));
    await expect(new Api("", node.fetch).status()).rejects.toMatchObject({ status: 502, message: "Bad Gateway" });
  });

  it("reports an unreachable node as status 0", async () => {
    const node = new FakeNode();
    await expect(new Api("", node.fetch).status()).rejects.toMatchObject({ status: 0 });
  });
});
