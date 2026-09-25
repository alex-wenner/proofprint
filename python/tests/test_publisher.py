import tempfile
import unittest
from pathlib import Path

from proofprint import KINDS, NodeClient, Publisher, TrainingRecorder

from .stub_node import StubNode


class PublisherTest(unittest.TestCase):
    def setUp(self) -> None:
        self.node = StubNode()
        self.client = NodeClient(self.node.start())
        self.addCleanup(self.node.stop)
        self.publisher = Publisher(self.client)
        self.folder = Path(tempfile.mkdtemp())

    def file(self, name: str, content: bytes = b"bytes") -> Path:
        path = self.folder / name
        path.write_bytes(content)
        return path

    def test_dataset_uploads_its_manifest_under_the_schema_name(self) -> None:
        published = self.publisher.dataset(
            "corpus",
            "1",
            "crawl",
            license="CC0-1.0",
            examples=10,
            manifest=self.file("list.txt", b"a\nb\n"),
            notes={"source": "test"},
        )
        detail = self.client.record(published.id)
        self.assertEqual(detail.kind, KINDS["dataset"])
        self.assertEqual(detail.payload["manifest_blob"], "manifest")
        self.assertEqual(detail.payload["examples"], 10)
        self.assertNotIn("parents", detail.record)
        self.assertEqual(detail.blobs[0].name, "manifest")
        self.assertEqual(detail.blobs[0].size, 4)
        self.assertTrue(detail.artifacts[0].available)
        self.assertTrue(self.client.verify(published.id).complete)

    def test_optional_fields_are_left_out_when_absent(self) -> None:
        published = self.publisher.model("m", "1", "transformer-decoder")
        payload = self.client.record(published.id).payload
        self.assertEqual(payload, {"name": "m", "version": "1", "architecture": "transformer-decoder"})

    def test_evaluation_and_verification_link_to_their_targets(self) -> None:
        model = self.publisher.model("m", "1", "arch", weights=self.file("w.bin"))
        evaluation = self.publisher.evaluation(
            "lm-eval/0.4",
            "demo",
            {"acc": 0.5},
            report=self.file("report.json", b"{}"),
            parents=[model.id],
        )
        review = self.publisher.verification(
            "re-run",
            "pass",
            detail="matched within 0.1",
            evidence=self.file("log.txt"),
            parents=[evaluation.id],
        )
        detail = self.client.record(review.id)
        self.assertEqual(detail.kind, KINDS["verification"])
        self.assertEqual(detail.parents, (evaluation.id,))
        self.assertEqual(detail.payload["evidence_blob"], "evidence")
        self.assertEqual(self.client.record(evaluation.id).payload["report_blob"], "report")
        ancestors = [a.kind for a in self.client.tree(review.id).ancestors]
        self.assertEqual(ancestors, [KINDS["evaluation"], KINDS["model"]])

        with self.assertRaises(ValueError):
            self.publisher.verification("re-run", "maybe")

    def test_training_recorder_chains_steps_to_the_run_and_previous_step(self) -> None:
        recorder = TrainingRecorder(
            self.client,
            run_id="run-1",
            framework="test/0",
            config={"lr": 0.001},
            total_steps=2,
        )
        run = recorder.start()
        self.assertEqual(recorder.start(), run, "start is idempotent")
        first = recorder.step(1, loss=2.0, metrics={"grad_norm": 1.0})
        second = recorder.step(2, loss=1.5, checkpoint=self.file("ckpt.bin", b"weights"))
        model = recorder.model("m", "1", "arch", weights=self.file("final.bin"))

        self.assertEqual(self.client.record(first).parents, (run,))
        self.assertEqual(self.client.record(second).parents, (run, first))
        self.assertEqual(self.client.record(second).payload["checkpoint_blob"], "checkpoint")
        self.assertEqual(self.client.record(model).parents, (second,))
        self.assertEqual(self.client.record(run).payload["total_steps"], 2)

        walked = [d.kind for d in self.client.walk_parents(model)]
        self.assertEqual(
            walked,
            [KINDS["model"], KINDS["training_step"], KINDS["training_run"], KINDS["training_step"]],
        )


if __name__ == "__main__":
    unittest.main()
