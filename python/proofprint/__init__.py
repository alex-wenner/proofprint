"""Client for a running proofprint-node.

The node holds the signing key. This package builds records, uploads
attachments, and asks the node to sign and append. It never sees a key and
never signs anything itself.
"""

from .client import NodeClient, NodeError
from .publisher import KINDS, Publisher
from .training import TrainingRecorder
from .types import (
    ArtifactStatus,
    BlobRef,
    Detail,
    Page,
    Proof,
    Published,
    Status,
    Summary,
    Tree,
    VerifyReport,
)

__all__ = [
    "KINDS",
    "ArtifactStatus",
    "BlobRef",
    "Detail",
    "NodeClient",
    "NodeError",
    "Page",
    "Proof",
    "Published",
    "Publisher",
    "Status",
    "Summary",
    "TrainingRecorder",
    "Tree",
    "VerifyReport",
]
