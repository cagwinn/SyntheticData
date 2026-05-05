//! L4 Audit graph — typed property-graph schema for an audit engagement.
//!
//! Sourced from `src/gam_scraper/audit_graph/{nodes,edges,graph}.py` in
//! AuditMethodology v0.14.
//!
//! 11 node types × 10 edge types model the relationships an audit
//! engagement carries between entities, components, accounts, assertions,
//! risks, controls, procedures, findings, working papers, evidence, and
//! personnel.  Used downstream for analytics (RMM heatmaps, evidence-
//! coverage joins, finding-to-WP traceability).
//!
//! The wrapper is a typed multi-directed adjacency graph (parallel edges
//! between the same pair of nodes are allowed when their `edge_type`
//! differs) — analogous to networkx `MultiDiGraph` but with strict
//! validation at insertion: edges with missing endpoints are rejected.
//!
//! # Node types (11)
//!
//! Entity, Component, Account, Assertion, Risk, Control, Procedure,
//! Finding, WorkingPaper, Evidence, Personnel.
//!
//! # Edge types (10)
//!
//! BelongsTo, HasAccount, AssertedBy, IndicatesRisk, MitigatedBy,
//! TestedBy, ProducesEvidence, Documents, YieldsFinding,
//! ConsolidatesWith.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ── Node-type discriminator ───────────────────────────────────────────────────

/// Kind of node in the audit graph.  Matches the `node_type`
/// discriminator used by the upstream Python Pydantic models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditNodeType {
    /// Root engagement entity (group parent or standalone auditee).
    Entity,
    /// Subsidiary or component of the group under audit.
    Component,
    /// Financial-statement line item / account balance.
    Account,
    /// Audit assertion applied to an account (ISA 315 framework).
    Assertion,
    /// Identified risk of material misstatement.
    Risk,
    /// Control activity relevant to the audit.
    Control,
    /// Audit procedure (substantive or controls test).
    Procedure,
    /// Audit finding / observation raised during fieldwork.
    Finding,
    /// Working-paper reference.
    WorkingPaper,
    /// Evidence item obtained or generated during the engagement.
    Evidence,
    /// Audit-team member or client contact.
    Personnel,
}

// ── Per-node enums ────────────────────────────────────────────────────────────

/// ISA 315 assertion kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssertionKind {
    Existence,
    Completeness,
    Accuracy,
    Valuation,
    Presentation,
    RightsObligations,
    CutOff,
}

/// Risk kinds in the audit risk model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskKind {
    Inherent,
    Control,
    Significant,
    Fraud,
}

/// Finding severity (ISA 265 / SOX 404 dialect).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Observation,
    Deficiency,
    SignificantDeficiency,
    MaterialWeakness,
}

// ── Tagged node enum ──────────────────────────────────────────────────────────

/// One node in the audit graph.  Internally tagged with `node_type` so
/// the wire format matches the upstream Python models 1:1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "node_type", rename_all = "snake_case")]
pub enum AuditNode {
    Entity {
        node_id: String,
        name: String,
        #[serde(default)]
        is_listed: bool,
        #[serde(default)]
        industry: String,
    },
    Component {
        node_id: String,
        name: String,
        #[serde(default)]
        is_significant: bool,
        #[serde(default)]
        is_equity_method: bool,
    },
    Account {
        node_id: String,
        name: String,
        #[serde(default)]
        fs_line: String,
    },
    Assertion {
        node_id: String,
        name: String,
        assertion_kind: AssertionKind,
    },
    Risk {
        node_id: String,
        name: String,
        risk_kind: RiskKind,
        #[serde(default = "default_severity")]
        severity: String,
    },
    Control {
        node_id: String,
        name: String,
        #[serde(default)]
        is_automated: bool,
        #[serde(default)]
        is_key_control: bool,
    },
    Procedure {
        node_id: String,
        name: String,
        #[serde(default)]
        isa_refs: Vec<String>,
    },
    Finding {
        node_id: String,
        name: String,
        #[serde(default = "default_finding_severity")]
        severity: FindingSeverity,
    },
    WorkingPaper {
        node_id: String,
        name: String,
        wp_id: String,
    },
    Evidence {
        node_id: String,
        name: String,
        #[serde(default)]
        evidence_kind: String,
    },
    Personnel {
        node_id: String,
        name: String,
        #[serde(default)]
        role: String,
        #[serde(default = "default_true")]
        is_independent: bool,
    },
}

