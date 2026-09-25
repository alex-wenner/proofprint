import json
import os
import tempfile
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path

from proofprint import BlobRef, NodeClient, NodeError

from .stub_node import StubNode


class ClientTest(unittest.TestCase):
    def setUp(self) -> None:
        self.node = StubNode()
        self.client = NodeClient(self.node.start())
        self.addCleanup(self.node.stop)

    def publish(self, kind: str = "example.note/v1", **fields):
        return self.node.publish({"kind": kind, "payload": {}, **fields})["id"]

    def test_status_listing_and_paging(self) -> None:
        self.assertEqual(self.client.status().records, 0)
        first = self.publish(payload={"title": "first"})
        self.publish(kind="ml.evaluation/v1", parents=[first])
        self.publish(signer="ed25519:" + "cd" * 32)

        page = self.client.records(limit=2)
        self.assertEqual(page.total, 3)
        self.assertEqual([r.position for r in page.records], [0, 1])
        self.assertEqual(page.records[0].label, "first")
        self.assertEqual(page.records[1].parents, (first,))

        self.assertEqual([r.position for r in self.client.all_records()], [0, 1, 2])
        self.assertEqual(len(self.client.all_records(kind="ml.evaluation/v1")), 1)
        self.assertEqual(len(self.client.all_records(signer="ed25519:" + "cd" * 32)), 1)
        self.assertIn("kind=ml.evaluation%2Fv1", self.node.requests[-2][1])

    def test_record_tree_proof_and_verify(self) -> None:
        root = self.publish(payload={"name": "root"})
        child = self.publish(parents=[root])
        detail = self.client.record(child)
        self.assertEqual(detail.parents, (root,))
        self.assertEqual(detail.signed["sig"], "aa" * 64)
        self.assertEqual(self.client.record(root).children, (child,))

        tree = self.client.tree(child)
        self.assertEqual([a.id for a in tree.ancestors], [root])
        self.assertEqual(tree.descendants, ())

        proof = self.client.proof(child)
        self.assertTrue(proof.valid)
        self.assertEqual(proof.leaf_index, 1)

        report = self.client.verify(child)
        self.assertTrue(report.signature and report.inclusion and report.complete)

    def test_walk_parents_visits_each_record_once_and_skips_absent(self) -> None:
        base = self.publish(payload={"name": "base"})
        left = self.publish(payload={"name": "left"}, parents=[base])
        right = self.publish(payload={"name": "right"}, parents=[base])
        merge = self.publish(payload={"name": "merge"}, parents=[left, right])
        self.node.records[0]["record"]["parents"] = ["ppr1:" + "ef" * 32]

        visited = [d.id for d in self.client.walk_parents(merge)]
        self.assertEqual(visited, [merge, left, right, base])

    def test_upload_streams_with_a_known_length_and_downloads_back(self) -> None:
        data = bytes(range(256)) * 4096 * 3 + b"tail"
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "weights.json"
            source.write_bytes(data)
            reference = self.client.upload(source)
            self.assertIsInstance(reference, BlobRef)
            self.assertEqual(reference.name, "weights.json")
            self.assertEqual(reference.size, len(data))
            self.assertEqual(reference.media, "application/json")

            method, path, headers, body = self.node.requests[-1]
            self.assertEqual(int(headers["Content-Length"]), len(body))
            self.assertEqual(self.node.blobs[reference.blob][1], data)

            named = self.client.upload(source, name="checkpoint", media="application/octet-stream")
            self.assertEqual((named.name, named.media, named.blob), ("checkpoint", "application/octet-stream", reference.blob))

            dest = Path(folder) / "copy.bin"
            self.assertEqual(self.client.download(reference.blob, dest), len(data))
            self.assertEqual(dest.read_bytes(), data)

        small = self.client.upload_bytes("note", b"hello", media="text/plain")
        self.assertEqual((small.name, small.size, small.media), ("note", 5, "text/plain"))

    def test_publish_sends_a_draft_the_node_signs(self) -> None:
        parent = self.publish()
        blob = BlobRef(name="weights", blob="ppb1:" + "12" * 32, size=3)
        when = datetime(2026, 1, 15, 13, 0, tzinfo=timezone(timedelta(hours=1)))
        published = self.client.publish(
            "ml.model/v1",
            {"name": "m"},
            parents=[parent],
            blobs=[blob],
            created=when,
        )
        self.assertEqual(published.position, 1)

        draft = json.loads(self.node.requests[-1][3])
        self.assertEqual(draft["kind"], "ml.model/v1")
        self.assertEqual(draft["parents"], [parent])
        self.assertEqual(draft["blobs"], [{"name": "weights", "blob": blob.blob, "size": 3}])
        self.assertEqual(draft["created"], "2026-01-15T12:00:00Z")

        stored = self.client.record(published.id)
        self.assertEqual(stored.blobs, (blob,))
        self.assertFalse(stored.artifacts[0].available)
        self.assertEqual(self.client.verify(published.id).missing, ("weights",))

        with self.assertRaises(ValueError):
            self.client.publish("example.note/v1", {}, created=datetime(2026, 1, 1))

    def test_append_hands_over_a_signed_envelope(self) -> None:
        envelope = {"record": {"kind": "example.note/v1", "payload": {"n": 1}}, "sig": "bb" * 64}
        published = self.client.append(envelope)
        self.assertEqual(self.client.record(published.id).sig, "bb" * 64)
        self.assertEqual(self.client.append(envelope).position, published.position)

    def test_copy_to_sends_ancestors_before_descendants(self) -> None:
        base = self.publish(payload={"name": "base"})
        left = self.publish(payload={"name": "left"}, parents=[base])
        right = self.publish(payload={"name": "right"}, parents=[base])
        merge = self.publish(payload={"name": "merge"}, parents=[left, right])
        self.publish(payload={"name": "unrelated"})

        other_node = StubNode()
        other = NodeClient(other_node.start())
        self.addCleanup(other_node.stop)
        other_node.publish(dict(self.node.records[0]["record"]), self.node.records[0]["sig"])

        sent = self.client.copy_to(other, merge)
        self.assertEqual(sent, [base, left, right, merge])
        self.assertEqual([r.id for r in other.all_records()], [base, left, right, merge])
        self.assertEqual(other.record(merge).sig, self.client.record(merge).sig)

    def test_failures_become_node_errors(self) -> None:
        with self.assertRaises(NodeError) as refused:
            self.client.record("ppr1:" + "00" * 32)
        self.assertEqual(refused.exception.status, 404)

        with self.assertRaises(NodeError) as malformed:
            self.client.record("nope")
        self.assertEqual(malformed.exception.status, 400)
        self.assertIn("invalid record id", malformed.exception.message)

        self.node.fail_next = (503, "signing is disabled")
        with self.assertRaises(NodeError) as unavailable:
            self.client.publish("example.note/v1", {})
        self.assertEqual((unavailable.exception.status, unavailable.exception.message), (503, "signing is disabled"))

        with self.assertRaises(NodeError) as unreachable:
            NodeClient("http://127.0.0.1:9", timeout=2).status()
        self.assertEqual(unreachable.exception.status, 0)


if __name__ == "__main__":
    unittest.main()
