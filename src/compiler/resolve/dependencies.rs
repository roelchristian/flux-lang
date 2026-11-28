//! Dependency ordering and cycle diagnostics.

use super::{decl_name, SymbolKind};
use crate::{parser::Program, Error};
use std::collections::BTreeSet;

/// Iterative Kosaraju so a long dependency chain does not grow the Rust stack.
pub(super) fn groups(edges: &[BTreeSet<usize>], p: &Program) -> Result<Vec<Vec<usize>>, Error> {
    let n = edges.len();
    let mut reverse = vec![Vec::new(); n];
    for (i, ds) in edges.iter().enumerate() {
        for &j in ds {
            reverse[j].push(i);
        }
    }
    let mut seen = vec![false; n];
    let mut postorder = Vec::with_capacity(n);
    for i in 0..n {
        if seen[i] {
            continue;
        }
        seen[i] = true;
        let mut stack = vec![(i, edges[i].iter())];
        while let Some((j, children)) = stack.last_mut() {
            if let Some(&k) = children.next() {
                if !seen[k] {
                    seen[k] = true;
                    stack.push((k, edges[k].iter()));
                }
            } else {
                postorder.push(*j);
                stack.pop();
            }
        }
    }
    seen.fill(false);
    let mut components = Vec::new();
    for i in postorder.into_iter().rev() {
        if seen[i] {
            continue;
        }
        seen[i] = true;
        let mut stack = vec![i];
        let mut component = Vec::new();
        while let Some(j) = stack.pop() {
            component.push(j);
            for &k in &reverse[j] {
                if !seen[k] {
                    seen[k] = true;
                    stack.push(k);
                }
            }
        }
        component.sort_unstable();
        if component.len() > 1 || edges[i].contains(&i) {
            if let Some(&v) = component
                .iter()
                .find(|&&j| !matches!(decl_name(&p.defs[j]), Some((_, SymbolKind::Callable(_), _))))
            {
                let names = component
                    .iter()
                    .filter_map(|&j| decl_name(&p.defs[j]).map(|(n, _, _)| n))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(Error::new(
                    p.defs[v].start(),
                    format!("cyclic value dependency involving {names}"),
                ));
            }
        }
        components.push(component);
    }
    components.reverse();
    Ok(components)
}