fn default_severity() -> String {
    "medium".to_string()
}
fn default_finding_severity() -> FindingSeverity {
    FindingSeverity::Observation
}
fn default_true() -> bool {
    true
}

impl AuditNode {
    /// Discriminator type of this node.
    pub fn node_type(&self) -> AuditNodeType {
        match self {
            AuditNode::Entity { .. } => AuditNodeType::Entity,
            AuditNode::Component { .. } => AuditNodeType::Component,
            AuditNode::Account { .. } => AuditNodeType::Account,
            AuditNode::Assertion { .. } => AuditNodeType::Assertion,
            AuditNode::Risk { .. } => AuditNodeType::Risk,
            AuditNode::Control { .. } => AuditNodeType::Control,
            AuditNode::Procedure { .. } => AuditNodeType::Procedure,
            AuditNode::Finding { .. } => AuditNodeType::Finding,
            AuditNode::WorkingPaper { .. } => AuditNodeType::WorkingPaper,
            AuditNode::Evidence { .. } => AuditNodeType::Evidence,
            AuditNode::Personnel { .. } => AuditNodeType::Personnel,
        }
    }

    /// Node id.
    pub fn node_id(&self) -> &str {
        match self {
            AuditNode::Entity { node_id, .. }
            | AuditNode::Component { node_id, .. }
            | AuditNode::Account { node_id, .. }
            | AuditNode::Assertion { node_id, .. }
            | AuditNode::Risk { node_id, .. }
            | AuditNode::Control { node_id, .. }
            | AuditNode::Procedure { node_id, .. }
            | AuditNode::Finding { node_id, .. }
            | AuditNode::WorkingPaper { node_id, .. }
            | AuditNode::Evidence { node_id, .. }
            | AuditNode::Personnel { node_id, .. } => node_id,
        }
    }

    /// Display name.
    pub fn name(&self) -> &str {
        match self {
            AuditNode::Entity { name, .. }
            | AuditNode::Component { name, .. }
            | AuditNode::Account { name, .. }
            | AuditNode::Assertion { name, .. }
            | AuditNode::Risk { name, .. }
            | AuditNode::Control { name, .. }
            | AuditNode::Procedure { name, .. }
            | AuditNode::Finding { name, .. }
            | AuditNode::WorkingPaper { name, .. }
            | AuditNode::Evidence { name, .. }
            | AuditNode::Personnel { name, .. } => name,
        }
    }
}

// ── Edges ─────────────────────────────────────────────────────────────────────

/// Kind of edge in the audit graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEdgeType {
    /// Component → Entity (membership).
    BelongsTo,
    /// Component (or Entity) → Account (financial-statement line).
    HasAccount,
    /// Assertion → Account.
    AssertedBy,
    /// Risk → Account or Assertion.
    IndicatesRisk,
    /// Risk → Control (risk is mitigated by control).
    MitigatedBy,
    /// Control or Risk → Procedure (procedure tests the control / risk).
    TestedBy,
    /// Procedure → Evidence.
    ProducesEvidence,
    /// WorkingPaper → Procedure or Evidence (WP documents the artifact).
    Documents,
    /// Procedure → Finding.
    YieldsFinding,
    /// Component → Entity (consolidation rollup).
    ConsolidatesWith,
}

/// One edge in the audit graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEdge {
    pub edge_id: String,
    pub source: String,
    pub target: String,
    pub edge_type: AuditEdgeType,
    #[serde(default)]
    pub description: String,
}

// ── Graph wrapper ─────────────────────────────────────────────────────────────

/// Errors raised by `AuditGraph` operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditGraphError {
    /// Edge endpoint refers to a missing node.
    MissingEndpoint {
        edge_id: String,
        endpoint: String,
        which: Endpoint,
    },
    /// Duplicate edge_id within the graph.
    DuplicateEdgeId { edge_id: String },
}

/// Which endpoint of an edge was missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    Source,
    Target,
}

impl std::fmt::Display for AuditGraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditGraphError::MissingEndpoint {
                edge_id,
                endpoint,
                which,
            } => {
                let which = match which {
                    Endpoint::Source => "source",
                    Endpoint::Target => "target",
                };
                write!(
                    f,
                    "edge {edge_id:?} references missing {which} node {endpoint:?}"
                )
            }
            AuditGraphError::DuplicateEdgeId { edge_id } => {
                write!(f, "duplicate edge_id {edge_id:?}")
            }
        }
    }
}

