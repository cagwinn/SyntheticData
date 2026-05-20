//! P3 — Shared-infrastructure graph motifs.

use std::collections::{HashMap, HashSet};

use super::math::wasserstein_1;
use super::types::Record;

/// For each attribute value, count the number of distinct entities that touched it.
pub fn fanout_distribution<F, G>(records: &[Record], entity_of: F, attr_of: G) -> Vec<f64>
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> Option<String> + Copy,
{
    let mut by_attr: HashMap<String, HashSet<String>> = HashMap::new();
    for r in records {
        let (Some(e), Some(a)) = (entity_of(r), attr_of(r)) else {
            continue;
        };
        by_attr.entry(a).or_default().insert(e);
    }
    by_attr.values().map(|s| s.len() as f64).collect()
}

pub fn fanout_w1<F, G>(real: &[Record], syn: &[Record], entity_of: F, attr_of: G) -> f64
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> Option<String> + Copy,
{
    let r = fanout_distribution(real, entity_of, attr_of);
    let s = fanout_distribution(syn, entity_of, attr_of);
    wasserstein_1(&r, &s)
}

/// Convenience projectors.
pub fn gl_account_of(r: &Record) -> Option<String> {
    Some(r.gl_account.clone())
}
pub fn cost_center_of(r: &Record) -> Option<String> {
    r.cost_center.clone()
}
pub fn profit_center_of(r: &Record) -> Option<String> {
    r.profit_center.clone()
}
pub fn trading_partner_attr_of(r: &Record) -> Option<String> {
    r.trading_partner.clone()
}

use petgraph::graph::{NodeIndex, UnGraph};

/// Entity-projection graph: undirected, one node per entity that shares any attribute.
fn build_entity_projection<F, G>(
    records: &[Record],
    entity_of: F,
    attr_of: G,
) -> (UnGraph<String, ()>, HashMap<String, NodeIndex>)
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> Option<String> + Copy,
{
    let mut by_entity: HashMap<String, HashSet<String>> = HashMap::new();
    for r in records {
        let (Some(e), Some(a)) = (entity_of(r), attr_of(r)) else {
            continue;
        };
        by_entity.entry(e).or_default().insert(a);
    }

    let mut g = UnGraph::<String, ()>::new_undirected();
    let mut idx: HashMap<String, NodeIndex> = HashMap::new();
    for e in by_entity.keys() {
        idx.insert(e.clone(), g.add_node(e.clone()));
    }
    let entities: Vec<&String> = by_entity.keys().collect();
    for i in 0..entities.len() {
        for j in (i + 1)..entities.len() {
            let a = &by_entity[entities[i]];
            let b = &by_entity[entities[j]];
            if a.iter().any(|v| b.contains(v)) {
                g.add_edge(idx[entities[i]], idx[entities[j]], ());
            }
        }
    }
    (g, idx)
}

pub fn clustering_coefficient<F, G>(records: &[Record], entity_of: F, attr_of: G) -> f64
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> Option<String> + Copy,
{
    let (g, _) = build_entity_projection(records, entity_of, attr_of);
    let mut triangles = 0usize;
    let mut triples = 0usize;
    for n in g.node_indices() {
        let neighbors: Vec<NodeIndex> = g.neighbors(n).collect();
        let k = neighbors.len();
        if k < 2 {
            continue;
        }
        triples += k * (k - 1) / 2;
        for i in 0..k {
            for j in (i + 1)..k {
                if g.find_edge(neighbors[i], neighbors[j]).is_some() {
                    triangles += 1;
                }
            }
        }
    }
    let triangles = triangles / 3;
    if triples == 0 {
        0.0
    } else {
        (3 * triangles) as f64 / triples as f64
    }
}

pub fn triangle_count<F, G>(records: &[Record], entity_of: F, attr_of: G) -> u64
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> Option<String> + Copy,
{
    let (g, _) = build_entity_projection(records, entity_of, attr_of);
    let mut t = 0u64;
    for n in g.node_indices() {
        let neighbors: Vec<NodeIndex> = g.neighbors(n).collect();
        let k = neighbors.len();
        for i in 0..k {
            for j in (i + 1)..k {
                if g.find_edge(neighbors[i], neighbors[j]).is_some() {
                    t += 1;
                }
            }
        }
    }
    t / 3
}

pub fn triangle_log_ratio_gap(real: u64, syn: u64) -> f64 {
    let lr = ((real + 1) as f64) / ((syn + 1) as f64);
    lr.ln().abs()
}

#[cfg(test)]
mod cluster_tests {
    use super::super::ietd::source_of;
    use super::*;
    use chrono::NaiveDate;

    fn rec(src: &str, gl: &str) -> Record {
        let d = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        Record {
            source: src.into(),
            gl_account: gl.into(),
            cost_center: None,
            profit_center: None,
            trading_partner: None,
            je_number: format!("J{src}{gl}"),
            je_line_number: "001".into(),
            effective_date: d,
            entry_date: d,
            created_at: None,
            functional_amount: 1.0,
            header_text: String::new(),
            line_text: String::new(),
        }
    }

    #[test]
    fn triangle_in_three_entity_ring() {
        let rs = vec![rec("A", "X"), rec("B", "X"), rec("C", "X")];
        let t = triangle_count(&rs, source_of, gl_account_of);
        assert_eq!(t, 1);
        let cc = clustering_coefficient(&rs, source_of, gl_account_of);
        assert!((cc - 1.0).abs() < 1e-9);
    }

    #[test]
    fn no_shared_attribute_no_edges() {
        let rs = vec![rec("A", "1"), rec("B", "2"), rec("C", "3")];
        assert_eq!(triangle_count(&rs, source_of, gl_account_of), 0);
        assert_eq!(clustering_coefficient(&rs, source_of, gl_account_of), 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::super::ietd::source_of;
    use super::*;
    use chrono::NaiveDate;

    fn rec(src: &str, gl: &str) -> Record {
        let d = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        Record {
            source: src.into(),
            gl_account: gl.into(),
            cost_center: None,
            profit_center: None,
            trading_partner: None,
            je_number: format!("J{src}{gl}"),
            je_line_number: "001".into(),
            effective_date: d,
            entry_date: d,
            created_at: None,
            functional_amount: 1.0,
            header_text: String::new(),
            line_text: String::new(),
        }
    }

    #[test]
    fn fanout_distribution_basic() {
        let rs = vec![rec("A", "100"), rec("A", "200"), rec("B", "100")];
        let mut fo = fanout_distribution(&rs, source_of, gl_account_of);
        fo.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(fo, vec![1.0, 2.0]);
    }
}
