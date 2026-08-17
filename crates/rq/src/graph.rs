//! `parents:` — the declared dependency graph.
//!
//! Every other tool in the category chains through collection ordering or an external
//! workflow file. Here each request states what it needs, and `rq` walks the graph: a DAG
//! you declare, not a script you write. A parent shared by two children runs once.

use anyhow::{bail, Result};

use crate::project::{Kind, Project};

/// Resolve `target` and its ancestry into an execution order: dependencies first, each
/// request at most once, in a stable order (declaration order, depth first).
pub fn plan(project: &Project, target: usize) -> Result<Vec<usize>> {
    let mut order = Vec::new();
    let mut done = Vec::new();
    let mut stack = Vec::new();
    visit(project, target, &mut order, &mut done, &mut stack)?;
    Ok(order)
}

fn visit(
    project: &Project,
    idx: usize,
    order: &mut Vec<usize>,
    done: &mut Vec<usize>,
    stack: &mut Vec<usize>,
) -> Result<()> {
    if done.contains(&idx) {
        return Ok(());
    }
    if let Some(at) = stack.iter().position(|i| *i == idx) {
        let mut cycle: Vec<&str> = stack[at..]
            .iter()
            .map(|i| project.entries[*i].rel.as_str())
            .collect();
        cycle.push(project.entries[idx].rel.as_str());
        bail!(
            "`parents:` forms a cycle: {}\n  a request cannot depend on itself, directly or \
             through its parents",
            cycle.join(" → ")
        );
    }

    stack.push(idx);
    let (doc, _) = project.load(idx)?;
    for parent in &doc.front.parents {
        let p = resolve_relative(project, idx, parent)?;
        visit(project, p, order, done, stack)?;
    }
    stack.pop();

    done.push(idx);
    order.push(idx);
    Ok(())
}

/// Resolve a `parents:` entry. A bare name is looked up as a sibling first, then upward
/// through the enclosing collections, then across the project — the scoping rule people
/// expect from directories, so `login` means "the login next to me" when there is one.
pub fn resolve_relative(project: &Project, from: usize, name: &str) -> Result<usize> {
    let name = name.trim().trim_matches('/');
    if name.is_empty() {
        bail!("{}: empty entry in `parents:`", project.entries[from].rel);
    }

    let mut scopes: Vec<String> = Vec::new();
    let mut cur = project.entries[from].parent;
    while let Some(i) = cur {
        scopes.push(project.entries[i].rel.clone());
        cur = project.entries[i].parent;
    }
    scopes.push(String::new());

    for scope in scopes {
        let candidate = if scope.is_empty() {
            name.to_string()
        } else {
            format!("{scope}/{name}")
        };
        if let Some(i) = project
            .entries
            .iter()
            .position(|e| e.kind == Kind::Request && e.rel == candidate)
        {
            if i == from {
                continue;
            }
            return Ok(i);
        }
    }

    project
        .resolve(name)
        .map_err(|e| anyhow::anyhow!("{}: `parents: [{name}]` — {e}", project.entries[from].rel))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project;
    use std::path::Path;

    fn write(root: &Path, rel: &str, parents: &[&str]) {
        let path = root.join(format!("{rel}.md"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let list = parents
            .iter()
            .map(|p| format!("\"{p}\""))
            .collect::<Vec<_>>()
            .join(", ");
        std::fs::write(
            &path,
            format!("---\nurl: https://x.test/{rel}\nparents: [{list}]\n---\n"),
        )
        .unwrap();
    }

    fn project_with(files: &[(&str, &[&str])]) -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        project::init(dir.path()).unwrap();
        for (rel, parents) in files {
            write(dir.path(), rel, parents);
        }
        let p = Project::open(dir.path().to_path_buf()).unwrap();
        (dir, p)
    }

    fn names(p: &Project, order: Vec<usize>) -> Vec<String> {
        order
            .into_iter()
            .map(|i| p.entries[i].rel.clone())
            .collect()
    }

    #[test]
    fn parents_run_first() {
        let (_d, p) = project_with(&[("login", &[]), ("me", &["login"])]);
        let order = plan(&p, p.resolve("me").unwrap()).unwrap();
        assert_eq!(names(&p, order), vec!["login", "me"]);
    }

    #[test]
    fn a_shared_parent_runs_once() {
        let (_d, p) = project_with(&[
            ("login", &[]),
            ("me", &["login"]),
            ("repos", &["login", "me"]),
        ]);
        let order = plan(&p, p.resolve("repos").unwrap()).unwrap();
        assert_eq!(names(&p, order), vec!["login", "me", "repos"]);
    }

    #[test]
    fn a_cycle_is_reported_with_its_path() {
        let (_d, p) = project_with(&[("a", &["b"]), ("b", &["a"])]);
        let err = plan(&p, p.resolve("a").unwrap()).unwrap_err().to_string();
        assert!(err.contains("cycle"), "{err}");
        assert!(
            err.contains("a → b → a") || err.contains("b → a → b"),
            "{err}"
        );
    }

    #[test]
    fn a_bare_name_prefers_the_sibling() {
        let (_d, p) = project_with(&[
            ("login", &[]),
            ("github/login", &[]),
            ("github/me", &["login"]),
        ]);
        let order = plan(&p, p.resolve("github/me").unwrap()).unwrap();
        assert_eq!(names(&p, order), vec!["github/login", "github/me"]);
    }

    #[test]
    fn an_unknown_parent_names_the_request_that_declared_it() {
        let (_d, p) = project_with(&[("me", &["nope"])]);
        let err = plan(&p, p.resolve("me").unwrap()).unwrap_err().to_string();
        assert!(err.contains("me:"), "{err}");
        assert!(err.contains("nope"), "{err}");
    }
}
