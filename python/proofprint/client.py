"""HTTP client for one node. Standard library only."""

from __future__ import annotations

import json
import mimetypes
import os
import shutil
import uuid
from collections import deque
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable, Iterator, Mapping
from urllib import error, parse, request

from .types import (
    BlobRef,
    Detail,
    JsonObject,
    Page,
    Proof,
    Published,
    Status,
    Summary,
    Tree,
    VerifyReport,
)

CHUNK = 1 << 20


class NodeError(Exception):
    """A non-2xx answer, or no answer at all (status 0)."""

    def __init__(self, status: int, message: str) -> None:
        super().__init__(f"{status}: {message}" if status else message)
        self.status = status
        self.message = message


class NodeClient:
    """Talks to one node. The node signs; this client never holds a key."""

    def __init__(self, base_url: str = "http://127.0.0.1:4780", *, timeout: float = 30.0) -> None:
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout

    # Reads

    def status(self) -> Status:
        return Status.from_json(self._json("GET", "/api/status"))

    def records(
        self,
        *,
        kind: str | None = None,
        signer: str | None = None,
        offset: int = 0,
        limit: int = 200,
    ) -> Page:
        query = {"offset": offset, "limit": limit, "kind": kind, "signer": signer}
        return Page.from_json(self._json("GET", "/api/records", query=query))

    def all_records(self, *, kind: str | None = None, signer: str | None = None) -> list[Summary]:
        """Every matching record, in log order, fetched a page at a time."""
        out: list[Summary] = []
        while True:
            page = self.records(kind=kind, signer=signer, offset=len(out), limit=1000)
            out.extend(page.records)
            if not page.records or len(out) >= page.total:
                return out

    def record(self, record_id: str) -> Detail:
        return Detail.from_json(self._json("GET", f"/api/records/{record_id}"))

    def tree(self, record_id: str) -> Tree:
        return Tree.from_json(self._json("GET", f"/api/records/{record_id}/tree"))

    def proof(self, record_id: str) -> Proof:
        return Proof.from_json(self._json("GET", f"/api/records/{record_id}/proof"))

    def verify(self, record_id: str) -> VerifyReport:
        """Re-check signature, inclusion, and every attachment's bytes."""
        return VerifyReport.from_json(self._json("POST", f"/api/records/{record_id}/verify"))

    def walk_parents(self, record_id: str) -> Iterator[Detail]:
        """Breadth-first over parents, each record once, starting with the record itself.

        A parent the node does not hold is skipped, not an error.
        """
        seen = {record_id}
        queue = deque([record_id])
        while queue:
            current = queue.popleft()
            try:
                detail = self.record(current)
            except NodeError as failure:
                if failure.status == 404:
                    continue
                raise
            yield detail
            for parent in detail.parents:
                if parent not in seen:
                    seen.add(parent)
                    queue.append(parent)

    # Writes

    def upload(self, path: str | os.PathLike[str], *, name: str | None = None, media: str | None = None) -> BlobRef:
        """Stream a file to the node. Returns the reference to put in a record."""
        source = Path(path)
        name = name or source.name
        media = media or mimetypes.guess_type(source.name)[0] or "application/octet-stream"
        body = _MultipartFile(name, media, source)
        data = self._json("POST", "/api/artifacts", body=body, content_type=body.content_type)
        return BlobRef.from_json(data)

    def upload_bytes(self, name: str, data: bytes, *, media: str = "application/octet-stream") -> BlobRef:
        body = _MultipartFile(name, media, data)
        answer = self._json("POST", "/api/artifacts", body=body, content_type=body.content_type)
        return BlobRef.from_json(answer)

    def download(self, blob_id: str, dest: str | os.PathLike[str]) -> int:
        """Stream an artifact to `dest`. Returns the byte count. Does not re-hash."""
        with self._open("GET", f"/api/artifacts/{blob_id}") as response, open(dest, "wb") as sink:
            shutil.copyfileobj(response, sink, CHUNK)
        return os.path.getsize(dest)

    def publish(
        self,
        kind: str,
        payload: Mapping[str, Any],
        *,
        parents: Iterable[str] = (),
        blobs: Iterable[BlobRef] = (),
        created: datetime | str | None = None,
    ) -> Published:
        """Ask the node to sign and append a record as itself."""
        draft: JsonObject = {
            "kind": kind,
            "payload": dict(payload),
            "parents": list(parents),
            "blobs": [b.to_json() for b in blobs],
        }
        if created is not None:
            draft["created"] = _timestamp(created)
        return Published.from_json(self._json("POST", "/api/publish", body=draft))

    def append(self, signed: Mapping[str, Any]) -> Published:
        """Hand over a record signed elsewhere, as `{"record": ..., "sig": ...}`."""
        return Published.from_json(self._json("POST", "/api/records", body=dict(signed)))

    def copy_to(self, other: NodeClient, record_id: str) -> list[str]:
        """Append a record and every ancestor this node holds to `other`, oldest first.

        Signatures travel with the records, so `other` checks them as usual.
        Records `other` already holds are left as they are. Returns the ids sent.
        """
        chain = sorted(self.walk_parents(record_id), key=lambda detail: detail.position)
        for detail in chain:
            other.append(detail.signed)
        return [detail.id for detail in chain]

    # Transport

    def _json(
        self,
        method: str,
        path: str,
        *,
        query: Mapping[str, Any] | None = None,
        body: Any = None,
        content_type: str | None = None,
    ) -> Any:
        with self._open(method, path, query=query, body=body, content_type=content_type) as response:
            return json.load(response)

    def _open(
        self,
        method: str,
        path: str,
        *,
        query: Mapping[str, Any] | None = None,
        body: Any = None,
        content_type: str | None = None,
    ) -> Any:
        url = self.base_url + path
        if query:
            present = {k: v for k, v in query.items() if v is not None}
            url += "?" + parse.urlencode(present)
        headers = {"Accept": "application/json"}
        data: Any = None
        if isinstance(body, _MultipartFile):
            headers["Content-Type"] = content_type or body.content_type
            headers["Content-Length"] = str(len(body))
            data = iter(body)
        elif body is not None:
            headers["Content-Type"] = content_type or "application/json"
            data = json.dumps(body).encode()
        elif method == "POST":
            data = b""
        req = request.Request(url, data=data, method=method, headers=headers)
        try:
            return request.urlopen(req, timeout=self.timeout)
        except error.HTTPError as failure:
            raise NodeError(failure.code, _message(failure)) from None
        except error.URLError as failure:
            raise NodeError(0, f"{url}: {failure.reason}") from None


