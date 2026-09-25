"""A run record followed by a chain of step records."""

from __future__ import annotations

from typing import Any, Iterable, Mapping

from .client import NodeClient
from .publisher import PathLike, Publisher


class TrainingRecorder:
    """Publishes one training-run record, then one record per step.

    Every step names the run and the previous step as parents, so the chain
    can be walked forward from the run or backward from any checkpoint.
    These are claims signed by the node's key; nothing here shows that the
    steps were computed.
    """

    def __init__(
        self,
        client: NodeClient,
        *,
        run_id: str,
        framework: str,
        config: Mapping[str, Any],
        code_commit: str | None = None,
        environment: Mapping[str, Any] | None = None,
        total_steps: int | None = None,
        parents: Iterable[str] = (),
    ) -> None:
        self.publisher = Publisher(client)
        self.run_id = run_id
        self.framework = framework
        self.config = dict(config)
        self.code_commit = code_commit
        self.environment = dict(environment) if environment else None
        self.total_steps = total_steps
        self.parents = tuple(parents)
        self.run: str | None = None
        self.last_step: str | None = None

    def start(self) -> str:
        """Publish the run record. Returns its id."""
        if self.run is not None:
            return self.run
        published = self.publisher.training_run(
            self.run_id,
            self.framework,
            self.config,
            code_commit=self.code_commit,
            environment=self.environment,
            total_steps=self.total_steps,
            parents=self.parents,
        )
        self.run = published.id
        return self.run

    def step(
        self,
        step: int,
        *,
        loss: float | None = None,
        samples_seen: int | None = None,
        metrics: Mapping[str, Any] | None = None,
        checkpoint: PathLike | None = None,
    ) -> str:
        """Publish a step linked to the run and the previous step. Returns its id."""
        run = self.start()
        parents = [run] if self.last_step is None else [run, self.last_step]
        published = self.publisher.training_step(
            step,
            loss=loss,
            samples_seen=samples_seen,
            metrics=metrics,
            checkpoint=checkpoint,
            parents=parents,
        )
        self.last_step = published.id
        return self.last_step

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
    ) -> str:
        """Publish the resulting model, linked to the last step (or the run)."""
        parent = self.last_step or self.start()
        published = self.publisher.model(
            name,
            version,
            architecture,
            parameters=parameters,
            weights=weights,
            definition=definition,
            notes=notes,
            parents=[parent],
        )
        return published.id
