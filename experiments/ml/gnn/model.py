"""Graph autoencoder for the relational sampler (Track 1).

GraphSAGE encoder + inner-product decoder (Kipf & Welling GAE), with hooks for
the degree-KL and triangle penalties described in SPEC.md. Runnable as-is on
torch-geometric; the structural regularizers carry TODO markers where they
need the exported corpus statistics.
"""

from __future__ import annotations

import torch
import torch.nn as nn
import torch.nn.functional as F

try:
    from torch_geometric.nn import SAGEConv
except ImportError as exc:  # pragma: no cover - import-time guard
    raise ImportError(
        "torch-geometric required: pip install -r ../requirements.txt"
    ) from exc


class GraphSAGEEncoder(nn.Module):
    def __init__(self, in_dim: int, hidden: int = 128, latent: int = 64):
        super().__init__()
        self.conv1 = SAGEConv(in_dim, hidden)
        self.conv2 = SAGEConv(hidden, latent)
        self.dropout = nn.Dropout(0.1)

    def forward(self, x: torch.Tensor, edge_index: torch.Tensor) -> torch.Tensor:
        h = F.relu(self.conv1(x, edge_index))
        h = self.dropout(h)
        return self.conv2(h, edge_index)  # node embeddings z


class InnerProductDecoder(nn.Module):
    """p(edge i~j) = sigmoid(z_i . z_j)."""

    def forward(self, z: torch.Tensor, edge_index: torch.Tensor) -> torch.Tensor:
        src, dst = edge_index
        logits = (z[src] * z[dst]).sum(dim=-1)
        return logits  # caller applies BCEWithLogits / sigmoid


class GraphAutoencoder(nn.Module):
    def __init__(self, in_dim: int, hidden: int = 128, latent: int = 64):
        super().__init__()
        self.encoder = GraphSAGEEncoder(in_dim, hidden, latent)
        self.decoder = InnerProductDecoder()

    def encode(self, x, edge_index) -> torch.Tensor:
        return self.encoder(x, edge_index)

    def recon_loss(
        self,
        z: torch.Tensor,
        pos_edge_index: torch.Tensor,
        neg_edge_index: torch.Tensor,
    ) -> torch.Tensor:
        pos = self.decoder(z, pos_edge_index)
        neg = self.decoder(z, neg_edge_index)
        logits = torch.cat([pos, neg])
        target = torch.cat([torch.ones_like(pos), torch.zeros_like(neg)])
        return F.binary_cross_entropy_with_logits(logits, target)

    # --- structural regularizers (SPEC.md § Architecture) -----------------
    @staticmethod
    def degree_kl(z: torch.Tensor, target_degree_hist: torch.Tensor) -> torch.Tensor:
        """KL between sampled expected-degree dist and the corpus target.

        TODO(gnn): expected degree of node i ≈ Σ_j σ(z_i·z_j); bucketize and
        KL against `target_degree_hist` exported from the corpus.
        """
        raise NotImplementedError("degree_kl: see SPEC.md § Architecture")

    @staticmethod
    def triangle_penalty(z: torch.Tensor) -> torch.Tensor:
        """Penalize deviation of expected triangle count from corpus.

        TODO(gnn): E[triangles] from the soft adjacency σ(ZZ^T); compare to the
        corpus TriangleLogRatio target. Keep it batched/sparse for the A100.
        """
        raise NotImplementedError("triangle_penalty: see SPEC.md § Architecture")
