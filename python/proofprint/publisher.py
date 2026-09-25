"""Typed publishing for the record kinds this project defines.

Each method builds one record, uploads the files it names, and asks the node
to sign it. What gets signed is what the caller states. Nothing here checks
that a dataset exists, that a model was trained, or that an evaluation ran.
"""

from __future__ import annotations

import os
from typing import Any, Iterable, Mapping

from .client import NodeClient
from .types import BlobRef, JsonObject, Published

PathLike = str | os.PathLike[str]

KINDS = {
    "dataset": "ml.dataset/v1",
    "model": "ml.model/v1",
    "training_run": "ml.training-run/v1",
    "training_step": "ml.training-step/v1",
    "evaluation": "ml.evaluation/v1",
    "verification": "attestation.verification/v1",
}

OUTCOMES = ("pass", "fail", "inconclusive")


class Publisher:
    """Builds records for the known kinds and publishes them through a client."""

    def __init__(self, client: NodeClient) -> None:
        self.client = client

    def dataset(
        self,
        name: str,
        version: str,
        origin: str,
        *,
        license: str | None = None,
        examples: int | None = None,
        manifest: PathLike | None = None,
        notes: Mapping[str, Any] | None = None,
        parents: Iterable[str] = (),
    ) -> Published:
        blobs = self._attachments(manifest=manifest)
        payload = _payload(
            name=name,
            version=version,
            origin=origin,
            license=license,
            examples=examples,
            manifest_blob="manifest" if manifest else None,
            notes=notes,
        )
        return self.client.publish(KINDS["dataset"], payload, parents=parents, blobs=blobs)

    def model(
        self,
        name: str,
        version: str,
        architecture: str,
        *,
        parameters: int | None = None,
        weights: PathLike | None = None,
        definition: PathLike | None = None,
        notes: Mapping[str, Any] | None = None,
        parents: Iterable[str] = (),
    ) -> Published:
        blobs = self._attachments(weights=weights, definition=definition)
        payload = _payload(
            name=name,
            version=version,
            architecture=architecture,
            parameters=parameters,
            weights_blob="weights" if weights else None,
            definition_blob="definition" if definition else None,
            notes=notes,
        )
        return self.client.publish(KINDS["model"], payload, parents=parents, blobs=blobs)

    def training_run(
        self,
        run_id: str,
        framework: str,
        config: Mapping[str, Any],
        *,
        code_commit: str | None = None,
        environment: Mapping[str, Any] | None = None,
        total_steps: int | None = None,
        parents: Iterable[str] = (),
    ) -> Published:
        payload = _payload(
            run_id=run_id,
            framework=framework,
            config=dict(config),
            code_commit=code_commit,
            environment=environment,
            total_steps=total_steps,
        )
        return self.client.publish(KINDS["training_run"], payload, parents=parents)

    def training_step(
        self,
        step: int,
        *,
        samples_seen: int | None = None,
        loss: float | None = None,
        metrics: Mapping[str, Any] | None = None,
        checkpoint: PathLike | None = None,
        parents: Iterable[str] = (),
    ) -> Published:
        blobs = self._attachments(checkpoint=checkpoint)
        payload = _payload(
            step=step,
            samples_seen=samples_seen,
            loss=loss,
            metrics=metrics,
            checkpoint_blob="checkpoint" if checkpoint else None,
        )
        return self.client.publish(KINDS["training_step"], payload, parents=parents, blobs=blobs)

    def evaluation(
        self,
        harness: str,
        benchmark: str,
        scores: Mapping[str, Any],
        *,
        report: PathLike | None = None,
        parents: Iterable[str] = (),
    ) -> Published:
        """A measured result. Name the evaluated model record in `parents`."""
        blobs = self._attachments(report=report)
        payload = _payload(
            harness=harness,
            benchmark=benchmark,
            scores=dict(scores),
            report_blob="report" if report else None,
        )
        return self.client.publish(KINDS["evaluation"], payload, parents=parents, blobs=blobs)

    def verification(
        self,
        method: str,
        outcome: str,
        *,
        detail: str | None = None,
        evidence: PathLike | None = None,
        parents: Iterable[str] = (),
    ) -> Published:
        """An outside party's check of someone else's record, named in `parents`."""
        if outcome not in OUTCOMES:
            raise ValueError(f"outcome must be one of {OUTCOMES}, not {outcome!r}")
        blobs = self._attachments(evidence=evidence)
        payload = _payload(
            method=method,
            outcome=outcome,
            detail=detail,
            evidence_blob="evidence" if evidence else None,
        )
        return self.client.publish(KINDS["verification"], payload, parents=parents, blobs=blobs)

    def _attachments(self, **files: PathLike | None) -> list[BlobRef]:
        return [self.client.upload(path, name=name) for name, path in files.items() if path]


def _payload(**fields: Any) -> JsonObject:
    return {key: value for key, value in fields.items() if value is not None}
