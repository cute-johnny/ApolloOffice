use crate::ai::{ask, p as P, pj, wrap, Ask};
use crate::app::{drop, Ax, X, R};
use crate::db::{Job, Project};
use crate::pipe::{answers_context, ok, prep, push_branch, rn, rp, rules, trs, use_, x};
use base64::Engine;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Clone, Debug)]
pub struct MilestoneSpec {
    pub idx: i64,
    pub title: String,
    pub description: String,
    pub depends_on_idx: Vec<i64>,
    pub visual: bool,
    pub acceptance: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ValidPlan {
    pub summary: String,
    pub stack: String,
    pub setup_cmd: String,
    pub ci_cmd: String,
    pub milestones: Vec<MilestoneSpec>,
}

/// Validates the Architect JSON response and topologically orders its milestone DAG.
pub fn validate_plan(js: &Value) -> Result<ValidPlan, String> {
    let Some(obj) = js.as_object() else {
        return Err("Architect response is not a JSON object".into());
    };
    let summary = obj
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if summary.is_empty() {
        return Err("missing non-empty `summary`".into());
    }
    let stack = obj
        .get("stack")
        .and_then(|v| v.as_str())
        .unwrap_or("Unspecified")
        .trim()
        .to_string();
    let setup_cmd = obj
        .get("setup_cmd")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let ci_cmd = obj
        .get("ci_cmd")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let raw_ms = obj
        .get("milestones")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "missing `milestones` array".to_string())?;
    if raw_ms.is_empty() {
        return Err("`milestones` array cannot be empty".into());
    }
    if raw_ms.len() > 20 {
        return Err("`milestones` exceeds maximum of 20 milestones".into());
    }

    let mut by_idx: BTreeMap<i64, MilestoneSpec> = BTreeMap::new();
    for (pos, m) in raw_ms.iter().enumerate() {
        let idx = m["idx"].as_i64().unwrap_or((pos + 1) as i64);
        if idx <= 0 {
            return Err(format!("milestone index must be positive (got {idx})"));
        }
        if by_idx.contains_key(&idx) {
            return Err(format!("duplicate milestone idx {idx}"));
        }
        let title = m["title"].as_str().unwrap_or("").trim().to_string();
        if title.is_empty() {
            return Err(format!("milestone #{idx} missing `title`"));
        }
        let description = m["description"].as_str().unwrap_or("").trim().to_string();
        if description.is_empty() {
            return Err(format!("milestone #{idx} missing `description`"));
        }
        let depends_on_idx: Vec<i64> = m["depends_on_idx"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| v.as_i64())
            .collect();
        let visual = m["visual"].as_bool().unwrap_or(false);
        let acceptance: Vec<String> = m["acceptance"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect();
        by_idx.insert(
            idx,
            MilestoneSpec {
                idx,
                title,
                description,
                depends_on_idx,
                visual,
                acceptance,
            },
        );
    }

    let mut indeg: BTreeMap<i64, usize> = BTreeMap::new();
    let mut adj: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    for (&idx, m) in &by_idx {
        indeg.entry(idx).or_insert(0);
        let mut seen_deps = BTreeSet::new();
        for &dep in &m.depends_on_idx {
            if dep == idx {
                return Err(format!("milestone #{idx} cannot depend on itself"));
            }
            if !by_idx.contains_key(&dep) {
                return Err(format!(
                    "milestone #{idx} depends on unknown milestone #{dep}"
                ));
            }
            if seen_deps.insert(dep) {
                adj.entry(dep).or_default().push(idx);
                *indeg.entry(idx).or_insert(0) += 1;
            }
        }
    }

    // Kahn's topological sort breaking ties deterministically by lowest idx.
    let mut ready: BTreeSet<i64> = indeg
        .iter()
        .filter_map(|(&idx, &deg)| if deg == 0 { Some(idx) } else { None })
        .collect();
    let mut ordered = Vec::with_capacity(by_idx.len());
    while let Some(idx) = ready.pop_first() {
        let m = by_idx.get(&idx).expect("checked key").clone();
        ordered.push(m);
        if let Some(children) = adj.get(&idx) {
            for &ch in children {
                if let Some(deg) = indeg.get_mut(&ch) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        ready.insert(ch);
                    }
                }
            }
        }
    }
    if ordered.len() != by_idx.len() {
        return Err("milestone dependency graph contains a cycle".into());
    }

    Ok(ValidPlan {
        summary,
        stack,
        setup_cmd,
        ci_cmd,
        milestones: ordered,
    })
}

