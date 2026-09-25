// Typed access to one node's /api. Shapes mirror crates/proofprint-node/src/node.rs.

import { segment } from "./format";

export interface Status {
  readonly version: string;
  readonly records: number;
  readonly artifacts: number;
  readonly root: string | null;
  readonly signer: string | null;
  readonly mode: string;
}

export interface Summary {
  readonly id: string;
  readonly position: number;
  readonly kind: string;
  readonly created: string;
  readonly signer: string;
  readonly parents: readonly string[];
  readonly artifacts: number;
  readonly label: string;
}

export interface RecordPage {
  readonly records: readonly Summary[];
  readonly total: number;
  readonly offset: number;
  readonly root: string | null;
}

export interface BlobRef {
  readonly name: string;
  readonly blob: string;
  readonly size: number;
  readonly media?: string;
}

export interface RecordBody {
  readonly v: number;
  readonly kind: string;
  readonly parents?: readonly string[];
  readonly payload: Readonly<Record<string, unknown>>;
  readonly blobs?: readonly BlobRef[];
  readonly created: string;
  readonly signer: string;
}

export interface SignedRecord {
  readonly record: RecordBody;
  readonly sig: string;
}

export interface ArtifactStatus {
  readonly reference: BlobRef;
  readonly available: boolean;
}

export interface RecordDetail {
  readonly id: string;
  readonly position: number;
  readonly signed: SignedRecord;
  /** The exact signed bytes. Numbers in `signed` may have been rounded by JSON.parse. */
  readonly canonical: string;
  readonly artifacts: readonly ArtifactStatus[];
  readonly children: readonly string[];
}

export interface Tree {
  readonly record: Summary;
  readonly ancestors: readonly Summary[];
  readonly descendants: readonly Summary[];
  readonly missing: readonly string[];
}

export interface Proof {
  readonly record: string;
  readonly leaf_index: number;
  readonly tree_size: number;
  readonly leaf_hash: string;
  readonly path: readonly string[];
  readonly root: string;
  readonly valid: boolean;
  readonly scope: string;
}

export interface VerifyReport {
  readonly signature: boolean;
  readonly inclusion: boolean;
  readonly complete: boolean;
  readonly consistent: boolean;
  readonly verified: readonly string[];
  readonly missing: readonly string[];
  readonly corrupt: readonly (readonly [string, string])[];
  readonly verified_bytes: number;
  readonly root: string;
  readonly tree_size: number;
}

export interface RecordQuery {
  readonly kind?: string;
  readonly signer?: string;
  readonly offset?: number;
  readonly limit?: number;
}

/** A refusal from the node, or status 0 when it did not answer at all. */
export class ApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }
}

export type Fetch = (url: string, init?: RequestInit) => Promise<Response>;

export class Api {
  private readonly base: string;
  private readonly fetcher: Fetch;

  constructor(base = "", fetcher: Fetch = (url, init) => fetch(url, init)) {
    this.base = base.replace(/\/+$/, "");
    this.fetcher = fetcher;
  }

  status(): Promise<Status> {
    return this.request("GET", "/api/status");
  }

  records(query: RecordQuery = {}): Promise<RecordPage> {
    const params = new URLSearchParams();
    for (const [key, value] of Object.entries(query)) {
      if (value !== undefined && value !== "") params.set(key, String(value));
    }
    const search = params.toString();
    return this.request("GET", search ? `/api/records?${search}` : "/api/records");
  }

  record(id: string): Promise<RecordDetail> {
    return this.request("GET", `/api/records/${segment(id)}`);
  }

  tree(id: string): Promise<Tree> {
    return this.request("GET", `/api/records/${segment(id)}/tree`);
  }

  proof(id: string): Promise<Proof> {
    return this.request("GET", `/api/records/${segment(id)}/proof`);
  }

  /** Re-checks the signature, the inclusion proof, and every stored attachment's bytes. */
  verify(id: string): Promise<VerifyReport> {
    return this.request("POST", `/api/records/${segment(id)}/verify`);
  }

  artifactUrl(blob: string): string {
    return `${this.base}/api/artifacts/${segment(blob)}`;
  }

  private async request<T>(method: "GET" | "POST", path: string): Promise<T> {
    let response: Response;
    try {
      response = await this.fetcher(this.base + path, {
        method,
        headers: { Accept: "application/json" },
      });
    } catch (error) {
      throw new ApiError(0, `The node did not answer (${messageOf(error)}).`);
    }
    if (!response.ok) throw new ApiError(response.status, await refusal(response));
    return (await response.json()) as T;
  }
}

export function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function refusal(response: Response): Promise<string> {
  try {
    const body: unknown = await response.json();
    if (typeof body === "object" && body !== null && "error" in body && typeof body.error === "string") {
      return body.error;
    }
  } catch {
    // Not JSON; fall through to the status line.
  }
  return response.statusText || `HTTP ${response.status}`;
}
