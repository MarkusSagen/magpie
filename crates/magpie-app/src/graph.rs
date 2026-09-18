//! Force-directed layout of the note link graph ([[wiki-links]]). Deterministic
//! (no RNG): nodes seed on a circle by index, then a Fruchterman-Reingold-style
//! relaxation. Positions are normalized to [0,1]. Pure + testable.
use crate::notes_view::wiki_links;

#[derive(Debug, Clone)]
pub struct GraphNode {
    pub id: i64,
    pub title: String,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct GraphEdge {
    pub a: usize,
    pub b: usize,
} // indices into the returned nodes

/// `notes`: (id, name, body). Builds nodes for notes that participate in at least one
/// resolvable [[link]] (orphans excluded to keep the graph legible), edges (undirected,
/// deduped, no self-loops), and runs `iterations` of relaxation. Caps at 150 highest-
/// degree nodes. Returns (nodes, edges) with node positions in [0,1].
pub fn build_graph(
    notes: &[(i64, String, String)],
    iterations: usize,
) -> (Vec<GraphNode>, Vec<GraphEdge>) {
    // Map lowercased+trimmed note name -> index into `notes`.
    let name_to_idx: std::collections::HashMap<String, usize> = notes
        .iter()
        .enumerate()
        .map(|(i, (_, name, _))| (name.trim().to_lowercase(), i))
        .collect();

    // Undirected, deduped edges keyed by (min, max) index into `notes`.
    let mut edge_set: std::collections::BTreeSet<(usize, usize)> =
        std::collections::BTreeSet::new();
    for (i, (_, _, body)) in notes.iter().enumerate() {
        for target in wiki_links(body) {
            let key = target.trim().to_lowercase();
            if let Some(&j) = name_to_idx.get(&key) {
                if i == j {
                    continue; // no self-loops
                }
                let edge = if i < j { (i, j) } else { (j, i) };
                edge_set.insert(edge);
            }
        }
    }

    // Degree per note index (into `notes`).
    let mut degree: Vec<usize> = vec![0; notes.len()];
    for &(a, b) in &edge_set {
        degree[a] += 1;
        degree[b] += 1;
    }

    // Keep only notes with degree >= 1.
    let mut kept: Vec<usize> = (0..notes.len()).filter(|&i| degree[i] > 0).collect();

    // Cap at 150 highest-degree nodes (stable tie-break by id).
    if kept.len() > 150 {
        kept.sort_by(|&a, &b| {
            degree[b]
                .cmp(&degree[a])
                .then_with(|| notes[a].0.cmp(&notes[b].0))
        });
        kept.truncate(150);
        kept.sort_unstable();
    }

    // Re-index: old note index -> new node index (only for kept notes).
    let mut reindex: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for (new_idx, &old_idx) in kept.iter().enumerate() {
        reindex.insert(old_idx, new_idx);
    }

    let n = kept.len();
    let mut nodes: Vec<GraphNode> = Vec::with_capacity(n);
    for &old_idx in &kept {
        let (id, name, _) = &notes[old_idx];
        nodes.push(GraphNode {
            id: *id,
            title: name.clone(),
            x: 0.0,
            y: 0.0,
        });
    }

    let mut edges: Vec<GraphEdge> = Vec::new();
    for &(a, b) in &edge_set {
        if let (Some(&na), Some(&nb)) = (reindex.get(&a), reindex.get(&b)) {
            edges.push(GraphEdge { a: na, b: nb });
        }
    }

    if n == 0 {
        return (nodes, edges);
    }

    // Seed positions on a circle.
    let two_pi = std::f32::consts::PI * 2.0;
    for (i, node) in nodes.iter_mut().enumerate() {
        let theta = two_pi * (i as f32) / (n as f32);
        node.x = 0.5 + 0.4 * theta.cos();
        node.y = 0.5 + 0.4 * theta.sin();
    }

    if n > 1 {
        relax(&mut nodes, &edges, iterations);
    }

    normalize(&mut nodes);

    (nodes, edges)
}

/// Fruchterman-Reingold-style relaxation: repulsion between all pairs, attraction
/// along edges, displacement capped by a cooling temperature.
fn relax(nodes: &mut [GraphNode], edges: &[GraphEdge], iterations: usize) {
    let n = nodes.len() as f32;
    let k = 1.0 / n.sqrt();
    let eps = 1e-6_f32;
    let mut temp = 0.1_f32; // initial max displacement per iteration, in [0,1]-space

    for iter in 0..iterations {
        let mut disp: Vec<(f32, f32)> = vec![(0.0, 0.0); nodes.len()];

        // Repulsive force between all pairs.
        for i in 0..nodes.len() {
            for j in (i + 1)..nodes.len() {
                let mut dx = nodes[i].x - nodes[j].x;
                let mut dy = nodes[i].y - nodes[j].y;
                let mut dist = (dx * dx + dy * dy).sqrt();
                if dist < eps {
                    // Deterministic nudge based on indices, not RNG.
                    let a = (i as f32 + 1.0) * 0.0001;
                    let b = (j as f32 + 1.0) * 0.0001;
                    dx = a - b + eps;
                    dy = b - a + eps;
                    dist = (dx * dx + dy * dy).sqrt().max(eps);
                }
                let force = (k * k) / dist;
                let fx = (dx / dist) * force;
                let fy = (dy / dist) * force;
                disp[i].0 += fx;
                disp[i].1 += fy;
                disp[j].0 -= fx;
                disp[j].1 -= fy;
            }
        }

        // Attractive force along edges.
        for e in edges {
            let mut dx = nodes[e.a].x - nodes[e.b].x;
            let mut dy = nodes[e.a].y - nodes[e.b].y;
            let mut dist = (dx * dx + dy * dy).sqrt();
            if dist < eps {
                let a = (e.a as f32 + 1.0) * 0.0001;
                let b = (e.b as f32 + 1.0) * 0.0001;
                dx = a - b + eps;
                dy = b - a + eps;
                dist = (dx * dx + dy * dy).sqrt().max(eps);
            }
            let force = (dist * dist) / k;
            let fx = (dx / dist) * force;
            let fy = (dy / dist) * force;
            disp[e.a].0 -= fx;
            disp[e.a].1 -= fy;
            disp[e.b].0 += fx;
            disp[e.b].1 += fy;
        }

        // Apply displacement, capped by temp, clamp to [0,1].
        for (i, node) in nodes.iter_mut().enumerate() {
            let (dx, dy) = disp[i];
            let dist = (dx * dx + dy * dy).sqrt().max(eps);
            let capped = dist.min(temp);
            node.x = (node.x + (dx / dist) * capped).clamp(0.0, 1.0);
            node.y = (node.y + (dy / dist) * capped).clamp(0.0, 1.0);
        }

        // Cool down linearly toward 0 over the run.
        let progress = (iter + 1) as f32 / iterations as f32;
        temp = (0.1 * (1.0 - progress)).max(0.001);
    }
}

/// Rescale positions to fit within [0.05, 0.95] on both axes. Degenerate (all
/// points coincide) leaves them centered.
fn normalize(nodes: &mut [GraphNode]) {
    if nodes.is_empty() {
        return;
    }
    if nodes.len() == 1 {
        nodes[0].x = 0.5;
        nodes[0].y = 0.5;
        return;
    }
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for n in nodes.iter() {
        min_x = min_x.min(n.x);
        max_x = max_x.max(n.x);
        min_y = min_y.min(n.y);
        max_y = max_y.max(n.y);
    }
    let span_x = max_x - min_x;
    let span_y = max_y - min_y;
    const LO: f32 = 0.05;
    const HI: f32 = 0.95;
    for n in nodes.iter_mut() {
        n.x = if span_x > 1e-6 {
            LO + (n.x - min_x) / span_x * (HI - LO)
        } else {
            0.5
        };
        n.y = if span_y > 1e-6 {
            LO + (n.y - min_y) / span_y * (HI - LO)
        } else {
            0.5
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triples(pairs: &[(i64, &str, &str)]) -> Vec<(i64, String, String)> {
        pairs
            .iter()
            .map(|(id, name, body)| (*id, name.to_string(), body.to_string()))
            .collect()
    }

    #[test]
    fn builds_nodes_and_edges_excluding_orphans() {
        let notes = triples(&[
            (1, "A", "links [[B]]"),
            (2, "B", "links [[C]]"),
            (3, "C", "no links"),
            (4, "D", "an orphan, no links in or out"),
        ]);
        let (nodes, edges) = build_graph(&notes, 50);
        assert_eq!(nodes.len(), 3);
        assert_eq!(edges.len(), 2);
        assert!(!nodes.iter().any(|n| n.title == "D"));
        for n in &nodes {
            assert!((0.0..=1.0).contains(&n.x));
            assert!((0.0..=1.0).contains(&n.y));
        }
    }

    #[test]
    fn self_link_produces_no_self_edge() {
        let notes = triples(&[(1, "A", "self [[A]] and [[B]]"), (2, "B", "nothing")]);
        let (nodes, edges) = build_graph(&notes, 50);
        assert_eq!(nodes.len(), 2);
        assert_eq!(edges.len(), 1);
        assert_ne!(edges[0].a, edges[0].b);
    }

    #[test]
    fn unresolved_link_produces_no_edge_or_extra_node() {
        let notes = triples(&[(1, "A", "see [[Ghost]]")]);
        let (nodes, edges) = build_graph(&notes, 50);
        assert!(nodes.is_empty());
        assert!(edges.is_empty());
    }

    #[test]
    fn deterministic_across_runs() {
        let notes = triples(&[
            (1, "A", "[[B]] [[C]]"),
            (2, "B", "[[A]] [[C]]"),
            (3, "C", "[[A]]"),
            (4, "E", "[[F]]"),
            (5, "F", "[[E]]"),
        ]);
        let (nodes1, edges1) = build_graph(&notes, 100);
        let (nodes2, edges2) = build_graph(&notes, 100);
        assert_eq!(nodes1.len(), nodes2.len());
        for (n1, n2) in nodes1.iter().zip(nodes2.iter()) {
            assert_eq!(n1.id, n2.id);
            assert_eq!(n1.x, n2.x);
            assert_eq!(n1.y, n2.y);
        }
        assert_eq!(edges1.len(), edges2.len());
    }
}
