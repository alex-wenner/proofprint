"""In-process stand-in for proofprint-node: same routes and shapes, no signing.

Ids are BLAKE2 here rather than BLAKE3; the client never checks them. The
end-to-end test covers the real node.
"""

from __future__ import annotations

import hashlib
import json
import re
import threading
from collections import deque
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any
from urllib.parse import parse_qs, urlparse

JsonObject = dict[str, Any]
FAKE_HASH = "00" * 32


class StubNode:
    signer = "ed25519:" + "ab" * 32

    def __init__(self) -> None:
        self.records: list[JsonObject] = []
        self.blobs: dict[str, tuple[str, bytes, str]] = {}
        self.requests: list[tuple[str, str, dict[str, str], bytes]] = []
        self.fail_next: tuple[int, str] | None = None
        self._server: ThreadingHTTPServer | None = None

    # Lifecycle

    def start(self) -> str:
        self._server = ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
        self._server.node = self  # type: ignore[attr-defined]
        threading.Thread(target=self._server.serve_forever, daemon=True).start()
        return f"http://127.0.0.1:{self._server.server_address[1]}"

    def stop(self) -> None:
        if self._server:
            self._server.shutdown()
            self._server.server_close()

    # Ledger

    def publish(self, record: JsonObject, sig: str = "aa" * 64) -> JsonObject:
        record.setdefault("v", 1)
        record.setdefault("signer", self.signer)
        record.setdefault("created", "2026-01-15T12:00:00Z")
        record_id = "ppr1:" + _digest(json.dumps(record, sort_keys=True).encode())
        for entry in self.records:
            if entry["id"] == record_id:
                return self._published(entry)
        for parent in record.get("parents", ()):
            if not any(entry["id"] == parent for entry in self.records):
                raise _Refused(400, f"parent record not found in log: {parent}")
        entry = {"id": record_id, "record": record, "sig": sig}
        self.records.append(entry)
        return self._published(entry)

    def handle(self, method: str, path: str, query: dict[str, list[str]], body: bytes, headers: Any) -> Any:
        parts = path.strip("/").split("/")
        if parts == ["api", "status"] and method == "GET":
            return self._status()
        if parts[:2] == ["api", "records"]:
            if len(parts) == 2 and method == "GET":
                return self._page(query)
            if len(parts) == 2 and method == "POST":
                signed = json.loads(body)
                return self.publish(signed["record"], signed["sig"])
            entry = self._find(parts[2])
            if len(parts) == 3 and method == "GET":
                return self._detail(entry)
            if len(parts) == 4 and parts[3] == "tree" and method == "GET":
                return self._tree(entry)
            if len(parts) == 4 and parts[3] == "proof" and method == "GET":
                return self._proof(entry)
            if len(parts) == 4 and parts[3] == "verify" and method == "POST":
                return self._verify(entry)
        if parts == ["api", "publish"] and method == "POST":
            draft = json.loads(body)
            record = {
                "kind": draft["kind"],
                "payload": draft.get("payload", {}),
                "created": draft.get("created", "2026-01-15T12:00:00Z"),
            }
            if draft.get("parents"):
                record["parents"] = draft["parents"]
            if draft.get("blobs"):
                record["blobs"] = draft["blobs"]
            return self.publish(record)
        if parts == ["api", "artifacts"] and method == "POST":
            return self._upload(body, headers["Content-Type"])
        if parts[:2] == ["api", "artifacts"] and len(parts) == 3 and method == "GET":
            if parts[2] not in self.blobs:
                raise _Refused(404, "not found")
            return self.blobs[parts[2]][1]
        raise _Refused(404, "not found")

    # Routes

    def _status(self) -> JsonObject:
        return {
            "version": "stub",
            "records": len(self.records),
            "artifacts": len(self.blobs),
            "root": FAKE_HASH if self.records else None,
            "signer": self.signer,
            "mode": "stub",
        }

    def _page(self, query: dict[str, list[str]]) -> JsonObject:
        kind = query.get("kind", [None])[0]
        signer = query.get("signer", [None])[0]
        offset = int(query.get("offset", ["0"])[0])
        limit = int(query.get("limit", ["200"])[0])
        matching = [
            self._summary(position, entry)
            for position, entry in enumerate(self.records)
            if (kind is None or entry["record"]["kind"] == kind)
            and (signer is None or entry["record"]["signer"] == signer)
        ]
        return {
            "records": matching[offset : offset + limit],
            "total": len(matching),
            "offset": offset,
            "root": FAKE_HASH if self.records else None,
        }

    def _detail(self, entry: JsonObject) -> JsonObject:
        return {
            "id": entry["id"],
            "position": self.records.index(entry),
            "signed": {"record": entry["record"], "sig": entry["sig"]},
            "canonical": json.dumps(entry["record"], sort_keys=True, separators=(",", ":")),
            "artifacts": [
                {"reference": blob, "available": blob["blob"] in self.blobs}
                for blob in entry["record"].get("blobs", ())
            ],
            "children": [e["id"] for e in self.records if entry["id"] in e["record"].get("parents", ())],
        }

    def _tree(self, entry: JsonObject) -> JsonObject:
        ancestors, missing = self._walk(entry, lambda e: e["record"].get("parents", ()))
        descendants, _ = self._walk(
            entry,
            lambda e: [c["id"] for c in self.records if e["id"] in c["record"].get("parents", ())],
        )
        return {
            "record": self._summary(self.records.index(entry), entry),
            "ancestors": ancestors,
            "descendants": descendants,
            "missing": missing,
        }

    def _proof(self, entry: JsonObject) -> JsonObject:
        return {
            "record": entry["id"],
            "leaf_index": self.records.index(entry),
            "tree_size": len(self.records),
            "leaf_hash": FAKE_HASH,
            "path": [],
            "root": FAKE_HASH,
            "valid": True,
            "scope": "local tree",
        }

    def _verify(self, entry: JsonObject) -> JsonObject:
        blobs = entry["record"].get("blobs", ())
        missing = [b["name"] for b in blobs if b["blob"] not in self.blobs]
        verified = [b["name"] for b in blobs if b["blob"] in self.blobs]
        return {
            "signature": True,
            "inclusion": True,
            "complete": not missing,
            "consistent": True,
            "verified": verified,
            "missing": missing,
            "corrupt": [],
            "verified_bytes": sum(b["size"] for b in blobs if b["blob"] in self.blobs),
            "root": FAKE_HASH,
            "tree_size": len(self.records),
        }

    def _upload(self, body: bytes, content_type: str) -> JsonObject:
        boundary = content_type.split("boundary=")[1].encode()
        part = body.split(b"--" + boundary + b"\r\n", 1)[1].rsplit(b"\r\n--" + boundary + b"--", 1)[0]
        head, _, data = part.partition(b"\r\n\r\n")
        headers = head.decode()
        name = re.search(r'filename="((?:[^"\\]|\\.)*)"', headers).group(1)
        media = re.search(r"Content-Type: (.+)", headers).group(1).strip()
        blob_id = "ppb1:" + _digest(data)
        self.blobs[blob_id] = (name, data, media)
        return {"name": name, "blob": blob_id, "size": len(data), "media": media}

    # Helpers

    def _find(self, record_id: str) -> JsonObject:
        if not re.fullmatch(r"ppr1:[0-9a-f]{64}", record_id):
            raise _Refused(400, f"invalid record id {record_id!r}")
        for entry in self.records:
            if entry["id"] == record_id:
                return entry
        raise _Refused(404, f"record not found: {record_id}")

    def _walk(self, start: JsonObject, neighbours: Any) -> tuple[list[JsonObject], list[str]]:
        out: list[JsonObject] = []
        missing: list[str] = []
        seen = {start["id"]}
        queue = deque([start])
        while queue:
            current = queue.popleft()
            for other_id in neighbours(current):
                if other_id in seen:
                    continue
                seen.add(other_id)
                other = next((e for e in self.records if e["id"] == other_id), None)
                if other is None:
                    missing.append(other_id)
                    continue
                out.append(self._summary(self.records.index(other), other))
                queue.append(other)
        return out, missing

    def _summary(self, position: int, entry: JsonObject) -> JsonObject:
        record = entry["record"]
        payload = record["payload"]
        label = next(
            (payload[k] for k in ("title", "name", "run_id", "benchmark", "method") if isinstance(payload.get(k), str)),
            f"step {payload['step']}" if isinstance(payload.get("step"), int) else record["kind"],
        )
        return {
            "id": entry["id"],
            "position": position,
            "kind": record["kind"],
            "created": record["created"],
            "signer": record["signer"],
            "parents": list(record.get("parents", ())),
            "artifacts": len(record.get("blobs", ())),
            "label": label,
        }

    def _published(self, entry: JsonObject) -> JsonObject:
        return {"id": entry["id"], "position": self.records.index(entry), "root": FAKE_HASH}


class _Refused(Exception):
    def __init__(self, status: int, message: str) -> None:
        super().__init__(message)
        self.status = status
        self.message = message


class _Handler(BaseHTTPRequestHandler):
    def log_message(self, *args: Any) -> None:
        pass

    def do_GET(self) -> None:
        self._dispatch("GET")

    def do_POST(self) -> None:
        self._dispatch("POST")

    def _dispatch(self, method: str) -> None:
        node: StubNode = self.server.node  # type: ignore[attr-defined]
        url = urlparse(self.path)
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else b""
        node.requests.append((method, self.path, dict(self.headers), body))
        if node.fail_next:
            status, message = node.fail_next
            node.fail_next = None
            self._json(status, {"error": message})
            return
        try:
            answer = node.handle(method, url.path, parse_qs(url.query), body, self.headers)
        except _Refused as refusal:
            self._json(refusal.status, {"error": refusal.message})
            return
        if isinstance(answer, bytes):
            self._bytes(answer)
        else:
            self._json(200, answer)

    def _json(self, status: int, value: Any) -> None:
        data = json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _bytes(self, data: bytes) -> None:
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


def _digest(data: bytes) -> str:
    return hashlib.blake2b(data, digest_size=32).hexdigest()