impl std::error::Error for AuditGraphError {}

/// Typed multi-directed audit graph.  Insertions are validated; the
/// underlying adjacency is exposed via traversal helpers.
#[derive(Debug, Clone, Default)]
pub struct AuditGraph {
    nodes: HashMap<String, AuditNode>,
    edges: HashMap<String, AuditEdge>,
    /// `node_id → list of edge_ids that have node_id as source`.
    out_adj: HashMap<String, Vec<String>>,
    /// `node_id → list of edge_ids that have node_id as target`.
    in_adj: HashMap<String, Vec<String>>,
}

impl AuditGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert (or replace) a typed node.  An existing node with the same
    /// id is silently overwritten — matches the upstream Python semantics.
    pub fn add_node(&mut self, node: AuditNode) {
        let id = node.node_id().to_string();
        self.nodes.insert(id.clone(), node);
        self.out_adj.entry(id.clone()).or_default();
        self.in_adj.entry(id).or_default();
    }

    /// Insert a typed edge.
    ///
    /// Returns `Err(MissingEndpoint)` if either endpoint is absent
    /// (matches the upstream Python behaviour of raising `KeyError` rather
    /// than silently auto-creating the missing node).
    /// Returns `Err(DuplicateEdgeId)` if the edge_id is already present.
    pub fn add_edge(&mut self, edge: AuditEdge) -> Result<(), AuditGraphError> {
        if !self.nodes.contains_key(&edge.source) {
            return Err(AuditGraphError::MissingEndpoint {
                edge_id: edge.edge_id.clone(),
                endpoint: edge.source.clone(),
                which: Endpoint::Source,
            });
        }
        if !self.nodes.contains_key(&edge.target) {
            return Err(AuditGraphError::MissingEndpoint {
                edge_id: edge.edge_id.clone(),
                endpoint: edge.target.clone(),
                which: Endpoint::Target,
            });
        }
        if self.edges.contains_key(&edge.edge_id) {
            return Err(AuditGraphError::DuplicateEdgeId {
                edge_id: edge.edge_id.clone(),
            });
        }
        let edge_id = edge.edge_id.clone();
        self.out_adj
            .entry(edge.source.clone())
            .or_default()
            .push(edge_id.clone());
        self.in_adj
            .entry(edge.target.clone())
            .or_default()
            .push(edge_id.clone());
        self.edges.insert(edge_id, edge);
        Ok(())
    }

    /// Total node count.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Total edge count.
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Look up a node by id.
    pub fn node(&self, node_id: &str) -> Option<&AuditNode> {
        self.nodes.get(node_id)
    }

    /// Look up an edge by id.
    pub fn edge(&self, edge_id: &str) -> Option<&AuditEdge> {
        self.edges.get(edge_id)
    }

    /// All node ids whose `node_type` matches.
    pub fn nodes_of_type(&self, node_type: AuditNodeType) -> Vec<&str> {
        self.nodes
            .values()
            .filter(|n| n.node_type() == node_type)
            .map(|n| n.node_id())
            .collect()
    }

    /// `(source, target, edge_id)` triples for matching edge type.
    pub fn edges_of_type(&self, edge_type: AuditEdgeType) -> Vec<(&str, &str, &str)> {
        self.edges
            .values()
            .filter(|e| e.edge_type == edge_type)
            .map(|e| (e.source.as_str(), e.target.as_str(), e.edge_id.as_str()))
            .collect()
    }

    /// Direct successor node ids of `node_id` (out-edges).  Returns each
    /// successor at most once even if multiple parallel edges connect to it.
    pub fn neighbors(&self, node_id: &str) -> Vec<&str> {
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        if let Some(out) = self.out_adj.get(node_id) {
            for eid in out {
                if let Some(edge) = self.edges.get(eid) {
                    if seen.insert(edge.target.as_str()) {
                        // pushed
                    }
                }
            }
        }
        seen.into_iter().collect()
    }

    /// Direct predecessor node ids of `node_id` (in-edges).  Deduplicated.
    pub fn predecessors(&self, node_id: &str) -> Vec<&str> {
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        if let Some(inp) = self.in_adj.get(node_id) {
            for eid in inp {
                if let Some(edge) = self.edges.get(eid) {
                    seen.insert(edge.source.as_str());
                }
            }
        }
        seen.into_iter().collect()
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(id: &str) -> AuditNode {
        AuditNode::Entity {
            node_id: id.to_string(),
            name: format!("Entity {id}"),
            is_listed: false,
            industry: String::new(),
        }
    }

    fn component(id: &str) -> AuditNode {
        AuditNode::Component {
            node_id: id.to_string(),
            name: format!("Component {id}"),
            is_significant: true,
            is_equity_method: false,
        }
    }

    fn account(id: &str, fs_line: &str) -> AuditNode {
        AuditNode::Account {
            node_id: id.to_string(),
            name: format!("Account {id}"),
            fs_line: fs_line.to_string(),
        }
    }

    fn assertion(id: &str, kind: AssertionKind) -> AuditNode {
        AuditNode::Assertion {
            node_id: id.to_string(),
            name: format!("Assertion {id}"),
            assertion_kind: kind,
        }
    }

    fn risk(id: &str, kind: RiskKind) -> AuditNode {
        AuditNode::Risk {
            node_id: id.to_string(),
            name: format!("Risk {id}"),
            risk_kind: kind,
            severity: "medium".to_string(),
        }
    }

    fn control(id: &str) -> AuditNode {
        AuditNode::Control {
            node_id: id.to_string(),
            name: format!("Control {id}"),
            is_automated: false,
            is_key_control: true,
        }
    }

    fn procedure(id: &str) -> AuditNode {
        AuditNode::Procedure {
            node_id: id.to_string(),
            name: format!("Procedure {id}"),
            isa_refs: vec!["ISA 330".to_string()],
        }
    }

    fn evidence(id: &str) -> AuditNode {
        AuditNode::Evidence {
            node_id: id.to_string(),
            name: format!("Evidence {id}"),
            evidence_kind: "external".to_string(),
        }
    }

    fn working_paper(id: &str) -> AuditNode {
        AuditNode::WorkingPaper {
            node_id: id.to_string(),
            name: format!("WP {id}"),
            wp_id: format!("WP-{id}"),
        }
    }

    fn finding(id: &str, sev: FindingSeverity) -> AuditNode {
        AuditNode::Finding {
            node_id: id.to_string(),
            name: format!("Finding {id}"),
            severity: sev,
        }
    }

    fn personnel(id: &str) -> AuditNode {
        AuditNode::Personnel {
            node_id: id.to_string(),
            name: format!("Person {id}"),
            role: "manager".to_string(),
            is_independent: true,
        }
    }

    fn make_edge(id: &str, source: &str, target: &str, et: AuditEdgeType) -> AuditEdge {
        AuditEdge {
            edge_id: id.to_string(),
            source: source.to_string(),
            target: target.to_string(),
            edge_type: et,
            description: String::new(),
        }
    }

    #[test]
    fn eleven_node_types_round_trip_via_node_type_accessor() {
        let nodes = [
            entity("E1"),
            component("C1"),
            account("A1", "revenue"),
            assertion("AS1", AssertionKind::Completeness),
            risk("R1", RiskKind::Significant),
            control("CT1"),
            procedure("P1"),
            finding("F1", FindingSeverity::Deficiency),
            working_paper("WP1"),
            evidence("EV1"),
            personnel("PR1"),
        ];
        let kinds: Vec<AuditNodeType> = nodes.iter().map(|n| n.node_type()).collect();
        assert_eq!(kinds.len(), 11);
        let unique: std::collections::HashSet<_> = kinds.iter().collect();
        assert_eq!(unique.len(), 11);
    }

    #[test]
    fn ten_edge_types_are_distinct() {
        let edge_types = [
            AuditEdgeType::BelongsTo,
            AuditEdgeType::HasAccount,
            AuditEdgeType::AssertedBy,
            AuditEdgeType::IndicatesRisk,
            AuditEdgeType::MitigatedBy,
            AuditEdgeType::TestedBy,
            AuditEdgeType::ProducesEvidence,
            AuditEdgeType::Documents,
            AuditEdgeType::YieldsFinding,
            AuditEdgeType::ConsolidatesWith,
        ];
        assert_eq!(edge_types.len(), 10);
        let unique: std::collections::HashSet<_> = edge_types.iter().collect();
        assert_eq!(unique.len(), 10);
    }

    #[test]
    fn add_node_and_lookup() {
        let mut g = AuditGraph::new();
        g.add_node(entity("E1"));
        assert_eq!(g.node_count(), 1);
        let n = g.node("E1").unwrap();
        assert_eq!(n.node_type(), AuditNodeType::Entity);
        assert_eq!(n.node_id(), "E1");
    }

    #[test]
    fn add_edge_with_missing_source_rejects() {
        let mut g = AuditGraph::new();
        g.add_node(component("C1"));
        // E1 is not present.
        let err = g
            .add_edge(make_edge("e1", "C1", "E1", AuditEdgeType::BelongsTo))
            .unwrap_err();
        match err {
            AuditGraphError::MissingEndpoint { which, .. } => {
                assert_eq!(which, Endpoint::Target);
            }
            _ => panic!("unexpected error: {err:?}"),
        }
    }

    #[test]
    fn add_edge_with_missing_target_rejects() {
        let mut g = AuditGraph::new();
        g.add_node(entity("E1"));
        let err = g
            .add_edge(make_edge("e1", "C1", "E1", AuditEdgeType::BelongsTo))
            .unwrap_err();
        match err {
            AuditGraphError::MissingEndpoint { which, .. } => {
                assert_eq!(which, Endpoint::Source);
            }
            _ => panic!("unexpected error: {err:?}"),
        }
    }

    #[test]
    fn duplicate_edge_id_rejects() {
        let mut g = AuditGraph::new();
        g.add_node(entity("E1"));
        g.add_node(component("C1"));
        g.add_edge(make_edge("e1", "C1", "E1", AuditEdgeType::BelongsTo))
            .unwrap();
        let err = g
            .add_edge(make_edge("e1", "C1", "E1", AuditEdgeType::BelongsTo))
            .unwrap_err();
        assert!(matches!(err, AuditGraphError::DuplicateEdgeId { .. }));
    }

    #[test]
    fn parallel_edges_with_different_types_allowed() {
        let mut g = AuditGraph::new();
        g.add_node(component("C1"));
        g.add_node(entity("E1"));
        g.add_edge(make_edge("e1", "C1", "E1", AuditEdgeType::BelongsTo))
            .unwrap();
        g.add_edge(make_edge("e2", "C1", "E1", AuditEdgeType::ConsolidatesWith))
            .unwrap();
        assert_eq!(g.edge_count(), 2);
        let belongs = g.edges_of_type(AuditEdgeType::BelongsTo);
        let consol = g.edges_of_type(AuditEdgeType::ConsolidatesWith);
        assert_eq!(belongs.len(), 1);
        assert_eq!(consol.len(), 1);
    }

    #[test]
    fn nodes_of_type_filters_correctly() {
        let mut g = AuditGraph::new();
        g.add_node(entity("E1"));
        g.add_node(component("C1"));
        g.add_node(component("C2"));
        g.add_node(account("A1", "revenue"));
        let entities = g.nodes_of_type(AuditNodeType::Entity);
        assert_eq!(entities, ["E1"]);
        let mut components = g.nodes_of_type(AuditNodeType::Component);
        components.sort();
        assert_eq!(components, ["C1", "C2"]);
        let accounts = g.nodes_of_type(AuditNodeType::Account);
        assert_eq!(accounts, ["A1"]);
    }

    #[test]
    fn neighbors_and_predecessors_work() {
        let mut g = AuditGraph::new();
        g.add_node(component("C1"));
        g.add_node(account("A1", "revenue"));
        g.add_node(assertion("AS1", AssertionKind::Completeness));
        g.add_edge(make_edge("e1", "C1", "A1", AuditEdgeType::HasAccount))
            .unwrap();
        g.add_edge(make_edge("e2", "AS1", "A1", AuditEdgeType::AssertedBy))
            .unwrap();
        let mut neighbors = g.neighbors("C1");
        neighbors.sort();
        assert_eq!(neighbors, ["A1"]);
        let mut preds = g.predecessors("A1");
        preds.sort();
        assert_eq!(preds, ["AS1", "C1"]);
    }

    #[test]
    fn neighbors_dedup_parallel_edges() {
        let mut g = AuditGraph::new();
        g.add_node(component("C1"));
        g.add_node(entity("E1"));
        g.add_edge(make_edge("e1", "C1", "E1", AuditEdgeType::BelongsTo))
            .unwrap();
        g.add_edge(make_edge("e2", "C1", "E1", AuditEdgeType::ConsolidatesWith))
            .unwrap();
        let neighbors = g.neighbors("C1");
        assert_eq!(
            neighbors.len(),
            1,
            "parallel edges should not duplicate the neighbor"
        );
    }

    #[test]
    fn full_engagement_walk_yields_finding() {
        // Component → Account ← Assertion
        // Risk → Account; Risk → Control; Procedure tests Control;
        // Procedure produces Evidence; WorkingPaper documents Procedure;
        // Procedure yields Finding.
        let mut g = AuditGraph::new();
        g.add_node(component("C1"));
        g.add_node(account("A1", "revenue"));
        g.add_node(assertion("AS1", AssertionKind::Completeness));
        g.add_node(risk("R1", RiskKind::Significant));
        g.add_node(control("CT1"));
        g.add_node(procedure("P1"));
        g.add_node(evidence("EV1"));
        g.add_node(working_paper("WP1"));
        g.add_node(finding("F1", FindingSeverity::Deficiency));

        g.add_edge(make_edge("e1", "C1", "A1", AuditEdgeType::HasAccount))
            .unwrap();
        g.add_edge(make_edge("e2", "AS1", "A1", AuditEdgeType::AssertedBy))
            .unwrap();
        g.add_edge(make_edge("e3", "R1", "A1", AuditEdgeType::IndicatesRisk))
            .unwrap();
        g.add_edge(make_edge("e4", "R1", "CT1", AuditEdgeType::MitigatedBy))
            .unwrap();
        g.add_edge(make_edge("e5", "CT1", "P1", AuditEdgeType::TestedBy))
            .unwrap();
        g.add_edge(make_edge(
            "e6",
            "P1",
            "EV1",
            AuditEdgeType::ProducesEvidence,
        ))
        .unwrap();
        g.add_edge(make_edge("e7", "WP1", "P1", AuditEdgeType::Documents))
            .unwrap();
        g.add_edge(make_edge("e8", "P1", "F1", AuditEdgeType::YieldsFinding))
            .unwrap();

        assert_eq!(g.node_count(), 9);
        assert_eq!(g.edge_count(), 8);
        // 8 unique edge types used out of 10.
        let used: std::collections::HashSet<_> = g.edges.values().map(|e| e.edge_type).collect();
        assert_eq!(used.len(), 8);
    }

    #[test]
    fn json_round_trip_each_node_kind() {
        let nodes = [
            entity("E1"),
            component("C1"),
            account("A1", "revenue"),
            assertion("AS1", AssertionKind::CutOff),
            risk("R1", RiskKind::Fraud),
            control("CT1"),
            procedure("P1"),
            finding("F1", FindingSeverity::MaterialWeakness),
            working_paper("WP1"),
            evidence("EV1"),
            personnel("PR1"),
        ];
        for n in nodes {
            let json = serde_json::to_string(&n).unwrap();
            let back: AuditNode = serde_json::from_str(&json).unwrap();
            assert_eq!(n, back);
            // Wire format includes the discriminator.
            assert!(json.contains("\"node_type\""));
        }
    }

    #[test]
    fn json_round_trip_each_edge_type() {
        for et in [
            AuditEdgeType::BelongsTo,
            AuditEdgeType::HasAccount,
            AuditEdgeType::AssertedBy,
            AuditEdgeType::IndicatesRisk,
            AuditEdgeType::MitigatedBy,
            AuditEdgeType::TestedBy,
            AuditEdgeType::ProducesEvidence,
            AuditEdgeType::Documents,
            AuditEdgeType::YieldsFinding,
            AuditEdgeType::ConsolidatesWith,
        ] {
            let edge = make_edge("e1", "src", "tgt", et);
            let json = serde_json::to_string(&edge).unwrap();
            let back: AuditEdge = serde_json::from_str(&json).unwrap();
            assert_eq!(edge, back);
        }
    }

    #[test]
    fn replacing_node_with_same_id_keeps_count() {
        let mut g = AuditGraph::new();
        g.add_node(component("C1"));
        assert_eq!(g.node_count(), 1);
        g.add_node(AuditNode::Component {
            node_id: "C1".to_string(),
            name: "Renamed".to_string(),
            is_significant: false,
            is_equity_method: true,
        });
        assert_eq!(g.node_count(), 1);
        match g.node("C1").unwrap() {
            AuditNode::Component {
                name,
                is_equity_method,
                ..
            } => {
                assert_eq!(name, "Renamed");
                assert!(*is_equity_method);
            }
            _ => panic!("expected Component"),
        }
    }
}