class _MultipartFile:
    """One-file multipart body, streamed in chunks with a known length."""

    def __init__(self, name: str, media: str, source: Path | bytes) -> None:
        self.boundary = uuid.uuid4().hex
        self.content_type = f"multipart/form-data; boundary={self.boundary}"
        self.source = source
        self.head = (
            f"--{self.boundary}\r\n"
            f'Content-Disposition: form-data; name="file"; filename="{_quote(name)}"\r\n'
            f"Content-Type: {media}\r\n\r\n"
        ).encode()
        self.tail = f"\r\n--{self.boundary}--\r\n".encode()

    def __len__(self) -> int:
        size = len(self.source) if isinstance(self.source, bytes) else self.source.stat().st_size
        return len(self.head) + size + len(self.tail)

    def __iter__(self) -> Iterator[bytes]:
        yield self.head
        if isinstance(self.source, bytes):
            yield self.source
        else:
            with open(self.source, "rb") as handle:
                while chunk := handle.read(CHUNK):
                    yield chunk
        yield self.tail


def _quote(name: str) -> str:
    return name.replace("\\", "\\\\").replace('"', '\\"')


def _timestamp(value: datetime | str) -> str:
    """RFC 3339 in UTC at seconds precision, the only form the node stores."""
    if isinstance(value, str):
        return value
    if value.tzinfo is None:
        raise ValueError("created must be timezone-aware")
    return value.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _message(failure: error.HTTPError) -> str:
    try:
        return json.load(failure)["error"]
    except (ValueError, KeyError, TypeError):
        return failure.reason if isinstance(failure.reason, str) else str(failure)
