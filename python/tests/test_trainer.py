"""The callback is exercised against a stand-in `transformers` module so the
test suite does not depend on the real package."""

import sys
import tempfile
import types
import unittest
from pathlib import Path
from types import SimpleNamespace

from proofprint import KINDS, NodeClient

from .stub_node import StubNode

if "transformers" not in sys.modules:
    fake = types.ModuleType("transformers")
    fake.TrainerCallback = object  # type: ignore[attr-defined]
    fake.__version__ = "0.0-test"
    sys.modules["transformers"] = fake

from proofprint.trainer import RecordingCallback  # noqa: E402


class TrainerCallbackTest(unittest.TestCase):
    def setUp(self) -> None:
        self.node = StubNode()
        self.client = NodeClient(self.node.start())
        self.addCleanup(self.node.stop)
        self.output = Path(tempfile.mkdtemp())
        self.args = SimpleNamespace(
            output_dir=str(self.output),
            to_dict=lambda: {"learning_rate": 5e-5, "output_dir": self.output},
        )
        self.state = SimpleNamespace(global_step=0, max_steps=4)
        self.callback = RecordingCallback(self.client, run_id="run-7", code_commit="abc123")

    def drive(self, *events: tuple[str, dict]) -> None:
        for name, extra in events:
            getattr(self.callback, name)(self.args, self.state, None, **extra)

    def test_run_then_one_record_per_logged_step_with_checkpoints_attached(self) -> None:
        self.drive(("on_train_begin", {}))
        run = self.client.record(self.callback.recorder.run)
        self.assertEqual(run.kind, KINDS["training_run"])
        self.assertEqual(run.payload["framework"], "transformers/0.0-test")
        self.assertEqual(run.payload["code_commit"], "abc123")
        self.assertEqual(run.payload["total_steps"], 4)
        self.assertEqual(run.payload["config"]["output_dir"], str(self.output))

        self.state.global_step = 2
        self.drive(("on_log", {"logs": {"loss": 2.5, "learning_rate": 5e-5, "epoch": 0.5}}))
        checkpoint = self.output / "checkpoint-2"
        checkpoint.mkdir()
        (checkpoint / "model.safetensors").write_bytes(b"weights at 2")
        self.drive(("on_save", {}))
        self.assertEqual(self.client.status().records, 1, "nothing published until the step closes")

        self.state.global_step = 3
        self.drive(("on_step_begin", {}))
        self.state.global_step = 4
        self.drive(("on_log", {"logs": {"loss": 2.0}}), ("on_train_end", {}))

        steps = self.client.all_records(kind=KINDS["training_step"])
        self.assertEqual([s.artifacts for s in steps], [1, 0])
        first = self.client.record(steps[0].id)
        self.assertEqual(first.payload["step"], 2)
        self.assertEqual(first.payload["loss"], 2.5)
        self.assertEqual(first.payload["metrics"], {"learning_rate": 5e-5, "epoch": 0.5})
        self.assertEqual(first.payload["checkpoint_blob"], "checkpoint")
        self.assertEqual(self.node.blobs[first.blobs[0].blob][1], b"weights at 2")
        second = self.client.record(steps[1].id)
        self.assertEqual(second.payload, {"step": 4, "loss": 2.0})
        self.assertEqual(second.parents, (run.id, first.id))

    def test_events_before_train_begin_are_ignored(self) -> None:
        self.drive(("on_log", {"logs": {"loss": 1.0}}), ("on_train_end", {}))
        self.assertEqual(self.client.status().records, 0)


if __name__ == "__main__":
    unittest.main()
