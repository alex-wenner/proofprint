"""Typed views of what the node returns. One class per response shape."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping

JsonObject = dict[str, Any]


@dataclass(frozen=True)
class BlobRef:
    """A content-addressed attachment as it appears inside a record."""

    name: str
    blob: str
    size: int
    media: str | None = None

    @classmethod
    def from_json(cls, data: Mapping[str, Any]) -> BlobRef:
        return cls(
            name=data["name"],
            blob=data["blob"],
            size=int(data["size"]),
            media=data.get("media"),
        )

    def to_json(self) -> JsonObject:
        out: JsonObject = {"name": self.name, "blob": self.blob, "size": self.size}
        if self.media is not None:
            out["media"] = self.media
        return out


@dataclass(frozen=True)
class Status:
    version: str
    records: int
    artifacts: int
    root: str | None
    signer: str | None
    mode: str

    @classmethod
    def from_json(cls, data: Mapping[str, Any]) -> Status:
        return cls(
            version=data["version"],
            records=int(data["records"]),
            artifacts=int(data["artifacts"]),
            root=data.get("root"),
            signer=data.get("signer"),
            mode=data["mode"],
        )


@dataclass(frozen=True)
class Summary:
    """One row of a listing."""

    id: str
    position: int
    kind: str
    created: str
    signer: str
    parents: tuple[str, ...]
    artifacts: int
    label: str

    @classmethod
    def from_json(cls, data: Mapping[str, Any]) -> Summary:
        return cls(
            id=data["id"],
            position=int(data["position"]),
            kind=data["kind"],
            created=data["created"],
            signer=data["signer"],
            parents=tuple(data.get("parents", ())),
            artifacts=int(data["artifacts"]),
            label=data["label"],
        )


@dataclass(frozen=True)
class Page:
    records: tuple[Summary, ...]
    total: int
    offset: int
    root: str | None

    @classmethod
    def from_json(cls, data: Mapping[str, Any]) -> Page:
        return cls(
            records=tuple(Summary.from_json(r) for r in data["records"]),
            total=int(data["total"]),
            offset=int(data["offset"]),
            root=data.get("root"),
        )


@dataclass(frozen=True)
class ArtifactStatus:
    reference: BlobRef
    available: bool

    @classmethod
    def from_json(cls, data: Mapping[str, Any]) -> ArtifactStatus:
        return cls(
            reference=BlobRef.from_json(data["reference"]),
            available=bool(data["available"]),
        )


@dataclass(frozen=True)
class Detail:
    """A stored record with its signature and attachment availability."""

    id: str
    position: int
    record: JsonObject
    sig: str
    #: The exact bytes the signature covers, as text.
    canonical: str
    artifacts: tuple[ArtifactStatus, ...]
    children: tuple[str, ...]

    @classmethod
    def from_json(cls, data: Mapping[str, Any]) -> Detail:
        signed = data["signed"]
        return cls(
            id=data["id"],
            position=int(data["position"]),
            record=dict(signed["record"]),
            sig=signed["sig"],
            canonical=data["canonical"],
            artifacts=tuple(ArtifactStatus.from_json(a) for a in data["artifacts"]),
            children=tuple(data.get("children", ())),
        )

    @property
    def kind(self) -> str:
        return self.record["kind"]

    @property
    def payload(self) -> JsonObject:
        return self.record["payload"]

    @property
    def parents(self) -> tuple[str, ...]:
        return tuple(self.record.get("parents", ()))

    @property
    def blobs(self) -> tuple[BlobRef, ...]:
        return tuple(BlobRef.from_json(b) for b in self.record.get("blobs", ()))

    @property
    def signer(self) -> str:
        return self.record["signer"]

    @property
    def created(self) -> str:
        return self.record["created"]

    @property
    def signed(self) -> JsonObject:
        """The envelope as the node stores it; what another node would accept."""
        return {"record": self.record, "sig": self.sig}


@dataclass(frozen=True)
class Tree:
    record: Summary
    ancestors: tuple[Summary, ...]
    descendants: tuple[Summary, ...]
    missing: tuple[str, ...]

    @classmethod
    def from_json(cls, data: Mapping[str, Any]) -> Tree:
        return cls(
            record=Summary.from_json(data["record"]),
            ancestors=tuple(Summary.from_json(r) for r in data["ancestors"]),
            descendants=tuple(Summary.from_json(r) for r in data["descendants"]),
            missing=tuple(data.get("missing", ())),
        )


@dataclass(frozen=True)
class Published:
    id: str
    position: int
    root: str

    @classmethod
    def from_json(cls, data: Mapping[str, Any]) -> Published:
        return cls(id=data["id"], position=int(data["position"]), root=data["root"])


@dataclass(frozen=True)
class Proof:
    record: str
    leaf_index: int
    tree_size: int
    leaf_hash: str
    path: tuple[str, ...]
    root: str
    valid: bool
    scope: str

    @classmethod
    def from_json(cls, data: Mapping[str, Any]) -> Proof:
        return cls(
            record=data["record"],
            leaf_index=int(data["leaf_index"]),
            tree_size=int(data["tree_size"]),
            leaf_hash=data["leaf_hash"],
            path=tuple(data["path"]),
            root=data["root"],
            valid=bool(data["valid"]),
            scope=data["scope"],
        )


@dataclass(frozen=True)
class VerifyReport:
    signature: bool
    inclusion: bool
    complete: bool
    consistent: bool
    verified: tuple[str, ...]
    missing: tuple[str, ...]
    corrupt: tuple[tuple[str, str], ...]
    verified_bytes: int
    root: str
    tree_size: int

    @classmethod
    def from_json(cls, data: Mapping[str, Any]) -> VerifyReport:
        return cls(
            signature=bool(data["signature"]),
            inclusion=bool(data["inclusion"]),
            complete=bool(data["complete"]),
            consistent=bool(data["consistent"]),
            verified=tuple(data["verified"]),
            missing=tuple(data["missing"]),
            corrupt=tuple((name, reason) for name, reason in data["corrupt"]),
            verified_bytes=int(data["verified_bytes"]),
            root=data["root"],
            tree_size=int(data["tree_size"]),
        )
