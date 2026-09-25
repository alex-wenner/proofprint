"""A `transformers.Trainer` callback that records a run and its logged steps.

Import this module only when `transformers` is installed:

    from proofprint import NodeClient
    from proofprint.trainer import RecordingCallback

    trainer = Trainer(..., callbacks=[RecordingCallback(NodeClient(), run_id="run-42")])

One run record is published at train start. Each log event becomes a step
record; a checkpoint saved at the same step is attached to that record
instead of producing a second one.
"""

from __future__ import annotations

import json
import os
from dataclasses import dataclass, field
from typing import Any, Iterable

from transformers import TrainerCallback

from .client import NodeClient
from .training import TrainingRecorder

WEIGHT_FILES = ("model.safetensors", "pytorch_model.bin")


@dataclass
class _Pending:
    step: int
    loss: float | None = None
    metrics: dict[str, Any] = field(default_factory=dict)
    checkpoint: str | None = None


class RecordingCallback(TrainerCallback):
    def __init__(
        self,
        client: NodeClient,
        *,
        run_id: str,
        parents: Iterable[str] = (),
        code_commit: str | None = None,
        attach_checkpoints: bool = True,
    ) -> None:
        self.client = client
        self.run_id = run_id
        self.parents = tuple(parents)
        self.code_commit = code_commit
        self.attach_checkpoints = attach_checkpoints
        self.recorder: TrainingRecorder | None = None
        self._pending: _Pending | None = None

    def on_train_begin(self, args: Any, state: Any, control: Any, **kwargs: Any) -> None:
        import transformers

        self.recorder = TrainingRecorder(
            self.client,
            run_id=self.run_id,
            framework=f"transformers/{transformers.__version__}",
            config=_plain(args.to_dict()),
            code_commit=self.code_commit,
            total_steps=state.max_steps or None,
            parents=self.parents,
        )
        self.recorder.start()

    def on_log(self, args: Any, state: Any, control: Any, logs: dict[str, Any] | None = None, **kwargs: Any) -> None:
        if not logs:
            return
        pending = self._pending_for(state.global_step)
        loss = logs.get("loss")
        if isinstance(loss, (int, float)):
            pending.loss = float(loss)
        pending.metrics.update(
            {key: value for key, value in logs.items() if key != "loss" and isinstance(value, (int, float, str, bool))}
        )

    def on_save(self, args: Any, state: Any, control: Any, **kwargs: Any) -> None:
        if not self.attach_checkpoints:
            return
        folder = os.path.join(args.output_dir, f"checkpoint-{state.global_step}")
        for name in WEIGHT_FILES:
            path = os.path.join(folder, name)
            if os.path.isfile(path):
                self._pending_for(state.global_step).checkpoint = path
                return

    def on_step_begin(self, args: Any, state: Any, control: Any, **kwargs: Any) -> None:
        self.flush()

    def on_train_end(self, args: Any, state: Any, control: Any, **kwargs: Any) -> None:
        self.flush()

    def flush(self) -> str | None:
        """Publish the buffered step, if any. Returns its record id."""
        if self.recorder is None or self._pending is None:
            return None
        pending, self._pending = self._pending, None
        return self.recorder.step(
            pending.step,
            loss=pending.loss,
            metrics=pending.metrics or None,
            checkpoint=pending.checkpoint,
        )

    def _pending_for(self, step: int) -> _Pending:
        if self._pending is not None and self._pending.step != step:
            self.flush()
        if self._pending is None:
            self._pending = _Pending(step)
        return self._pending


def _plain(config: Any) -> dict[str, Any]:
    """JSON-safe copy of the training arguments; unknown objects become strings."""
    return json.loads(json.dumps(config, default=str))