pub fn render_plan_md(proj: &Project, plan: &ValidPlan) -> String {
    let mut out = format!(
        "# Architecture Plan: {}\n\n- **Project ID:** {}\n- **Repository:** `{}`\n- **Epic branch:** `{}`\n- **Stack:** {}\n- **Setup command:** {}\n- **Verification (CI) command:** {}\n\n## Summary\n\n{}\n\n## Milestones\n\n",
        proj.title,
        proj.id,
        proj.repo,
        proj.epic_branch,
        plan.stack,
        if plan.setup_cmd.is_empty() {
            "(none)".to_string()
        } else {
            format!("`{}`", plan.setup_cmd)
        },
        if plan.ci_cmd.is_empty() {
            "(none)".to_string()
        } else {
            format!("`{}`", plan.ci_cmd)
        },
        plan.summary
    );
    for m in &plan.milestones {
        let deps = if m.depends_on_idx.is_empty() {
            "none".to_string()
        } else {
            m.depends_on_idx
                .iter()
                .map(|d| format!("M{d}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        out.push_str(&format!(
            "### M{}: {}\n- **Depends on:** {}\n- **Visual UI inspection:** {}\n\n{}\n",
            m.idx,
            m.title,
            deps,
            if m.visual { "yes" } else { "no" },
            m.description
        ));
        if !m.acceptance.is_empty() {
            out.push_str("\n**Acceptance criteria:**\n");
            for ac in &m.acceptance {
                out.push_str(&format!("- {ac}\n"));
            }
        }
        out.push('\n');
    }
    out
}

async fn write_work_file(a: &Ax, j: &Job, rel: &str, content: &str) -> R<()> {
    let b64 = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
    let dir = std::path::Path::new(rel)
        .parent()
        .map(|p| p.display().to_string().replace('\\', "/"))
        .unwrap_or_else(|| ".".into());
    let script = format!(
        "mkdir -p \"{dir}\" && base64 -d > \"{rel}\" <<'APOLLO_B64'\n{b64}\nAPOLLO_B64\n"
    );
    let r = x(a, j, &script, 120).await;
    if r.c != 0 {
        return Err(X::E(format!(
            "failed writing {rel}: {}",
            crate::sh::last(if r.e.is_empty() { &r.o } else { &r.e }, 240)
        )));
    }
    Ok(())
}

/// Runs the Architect job (`kind = "architect"`), producing `.apollo/spec.md`,
/// `.apollo/plan.md`, `.apollo/milestones.json`, pushing `epic/<slug>`, and
/// enqueuing topological milestone `build` jobs.
pub async fn run(a: &Ax, j: &mut Job) -> R<()> {
    let pid = j
        .project_id
        .ok_or_else(|| X::E("architect job missing project_id".into()))?;
    let proj = a
        .db
        .pg(pid)
        .ok_or_else(|| X::E(format!("project #{pid} not found")))?;
    let epic_br = if proj.epic_branch.trim().is_empty() {
        format!("epic/project-{pid}")
    } else {
        proj.epic_branch.clone()
    };

    prep(a, j).await?;
    let mt = crate::gh::meta(a, &j.repo).await.map_err(X::E)?;
    if !crate::pipe::gitb().starts_with("file:") && !mt["push"].as_bool().unwrap_or(false) {
        return drop("You do not have push access to create an epic branch on this repository");
    }

    a.db.jset(j.id, &[("st", json!("plan"))]);
    a.st("architect", "work", Some(j.id), "");
    a.say(
        "architect",
        &format!("planning architecture & milestones for project #{pid}: {}", proj.title),
        Some(j.id),
    );

    let spec_text = if proj.raw_spec.trim().is_empty() {
        proj.title.clone()
    } else {
        proj.raw_spec.clone()
    };
    write_work_file(a, j, ".apollo/spec.md", &spec_text).await?;

    let tree = ok(
        a,
        j,
        "find . -maxdepth 3 -not -path '*/.git*' -not -path './node_modules*' -not -path './target*' | sort | head -120",
        60,
    )
    .await
    .unwrap_or_default();
    let repo_cfg = rp(a, &j.repo);
    let existing_setup = repo_cfg["setup"].as_str().unwrap_or("").trim();
    let existing_ci = repo_cfg["ci"].as_str().unwrap_or("").trim();

    let spec_section = if spec_text.len() <= 32_000 {
        wrap("specification", &spec_text)
    } else {
        let lines = spec_text.lines().count();
        format!(
            "Specification is large ({} bytes, {lines} lines) and saved completely at `.apollo/spec.md` in the repository root. Use `read` / `grep` on `.apollo/spec.md` to inspect all sections.\n\n{}",
            spec_text.len(),
            wrap("specification-excerpt", &crate::sh::head(&spec_text, 24_000))
        )
    };

    let base_prompt = format!(
        "Project #{pid}: {}\nRepository: {}\nEpic integration branch: {epic_br} (branched from {})\nExisting configured setup command: {}\nExisting configured CI command: {}\n\nRepository files:\n{}\n\n{}{}{}",
        proj.title,
        j.repo,
        j.s("mn"),
        if existing_setup.is_empty() { "(none yet — propose setup_cmd)" } else { existing_setup },
        if existing_ci.is_empty() { "(none yet — propose ci_cmd)" } else { existing_ci },
        wrap("repo-files", &tree),
        spec_section,
        rules(a, j),
        answers_context(j),
    );

    let md = a.md("architect");
    use_(a, j, &md);

    let mut parsed: Option<(ValidPlan, Value)> = None;
    let mut repair_note = String::new();
    for attempt in 0..2 {
        let prompt = if repair_note.is_empty() {
            base_prompt.clone()
        } else {
            format!("{base_prompt}\n\nPrevious plan output failed validation:\n{repair_note}\nReturn valid JSON matching the schema.")
        };
        let mut o = Ask::new("architect", md.clone(), P("architect"), prompt);
        o.job = Some(j.id);
        o.rn = rn(a, j);
        o.turns = 40;
        o.ef = Some("high");
        o.t = 60 * 60;
        let (txt, mut js) = ask(a, o).await?;
        if js.is_null() {
            js = pj(&txt);
        }
        match validate_plan(&js) {
            Ok(plan) => {
                parsed = Some((plan, js));
                break;
            }
            Err(err) => {
                if attempt == 1 {
                    a.st("architect", "idle", None, "sad");
                    return drop(format!("Architect produced an invalid milestone plan: {err}"));
                }
                repair_note = err;
            }
        }
    }

    let (plan, raw_plan_json) = parsed.expect("loop returns or sets parsed");

    // Materialize spec, plan markdown, and canonical JSON onto epic branch.
    let plan_md = render_plan_md(&proj, &plan);
    let milestones_json = serde_json::to_string_pretty(&json!({
        "project_id": pid,
        "title": proj.title,
        "epic_branch": epic_br,
        "summary": plan.summary,
        "stack": plan.stack,
        "setup_cmd": plan.setup_cmd,
        "ci_cmd": plan.ci_cmd,
        "milestones": raw_plan_json["milestones"],
    }))
    .unwrap_or_else(|_| "{}".into());

    write_work_file(a, j, ".apollo/plan.md", &plan_md).await?;
    write_work_file(a, j, ".apollo/milestones.json", &milestones_json).await?;

    let mut commit_msg = format!("chore(apollo): architect plan for {}", proj.title);
    let trailers = trs(j);
    if !trailers.is_empty() {
        commit_msg.push_str("\n\n");
        commit_msg.push_str(&trailers.join("\n"));
    }
    ok(
        a,
        j,
        "git add -f .apollo/spec.md .apollo/plan.md .apollo/milestones.json",
        60,
    )
    .await?;
    let dt = ok(a, j, "git status --porcelain", 60)
        .await
        .unwrap_or_default();
    if !dt.trim().is_empty() {
        ok(
            a,
            j,
            &format!("git commit -qm {}", crate::sh::q(&commit_msg)),
            60,
        )
        .await?;
    }

    push_branch(a, j, &epic_br, &j.s("base"), &j.repo, "--force").await?;

    a.db.pset(
        pid,
        &[
            ("epic_branch", json!(epic_br)),
            ("arch_json", raw_plan_json.clone()),
            ("st", json!("running")),
        ],
    );

    let total = plan.milestones.len() as i64;
    let mut idx_to_job: HashMap<i64, i64> = HashMap::new();
    for m in &plan.milestones {
        let deps: Vec<i64> = m
            .depends_on_idx
            .iter()
            .filter_map(|dep_idx| idx_to_job.get(dep_idx).copied())
            .collect();
        let acceptance_md = if m.acceptance.is_empty() {
            String::new()
        } else {
            format!(
                "\n\nAcceptance criteria:\n{}",
                m.acceptance
                    .iter()
                    .map(|ac| format!("- {ac}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };
        let body = format!(
            "Project: {} (#{pid})\nMilestone {}/{total}: {}\nTech stack: {}\n\nArchitecture summary:\n{}\n\nMilestone specification:\n{}{}\n\nNote: Full specification is available at `.apollo/spec.md` and milestone overview at `.apollo/plan.md`.",
            proj.title,
            m.idx,
            m.title,
            plan.stack,
            plan.summary,
            m.description,
            acceptance_md,
        );
        let d = json!({
            "desk": 1,
            "ext": {"src": "architect"},
            "title": m.title,
            "body": body,
            "feat": true,
            "cx": "complex",
            "visual": m.visual,
            "acceptance": m.acceptance,
            "stack": plan.stack,
            "arch_setup": plan.setup_cmd,
            "arch_ci": plan.ci_cmd,
            "milestone_total": total,
            "project_id": pid,
        });
        let mid = a.db.jnew_proj(
            &j.repo,
            0,
            "build",
            &format!("M{}: {}", m.idx, m.title),
            &j.url,
            "ready",
            &format!("project #{pid} milestone {}/{total}", m.idx),
            d,
            None,
            -1.0 + (m.idx as f64) * 0.01,
            Some(pid),
            m.idx,
            &deps,
            &epic_br,
        );
        idx_to_job.insert(m.idx, mid);
    }

    a.db.jset(
        j.id,
        &[
            ("st", json!("done")),
            (
                "why",
                json!(format!("{total} milestone(s) planned on {epic_br}")),
            ),
        ],
    );
    a.st("architect", "idle", None, "ok");
    a.say(
        "architect",
        &format!("planned {total} milestone(s) on {epic_br} for project #{pid}"),
        Some(j.id),
    );
    a.sw.notify_one();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_plan_orders_acyclic_dag_and_rejects_cycles_or_missing() {
        let valid = json!({
            "summary": "Full-stack bookmarks app",
            "stack": "Node + Express + SQLite + Vanilla JS",
            "setup_cmd": "npm ci",
            "ci_cmd": "npm test",
            "milestones": [
                {"idx": 3, "title": "Frontend UI", "description": "Build SPA UI", "depends_on_idx": [1, 2], "visual": true, "acceptance": ["Page renders", "Form saves"]},
                {"idx": 1, "title": "Bootstrap & DB", "description": "Init Express + SQLite", "depends_on_idx": [], "visual": false, "acceptance": ["GET /health -> 200"]},
                {"idx": 2, "title": "API CRUD", "description": "Bookmarks REST endpoints", "depends_on_idx": [1], "visual": false, "acceptance": ["POST /api/bookmarks returns 201"]}
            ]
        });
        let p = validate_plan(&valid).expect("valid DAG");
        assert_eq!(p.milestones.len(), 3);
        assert_eq!(
            p.milestones.iter().map(|m| m.idx).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(p.milestones[2].visual);
        assert_eq!(p.ci_cmd, "npm test");

        // Rejects cycle 1 -> 2 -> 1
        let cycle = json!({
            "summary": "Broken cycle",
            "milestones": [
                {"idx": 1, "title": "A", "description": "A", "depends_on_idx": [2]},
                {"idx": 2, "title": "B", "description": "B", "depends_on_idx": [1]}
            ]
        });
        assert!(validate_plan(&cycle)
            .unwrap_err()
            .contains("cycle"));

        // Rejects unknown dependency
        let missing_dep = json!({
            "summary": "Missing dep",
            "milestones": [
                {"idx": 1, "title": "A", "description": "A", "depends_on_idx": [99]}
            ]
        });
        assert!(validate_plan(&missing_dep)
            .unwrap_err()
            .contains("unknown milestone"));

        // Rejects empty milestones
        let empty = json!({"summary": "Empty", "milestones": []});
        assert!(validate_plan(&empty).is_err());
    }
}
