"""Drives a real proofprint-node. Runs only when PROOFPRINT_NODE_BIN names the binary.

    cargo build -p proofprint-node
    PROOFPRINT_NODE_BIN=../target/debug/proofprint-node python -m unittest tests.test_end_to_end
"""

import json
import os
import socket
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

from proofprint import KINDS, NodeClient, NodeError, Publisher, TrainingRecorder

BINARY = os.environ.get("PROOFPRINT_NODE_BIN")


def free_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def start_node(folder: Path) -> tuple[subprocess.Popen[bytes], NodeClient]:
    """Start a node on a free port with its own data directory; wait until it answers."""
    port = free_port()
    process = subprocess.Popen(
        [BINARY, "--data", str(folder / "data"), "--listen", f"127.0.0.1:{port}", "--ui", str(folder / "no-ui")],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    client = NodeClient(f"http://127.0.0.1:{port}", timeout=10)
    deadline = time.monotonic() + 30
    while True:
        try:
            client.status()
            return process, client
        except NodeError:
            if process.poll() is not None or time.monotonic() > deadline:
                stop_node(process)
                raise
            time.sleep(0.1)


def stop_node(process: subprocess.Popen[bytes]) -> None:
    process.terminate()
    process.wait(timeout=10)


@unittest.skipUnless(BINARY, "PROOFPRINT_NODE_BIN is not set")
class EndToEndTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.folder = Path(tempfile.mkdtemp())
        cls.process, cls.client = start_node(cls.folder)

    @classmethod
    def tearDownClass(cls) -> None:
        stop_node(cls.process)

    def test_a_training_history_round_trips_through_the_real_node(self) -> None:
        client = self.client
        publisher = Publisher(client)
        status = client.status()
        self.assertTrue(status.signer.startswith("ed25519:"))

        manifest = self.folder / "manifest.txt"
        manifest.write_bytes(b"one\ntwo\n")
        dataset = publisher.dataset("corpus", "1", "crawl", manifest=manifest, examples=2)

        recorder = TrainingRecorder(
            client,
            run_id="run-1",
            framework="test/0",
            config={"lr": 0.001},
            parents=[dataset.id],
        )
        weights = self.folder / "weights.bin"
        weights.write_bytes(bytes(range(256)) * 8192)
        recorder.step(1, loss=2.0)
        last = recorder.step(2, loss=1.5, checkpoint=weights)
        model = recorder.model("m", "1", "arch", weights=weights)
        evaluation = publisher.evaluation("harness/1", "bench", {"acc": 0.5}, parents=[model])

        detail = client.record(last)
        blob = detail.blobs[0]
        self.assertRegex(blob.blob, r"^ppb1:[0-9a-f]{64}$")
        self.assertEqual(blob.size, weights.stat().st_size)
        self.assertTrue(detail.artifacts[0].available)
        self.assertEqual(client.record(model).blobs[0].blob, blob.blob, "same bytes, same id")

        report = client.verify(last)
        self.assertTrue(report.signature and report.inclusion and report.complete)
        self.assertEqual(report.verified_bytes, weights.stat().st_size)

        copy = self.folder / "copy.bin"
        self.assertEqual(client.download(blob.blob, copy), weights.stat().st_size)
        self.assertEqual(copy.read_bytes(), weights.read_bytes())

        proof = client.proof(evaluation.id)
        self.assertTrue(proof.valid)
        self.assertEqual(proof.tree_size, client.status().records)

        kinds = [d.kind for d in client.walk_parents(evaluation.id)]
        self.assertEqual(
            kinds,
            [
                KINDS["evaluation"],
                KINDS["model"],
                KINDS["training_step"],
                KINDS["training_run"],
                KINDS["training_step"],
                KINDS["dataset"],
            ],
        )

        stored = client.record(model)
        self.assertEqual(json.loads(stored.canonical), stored.record)
        signed = stored.signed
        self.assertEqual(client.append(signed).id, model, "re-publishing a stored record is a no-op")
        signed["record"]["payload"]["name"] = "forged"
        with self.assertRaises(NodeError) as refused:
            client.append(signed)
        self.assertEqual(refused.exception.status, 400)

        tree = client.tree(dataset.id)
        self.assertEqual(len(tree.descendants), 5)

    def test_a_second_node_accepts_copied_history_and_adds_its_own_check(self) -> None:
        publisher = Publisher(self.client)
        dataset = publisher.dataset("corpus-b", "1", "crawl")
        model = publisher.model("m-b", "1", "arch", parents=[dataset.id])
        evaluation = publisher.evaluation("harness/1", "bench", {"acc": 0.5}, parents=[model.id])

        with SecondNode() as reviewer:
            sent = self.client.copy_to(reviewer.client, evaluation.id)
            self.assertEqual(sent, [dataset.id, model.id, evaluation.id])
            review = Publisher(reviewer.client).verification("re-evaluation", "pass", parents=[evaluation.id])
            detail = reviewer.client.record(review.id)
            self.assertNotEqual(detail.signer, self.client.status().signer)
            self.assertEqual(reviewer.client.record(evaluation.id).signer, self.client.status().signer)
            self.assertTrue(reviewer.client.verify(evaluation.id).signature)


class SecondNode:
    """Another node with its own key and data directory, for the length of a `with` block."""

    def __enter__(self) -> "SecondNode":
        self.folder = tempfile.mkdtemp()
        self.process, self.client = start_node(Path(self.folder))
        return self

    def __exit__(self, *exc: object) -> None:
        stop_node(self.process)


if __name__ == "__main__":
    unittest.main()
