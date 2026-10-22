//! A skill loaded at invoke time (progressive disclosure: discovery reads only the frontmatter).

use std::path::{Path, PathBuf};

use crate::template::{self, TemplateVars};
use crate::{forge_native, frontmatter, SkillMeta};

/// A fully-loaded skill: the methodology body plus any readable bundled resources. Built by
/// [`Skill::load`] at invoke time only.
#[derive(Debug, Clone)]
pub struct Skill {
    pub meta: SkillMeta,
    pub body: String,
    /// `(name, contents)` for each resource that loaded; missing ones are skipped + warned.
    pub resources: Vec<(String, String)>,
    pub warnings: Vec<String>,
}

impl Skill {
    /// Read the skill body + resources from disk (progressive disclosure). Never fails: a
    /// missing/unreadable resource is recorded as a warning and the skill still runs.
    pub fn load(meta: &SkillMeta) -> Skill {
        let mut warnings = Vec::new();
        // A built-in skill carries its body inline (no dir to read); on-disk skills read SKILL.md.
        let body = match &meta.body {
            Some(b) => b.clone(),
            None => match std::fs::read_to_string(meta.dir.join("SKILL.md")) {
                Ok(raw) => frontmatter::split(&raw).1.trim().to_string(),
                Err(e) => {
                    warnings.push(format!("skill {}: cannot read SKILL.md ({e})", meta.name));
                    String::new()
                }
            },
        };
        let mut resources = Vec::new();
        for res in &meta.resources {
            let Some(path) = safe_resource_path(&meta.dir, res) else {
                warnings.push(format!(
                    "skill {}: resource path escapes skill dir: {res}",
                    meta.name
                ));
                continue;
            };
            match std::fs::read_to_string(path) {
                Ok(c) => resources.push((res.clone(), c)),
                Err(_) => warnings.push(format!("skill {}: missing resource {res}", meta.name)),
            }
        }
        Skill {
            meta: meta.clone(),
            body,
            resources,
            warnings,
        }
    }

    /// The full guidance block injected into the turn: the methodology body followed by each
    /// loaded resource under a header. Rebranded so an imported skill presents as Forge-native
    /// (see [`forge_native`]). `${CLAUDE_SKILL_DIR}` points at this skill's directory.
    pub fn guidance(&self) -> String {
        self.render(None, &TemplateVars::default())
    }

    /// [`Skill::guidance`] with the invocation's arguments (`$ARGUMENTS`, `$0`, `$name`, …) and
    /// session variables substituted into the body. `args` is `None` when the model loaded the
    /// skill itself and there is no typed argument text; argument placeholders then stay as
    /// written. Resources are appended verbatim.
    pub fn render(&self, args: Option<&str>, vars: &TemplateVars) -> String {
        let vars = vars.for_skill_dir(&self.meta.dir);
        // Rebrand the authored text first so user-typed arguments are never rewritten.
        let mut out = forge_native(&self.body);
        if let Some(raw) = args {
            let words = template::split_args(raw);
            let positional: Vec<&str> = words.iter().map(String::as_str).collect();
            let named: Vec<(String, String)> = self
                .meta
                .arg_names
                .iter()
                .enumerate()
                .map(|(i, n)| (n.clone(), words.get(i).cloned().unwrap_or_default()))
                .collect();
            out = template::expand(&out, &positional, &named, raw);
        }
        out = vars.apply(&out);
        for (name, contents) in &self.resources {
            out.push_str(&forge_native(&format!(
                "\n\n--- resource: {name} ---\n{contents}"
            )));
        }
        out
    }
}

fn safe_resource_path(dir: &Path, res: &str) -> Option<PathBuf> {
    let rel = Path::new(res);
    if rel.is_absolute() {
        return None;
    }
    rel.components()
        .all(|c| {
            matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
        .then(|| dir.join(rel))
}
