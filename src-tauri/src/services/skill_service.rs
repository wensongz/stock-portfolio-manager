//! Skill service: loads, parses, and persists AI assistant skills.
//!
//! A skill is a Markdown file with a small YAML-like frontmatter block. The
//! body becomes extra instructions appended to the LLM's system prompt when
//! the skill activates. Files live in `{app_data_dir}/skills/*.md`.
//!
//! Built-in skills are embedded into the binary via `include_str!` and
//! materialised into the user skills directory on first launch (or on
//! `reset_skills`). We record which files are factory-shipped using a hidden
//! `.builtin` marker directory so a user edit (which flips `source` to
//! `User`) can be reverted by `reset_skills`.

use crate::models::skill::{Skill, SkillSource};
use chrono::Utc;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};
use tracing::warn;

/// Embedded built-in skills. Each entry is `(file_stem, raw_markdown)`.
///
/// `include_str!` paths are relative to this source file; the `.md` bodies
/// live next to the crate root under `src/skills/`.
static BUILTIN_SKILLS: &[(&str, &str)] = &[
    (
        "portfolio-risk-analysis",
        include_str!("../skills/portfolio-risk-analysis.md"),
    ),
    ("trade-review", include_str!("../skills/trade-review.md")),
    (
        "quarterly-report",
        include_str!("../skills/quarterly-report.md"),
    ),
    (
        "performance-diagnosis",
        include_str!("../skills/performance-diagnosis.md"),
    ),
    ("market-pulse", include_str!("../skills/market-pulse.md")),
    (
        "stock-deep-dive",
        include_str!("../skills/stock-deep-dive.md"),
    ),
    (
        "return-attribution",
        include_str!("../skills/return-attribution.md"),
    ),
    (
        "dividend-income",
        include_str!("../skills/dividend-income.md"),
    ),
    (
        "allocation-checkup",
        include_str!("../skills/allocation-checkup.md"),
    ),
    (
        "options-review",
        include_str!("../skills/options-review.md"),
    ),
    ("stock-review", include_str!("../skills/stock-review.md")),
    (
        "munger-perspective",
        include_str!("../skills/munger-perspective.md"),
    ),
];

/// Name of the hidden marker directory that tracks which skill files were
/// written by the app (vs. created/edited by the user). Stored as a sibling
/// of the `.md` files so a delete of the `.md` is independent.
const BUILTIN_MARKER_DIR: &str = ".builtin";

/// Resolve the user skills directory, creating it (idempotently) if missing.
pub fn skills_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("无法定位应用数据目录：{e}"))?;
    let dir = base.join("skills");
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建 skills 目录失败：{e}"))?;
    }
    Ok(dir)
}

/// Bump this when built-in skill content changes (trigger words, instructions,
/// etc.). On startup, `export_builtin_skills` overwrites any built-in skill
/// whose on-disk version is older than this — but ONLY if the user hasn't
/// edited it (the `.builtin/{stem}` marker still exists). User-edited skills
/// (marker removed by `save_skill`) are never auto-overwritten.
const BUILTIN_SKILLS_VERSION: u32 = 9;

/// Materialise built-in skills into the user directory on first launch, and
/// auto-update them when [`BUILTIN_SKILLS_VERSION`] bumps.
///
/// For each built-in skill:
/// - If no `.md` exists yet → write the embedded copy (first launch).
/// - If a `.md` exists AND the `.builtin/{stem}` marker exists (user hasn't
///   edited it) AND the recorded version is outdated → overwrite with the new
///   embedded content. This is how trigger-word fixes reach existing users
///   without requiring a manual "恢复内置".
/// - If a `.md` exists but the marker is gone (user edited/deleted-then-remade
///   it) → never overwrite; the user's version wins.
pub fn export_builtin_skills(app: &AppHandle) -> Result<(), String> {
    let dir = skills_dir(app)?;
    export_builtin_skills_to_dir(&dir)
}

fn export_builtin_skills_to_dir(dir: &Path) -> Result<(), String> {
    let marker_dir = dir.join(BUILTIN_MARKER_DIR);
    if !marker_dir.exists() {
        fs::create_dir_all(&marker_dir).map_err(|e| format!("创建内置标记目录失败：{e}"))?;
    }
    for (stem, body) in BUILTIN_SKILLS {
        let path = dir.join(format!("{stem}.md"));
        let marker = marker_dir.join(stem);

        if !path.exists() {
            // First launch (or user deleted it): write the factory version.
            fs::write(&path, body).map_err(|e| format!("写入内置技能 {stem} 失败：{e}"))?;
            let _ = fs::write(&marker, BUILTIN_SKILLS_VERSION.to_string());
            continue;
        }

        // File exists. If the user hasn't edited it (marker present) and the
        // recorded version is outdated, update it to the latest factory content.
        if marker.exists() {
            let current_version = fs::read_to_string(&marker)
                .ok()
                .and_then(|s| s.trim().parse::<u32>().ok())
                .unwrap_or(0);
            if current_version < BUILTIN_SKILLS_VERSION {
                fs::write(&path, body).map_err(|e| format!("更新内置技能 {stem} 失败：{e}"))?;
                let _ = fs::write(&marker, BUILTIN_SKILLS_VERSION.to_string());
            }
        }
        // else: user edited this skill (marker removed by save_skill) → skip.
    }
    Ok(())
}

/// Wipe every user-authored skill and restore the built-in set to its
/// factory state. Used by `reset_skills` and `factory_reset`.
pub fn reset_all_skills(app: &AppHandle) -> Result<(), String> {
    let dir = skills_dir(app)?;

    // Delete every `.md` then re-export the embedded defaults. Removing all
    // files (rather than only user-authored ones) keeps the semantics simple
    // and matches the "restore factory state" contract.
    if dir.exists() {
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                let _ = fs::remove_file(&path);
            }
        }
    }
    // Clear stale builtin markers so they get re-recorded cleanly.
    let marker_dir = dir.join(BUILTIN_MARKER_DIR);
    if marker_dir.exists() {
        let _ = fs::remove_dir_all(&marker_dir);
    }

    export_builtin_skills(app)
}

/// List all skills: every `.md` in the skills directory. `source` is `Builtin`
/// when a matching marker file exists under `.builtin/`, else `User` (covers
/// both brand-new user files and user edits of originally-builtin files).
pub fn list_skills(app: &AppHandle) -> Result<Vec<Skill>, String> {
    let dir = skills_dir(app)?;
    let marker_dir = dir.join(BUILTIN_MARKER_DIR);

    let mut skills: Vec<Skill> = Vec::new();
    if dir.exists() {
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            match parse_skill_file(&path) {
                Ok(mut skill) => {
                    let stem = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string();
                    let is_builtin = marker_dir.join(&stem).exists();
                    skill.source = if is_builtin {
                        SkillSource::Builtin
                    } else {
                        SkillSource::User
                    };
                    skills.push(skill);
                }
                Err(e) => {
                    // One bad file shouldn't break listing the rest.
                    warn!(target: "skills", "failed to parse {}: {e}", path.display());
                }
            }
        }
    }
    // Stable, readable order: enabled first, then alphabetical by name.
    skills.sort_by(|a, b| match (a.enabled, b.enabled) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });
    Ok(skills)
}

/// Load a single skill by id (file stem).
pub fn get_skill(app: &AppHandle, id: &str) -> Result<Skill, String> {
    let dir = skills_dir(app)?;
    let path = dir.join(format!("{id}.md"));
    if !path.exists() {
        return Err(format!("找不到技能：{id}"));
    }
    let mut skill = parse_skill_file(&path)?;
    let is_builtin = dir.join(BUILTIN_MARKER_DIR).join(id).exists();
    skill.source = if is_builtin {
        SkillSource::Builtin
    } else {
        SkillSource::User
    };
    Ok(skill)
}

/// Create or update a skill file. Re-serialises frontmatter from the model
/// and always writes to `{id}.md`, so renaming is not supported via this
/// API (the UI creates a new file + deletes the old one instead).
pub fn save_skill(app: &AppHandle, skill: &Skill) -> Result<(), String> {
    validate_id(&skill.id)?;
    let dir = skills_dir(app)?;
    let path = dir.join(format!("{}.md", skill.id));
    let body = serialize_skill(skill);
    fs::write(&path, body).map_err(|e| format!("保存技能失败：{e}"))?;

    // A user save always graduates the file out of the builtin set: drop the
    // marker so list_skills reports it as User from now on. This lets the UI
    // show "edited" state and lets delete_skill actually remove it.
    let marker = dir.join(BUILTIN_MARKER_DIR).join(&skill.id);
    if marker.exists() {
        let _ = fs::remove_file(&marker);
    }
    Ok(())
}

/// Delete a skill file. Always allowed for user-authored skills; for
/// built-in skills it removes the file but leaves the marker behind (so the
/// skill stays deleted until the next `reset_skills`), matching how users
/// expect "delete" to work on a factory item.
pub fn delete_skill(app: &AppHandle, id: &str) -> Result<(), String> {
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err("技能 id 含有非法字符".to_string());
    }
    let dir = skills_dir(app)?;
    let path = dir.join(format!("{id}.md"));
    if !path.exists() {
        // Nothing to do — treat as success so the UI is idempotent.
        return Ok(());
    }
    fs::remove_file(&path).map_err(|e| format!("删除技能失败：{e}"))?;
    // Drop the marker too: a deleted builtin shouldn't reappear as
    // "builtin" on next list (file is gone anyway).
    let marker = dir.join(BUILTIN_MARKER_DIR).join(id);
    if marker.exists() {
        let _ = fs::remove_file(&marker);
    }
    Ok(())
}

/// Clone an existing skill to a new id. The copy is always user-owned
/// (no builtin marker), so cloning a built-in produces an editable custom
/// skill without touching the original. Returns the newly-created skill.
///
/// `new_id` must be a fresh, valid id (kebab-case, not already in use);
/// callers surface a friendly error otherwise.
pub fn clone_skill(app: &AppHandle, source_id: &str, new_id: &str) -> Result<Skill, String> {
    validate_id(new_id)?;
    let source = get_skill(app, source_id)?;
    let dir = skills_dir(app)?;
    let target_path = dir.join(format!("{new_id}.md"));
    if target_path.exists() {
        return Err(format!("技能 id 已存在：{new_id}"));
    }
    let clone = Skill {
        id: new_id.to_string(),
        name: format!("{}（副本）", source.name),
        description: source.description.clone(),
        trigger: source.trigger.clone(),
        enabled: source.enabled,
        content: source.content.clone(),
        source: SkillSource::User,
        updated_at: Utc::now().to_rfc3339(),
    };
    let body = serialize_skill(&clone);
    fs::write(&target_path, body).map_err(|e| format!("克隆技能失败：{e}"))?;
    // No builtin marker — clones are always user-owned.
    Ok(clone)
}

/// Write a skill's full Markdown representation to an arbitrary path on disk.
/// Used by the "导出" button (the frontend picks the path via the Tauri save
/// dialog). Returns the path written.
pub fn export_skill_to_path(skill: &Skill, path: &Path) -> Result<String, String> {
    let body = serialize_skill(skill);
    fs::write(path, body).map_err(|e| format!("导出技能失败：{e}"))?;
    path.to_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "导出路径包含非法字符".to_string())
}

/// Read a Markdown skill file from an arbitrary path, parse it, and save it
/// into the user skills directory under a (possibly suffixed) id. Used by the
/// "导入" button. If a skill with the parsed id already exists, we append a
/// numeric suffix (`-2`, `-3`, …) until a free id is found.
///
/// The imported skill is always user-owned. Returns the saved skill.
pub fn import_skill_from_path(app: &AppHandle, path: &Path) -> Result<Skill, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("读取技能文件失败：{e}"))?;
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("imported")
        .to_string();
    let parsed = parse_skill_from_str(&stem, &raw, path)?;
    let dir = skills_dir(app)?;

    // Resolve a free id: keep the parsed id if available, else suffix -2/-3/…
    let base_id = parsed.id.clone();
    let mut final_id = base_id.clone();
    let mut suffix = 2;
    while dir.join(format!("{final_id}.md")).exists() {
        final_id = format!("{base_id}-{suffix}");
        suffix += 1;
        if suffix > 99 {
            return Err(format!(
                "无法为导入的技能找到可用 id（已尝试到 {final_id}）"
            ));
        }
    }
    validate_id(&final_id)?;
    let imported = Skill {
        id: final_id.clone(),
        name: parsed.name,
        description: parsed.description,
        trigger: parsed.trigger,
        enabled: parsed.enabled,
        content: parsed.content,
        source: SkillSource::User,
        updated_at: Utc::now().to_rfc3339(),
    };
    let body = serialize_skill(&imported);
    let target_path = dir.join(format!("{final_id}.md"));
    fs::write(&target_path, body).map_err(|e| format!("保存导入的技能失败：{e}"))?;
    Ok(imported)
}

/// Reject ids that could escape the skills directory, are empty, or don't
/// follow strict kebab-case (lowercase letter first, then lowercase letters /
/// digits / hyphen-separated segments — no leading digit, no consecutive or
/// trailing hyphens). Mirrors the frontend `SKILL_ID_PATTERN`.
fn validate_id(id: &str) -> Result<(), String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err("技能 id 不能为空".to_string());
    }
    // Defence in depth: never allow path traversal even though the regex
    // below would already reject these.
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err("技能 id 含有非法字符".to_string());
    }
    let re = regex::Regex::new(r"^[a-z]([a-z0-9]*)(-[a-z0-9]+)*$").unwrap();
    if !re.is_match(trimmed) {
        return Err(
            "技能 id 需为小写字母开头的 kebab-case（仅小写字母、数字、连字符）".to_string(),
        );
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Frontmatter parsing & serialisation
// ─────────────────────────────────────────────────────────────────────────────

/// Parsed frontmatter fields. Everything is optional in the file; defaults
/// are applied when building the `Skill`.
#[derive(Debug, Default, PartialEq)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
    trigger: Vec<String>,
    enabled: Option<bool>,
}

/// Parse a skill file from disk into a `Skill`.
pub fn parse_skill_file(path: &Path) -> Result<Skill, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("读取文件失败：{e}"))?;
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    parse_skill_from_str(&stem, &raw, path)
}

/// Pure parser used by both `parse_skill_file` and unit tests. `path` is only
/// used to surface nicer error context.
fn parse_skill_from_str(stem: &str, raw: &str, path: &Path) -> Result<Skill, String> {
    let (fm, body) = split_frontmatter(raw);
    let fm = parse_frontmatter(fm).map_err(|e| format!("{}：{e}", path.display()))?;

    let name = fm
        .name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| stem.to_string());
    let description = fm.description.unwrap_or_default();
    let updated_at = fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| {
            chrono::DateTime::<Utc>::from_timestamp(d.as_secs() as i64, 0)
                .map(|t| t.to_rfc3339())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    Ok(Skill {
        id: stem.to_string(),
        name,
        description,
        trigger: fm.trigger,
        enabled: fm.enabled.unwrap_or(true),
        content: body.trim_start_matches('\n').to_string(),
        source: SkillSource::User, // finalised by callers based on markers
        updated_at,
    })
}

/// Split off a leading `---\n...\n---` block. Returns `(frontmatter_block,
/// body)`. If there is no valid opening fence the whole input is treated as
/// body with empty frontmatter.
fn split_frontmatter(raw: &str) -> (&str, &str) {
    let raw = raw.strip_prefix('\u{FEFF}').unwrap_or(raw);
    // Require the very first line to be exactly `---`.
    let after_open = match raw
        .strip_prefix("---\n")
        .or_else(|| raw.strip_prefix("---\r\n"))
    {
        Some(rest) => rest,
        None => return ("", raw),
    };
    // Find the first line that is exactly `---`; that closes the block.
    let mut consumed = 0usize;
    let mut close_start = None;
    for line in after_open.split_inclusive('\n') {
        if line.trim_end_matches(['\n', '\r']) == "---" {
            close_start = Some(consumed);
            break;
        }
        consumed += line.len();
    }
    match close_start {
        Some(idx) => {
            let fm = &after_open[..idx];
            // Skip past the closing fence line (advance to the next `\n`).
            let mut body_start = idx;
            while body_start < after_open.len() && after_open.as_bytes()[body_start] != b'\n' {
                body_start += 1;
            }
            if body_start < after_open.len() {
                body_start += 1; // consume the `\n`
            }
            (fm, &after_open[body_start..])
        }
        None => ("", raw),
    }
}

/// Parse the frontmatter block into structured fields. Hand-written to avoid
/// pulling in a YAML dependency — we only support the flat `key: value` form.
fn parse_frontmatter(fm: &str) -> Result<Frontmatter, String> {
    let mut out = Frontmatter::default();
    if fm.is_empty() {
        return Ok(out);
    }
    for line in fm.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = match line.split_once(':') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };
        match key {
            "name" => out.name = Some(strip_quotes(value).to_string()),
            "description" => out.description = Some(strip_quotes(value).to_string()),
            "trigger" => {
                // The serialiser quotes the whole list when it contains a
                // comma, so strip a single layer of surrounding quotes before
                // splitting on commas.
                let value = strip_quotes(value);
                out.trigger = value
                    .split([',', '，'])
                    .map(|s| strip_quotes(s.trim()).to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "enabled" => {
                out.enabled = match value.to_ascii_lowercase().as_str() {
                    "true" | "yes" | "on" | "1" => Some(true),
                    "false" | "no" | "off" | "0" => Some(false),
                    _ => None,
                };
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Strip a single layer of matching surrounding quotes (" or ').
fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.len() >= 2
        && bytes[0] == bytes[bytes.len() - 1]
        && (bytes[0] == b'"' || bytes[0] == b'\'')
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Render a `Skill` back to its on-disk Markdown representation.
pub fn serialize_skill(skill: &Skill) -> String {
    let trigger = skill.trigger.join(", ");
    let enabled = if skill.enabled { "true" } else { "false" };
    format!(
        "---\nname: {name}\ndescription: {desc}\ntrigger: {trigger}\nenabled: {enabled}\n---\n\n{body}\n",
        name = escape_scalar(&skill.name),
        desc = escape_scalar(&skill.description),
        trigger = escape_scalar(&trigger),
        body = skill.content.trim_end(),
    )
}

/// Quote a scalar if it contains characters that would confuse our simple
/// parser (colon, comma, leading/trailing whitespace, surrounding quotes).
fn escape_scalar(s: &str) -> String {
    let needs_quotes = s.is_empty()
        || s != s.trim()
        || s.contains(':')
        || s.contains([',', '"', '\''])
        || s.contains('\n');
    if needs_quotes {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Matching helpers (also used by ai_chat_service for auto-activation)
// ─────────────────────────────────────────────────────────────────────────────

/// Return the subset of `skills` whose `trigger` keywords appear in `text`.
/// Case-insensitive; an empty trigger list never matches. Order follows the
/// input iterator.
///
/// **Matching rule** (avoids false positives like `risk` matching `riskier`):
/// - If the keyword contains any non-ASCII (CJK etc.) character → substring
///   match. CJK has no word boundaries, so containment is the right model.
/// - Otherwise (pure ASCII/Latin) → require the keyword to appear as a whole
///   word, where a "word boundary" is the edge between an alphanumeric and a
///   non-alphanumeric character (or the start/end of text). This prevents
///   `夏普`-style short Latin triggers like `roi` from matching `reality`.
pub fn match_triggers<'a, I>(skills: I, text: &str) -> Vec<Skill>
where
    I: IntoIterator<Item = &'a Skill>,
{
    let haystack = text.to_lowercase();
    let skills: Vec<&Skill> = skills.into_iter().filter(|skill| skill.enabled).collect();
    let mut out = Vec::new();
    for s in &skills {
        let hit = s.trigger.iter().any(|t| {
            let t = t.trim().to_lowercase();
            if t.is_empty() {
                return false;
            }
            let is_cjk = !t.is_ascii();
            if is_cjk {
                haystack.contains(&t)
            } else {
                contains_whole_word(&haystack, &t)
            }
        });
        if hit {
            out.push((*s).clone());
        }
    }

    // Built-in review skills need an explicit review intent and instrument
    // semantics; option-only historical aliases are the narrow exception.
    // Frontmatter keywords remain useful for discovery, but nouns such as
    // "期权交易" and "Campaign" must not activate an unrelated review.
    let explicit_review_intent =
        haystack.contains("复盘") || contains_whole_word(&haystack, "review");
    let has_option_domain = haystack.contains("期权")
        || contains_whole_word(&haystack, "csp")
        || haystack.contains("covered call")
        || haystack.contains("权利金");
    let option_historical_alias = has_option_domain
        && (haystack.contains("历史表现")
            || haystack.contains("历史campaign")
            || haystack.contains("权利金留存率"));
    let has_option_review =
        has_option_domain && (explicit_review_intent || option_historical_alias);
    let has_stock_review = explicit_review_intent
        && (haystack.contains("调仓")
            || (haystack.contains("股票")
                && (haystack.contains("操作")
                    || haystack.contains("campaign")
                    || haystack.contains("复盘报告"))));

    out.retain(|skill| {
        !matches!(skill.id.as_str(), "stock-review" | "options-review")
            || (skill.id == "stock-review" && has_stock_review)
            || (skill.id == "options-review" && has_option_review)
    });
    for (id, active) in [
        ("options-review", has_option_review),
        ("stock-review", has_stock_review),
    ] {
        if active && !out.iter().any(|skill| skill.id == id) {
            if let Some(skill) = skills.iter().find(|skill| skill.id == id) {
                out.push((**skill).clone());
            }
        }
    }
    resolve_builtin_review_conflicts(out)
}

fn resolve_builtin_review_conflicts(mut skills: Vec<Skill>) -> Vec<Skill> {
    let has_stock_review = skills.iter().any(|skill| skill.id == "stock-review");
    let has_options_review = skills.iter().any(|skill| skill.id == "options-review");
    if has_stock_review {
        skills.retain(|skill| {
            !matches!(
                skill.id.as_str(),
                "trade-review" | "quarterly-report" | "return-attribution"
            )
        });
    }
    if has_options_review {
        skills.retain(|skill| skill.id != "trade-review");
    }
    skills
}

/// Whole-word ASCII containment: matches `needle` in `haystack` only when
/// both sides are bounded by non-alphanumeric characters (or the string
/// edge). Lowercase both sides before calling.
fn contains_whole_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut start = 0usize;
    let h_bytes = haystack.as_bytes();
    let n_bytes = needle.as_bytes();
    while start + n_bytes.len() <= h_bytes.len() {
        if let Some(idx) = haystack[start..].find(needle) {
            let match_start = start + idx;
            let match_end = match_start + n_bytes.len();
            let left_ok = match_start == 0 || !is_word_byte(h_bytes[match_start - 1]);
            let right_ok = match_end == h_bytes.len() || !is_word_byte(h_bytes[match_end]);
            if left_ok && right_ok {
                return true;
            }
            // Advance past this (rejected) match to keep searching. `find`
            // returns byte offsets, so advance by at least 1 to guarantee
            // progress even on overlapping candidates.
            start = match_end.max(match_start + 1);
        } else {
            return false;
        }
    }
    false
}

/// A "word" byte for boundary purposes: ASCII alphanumeric. Everything else
/// (space, punctuation, CJK byte) counts as a boundary.
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

/// Build the system-prompt fragment appended when one or more skills are
/// active. Pure function so it can be unit-tested without touching the
/// filesystem.
pub fn build_skill_system_message(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut buf = String::from("以下技能已激活，请在回答时严格遵循其指令：\n");
    for s in skills {
        buf.push_str(&format!("\n## {}\n{}\n", s.name, s.content.trim()));
    }
    buf
}

// Dedup helper kept simple & explicit (no extra imports needed in callers).
#[allow(dead_code)]
pub fn dedup_ids(skills: Vec<Skill>) -> Vec<Skill> {
    let mut seen = HashSet::new();
    skills
        .into_iter()
        .filter(|s| seen.insert(s.id.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
    }

    // --- built-in skills -----------------------------------------------------

    #[test]
    fn all_builtin_skills_parse_with_name_and_trigger() {
        for (stem, body) in BUILTIN_SKILLS {
            let parsed = parse_skill_from_str(stem, body, Path::new(stem)).expect(stem);
            assert!(!parsed.name.is_empty(), "{stem} missing name");
            assert!(!parsed.description.is_empty(), "{stem} missing description");
            assert!(!parsed.trigger.is_empty(), "{stem} missing trigger");
        }
    }

    #[test]
    fn munger_perspective_builtin() {
        // The 芒格视角 skill: Chinese name, description and CJK trigger words.
        let (_, body) = BUILTIN_SKILLS
            .iter()
            .find(|(stem, _)| *stem == "munger-perspective")
            .expect("munger-perspective must be a builtin skill");
        let parsed = parse_skill_from_str(
            "munger-perspective",
            body,
            Path::new("munger-perspective.md"),
        )
        .unwrap();
        assert_eq!(parsed.name, "芒格视角");
        assert_eq!(parsed.trigger, vec!["芒格视角", "芒格"]);
        assert!(parsed.content.contains("查理·芒格"));
    }

    #[test]
    fn options_review_routes_historical_and_current_risk_tools() {
        let (_, body) = BUILTIN_SKILLS
            .iter()
            .find(|(stem, _)| *stem == "options-review")
            .expect("options-review builtin");
        let parsed =
            parse_skill_from_str("options-review", body, Path::new("options-review.md")).unwrap();

        assert!(parsed.content.contains("get_option_review"));
        assert!(parsed.content.contains("get_option_positions"));
        assert!(parsed.content.contains("历史执行复盘"));
        assert!(parsed
            .content
            .contains("当前持仓、到期风险、ITM/OTM、被行权准备"));
        assert!(parsed.content.contains("未平仓"));
        assert!(parsed.content.contains("accountId"));
        assert!(parsed.content.contains("periodDays"));
        assert!(parsed.content.contains("allHistory"));
        assert!(parsed.content.contains("做得好的"));
        assert!(parsed.content.contains("值得改进"));

        for broad_trigger in ["到期", "put", "call", "被行权"] {
            assert!(
                !parsed
                    .trigger
                    .iter()
                    .any(|trigger| trigger == broad_trigger),
                "broad current-risk trigger must be absent: {broad_trigger}"
            );
        }
    }

    fn parsed_builtins() -> Vec<Skill> {
        BUILTIN_SKILLS
            .iter()
            .map(|(stem, body)| {
                parse_skill_from_str(stem, body, Path::new(stem)).expect("builtin parses")
            })
            .collect()
    }

    fn activated_ids(text: &str) -> Vec<String> {
        let skills = parsed_builtins();
        match_triggers(&skills, text)
            .into_iter()
            .map(|skill| skill.id)
            .collect()
    }

    #[test]
    fn stock_review_builtin_parses_registers_and_routes_without_generic_review_conflicts() {
        let skills = parsed_builtins();
        let stock_review = skills
            .iter()
            .find(|skill| skill.id == "stock-review")
            .expect("stock-review builtin must be registered");
        assert_eq!(
            stock_review.trigger,
            vec!["股票操作复盘", "调仓复盘", "股票复盘报告"]
        );
        assert!(stock_review.content.contains("股票操作复盘是只读分析"));
        assert!(!stock_review
            .content
            .contains("save_stock_review_annotation"));

        assert_eq!(activated_ids("股票操作复盘"), vec!["stock-review"]);
        assert_eq!(activated_ids("请做调仓复盘"), vec!["stock-review"]);
        assert_eq!(activated_ids("生成股票复盘报告"), vec!["stock-review"]);
        assert_eq!(activated_ids("复盘"), vec!["trade-review"]);
        assert_eq!(
            activated_ids("复盘一下我的期权交易"),
            vec!["options-review"]
        );
        assert_eq!(activated_ids("请做期权复盘"), vec!["options-review"]);
    }

    #[test]
    fn review_routing_requires_intent_and_keeps_stock_and_option_domains_disjoint() {
        let cases = [
            ("股票 Campaign 复盘", vec!["stock-review"]),
            ("复盘一下我的期权交易", vec!["options-review"]),
            ("请做期权历史表现复盘", vec!["options-review"]),
            ("导入一笔期权交易", vec!["trade-review"]),
            ("Campaign复盘", vec!["trade-review"]),
            ("复盘", vec!["trade-review"]),
            (
                "一起复盘股票调仓和期权交易",
                vec!["options-review", "stock-review"],
            ),
        ];

        for (text, expected) in cases {
            assert_eq!(
                activated_ids(text),
                expected,
                "activation mismatch for {text}"
            );
        }
    }

    #[test]
    fn stock_review_requires_operation_rebalance_or_campaign_review_semantics() {
        for text in [
            "这只股票历史表现怎么样",
            "查询这只股票的风险",
            "导入股票交易记录",
            "查询股票行情",
        ] {
            assert!(
                !activated_ids(text).iter().any(|id| id == "stock-review"),
                "stock-review must stay inactive for {text}"
            );
        }

        for text in ["股票操作复盘", "复盘股票调仓", "股票 Campaign 复盘"] {
            assert_eq!(activated_ids(text), vec!["stock-review"], "{text}");
        }
        assert_eq!(
            activated_ids("一起复盘股票调仓和期权交易"),
            vec!["options-review", "stock-review"]
        );
    }

    #[test]
    fn version_seven_upgrade_installs_stock_review_without_deleting_user_skills() {
        let dir = tempdir().unwrap();
        let skills_dir = dir.path();
        let marker_dir = skills_dir.join(BUILTIN_MARKER_DIR);
        fs::create_dir_all(&marker_dir).unwrap();
        write(&skills_dir.join("trade-review.md"), "old builtin");
        write(&marker_dir.join("trade-review"), "7");
        write(
            &skills_dir.join("my-private-process.md"),
            "---\nname: private\ndescription: user owned\ntrigger: private\n---\nbody\n",
        );

        export_builtin_skills_to_dir(skills_dir).unwrap();

        assert!(skills_dir.join("stock-review.md").exists());
        assert_eq!(
            fs::read_to_string(marker_dir.join("stock-review")).unwrap(),
            "9"
        );
        assert!(skills_dir.join("my-private-process.md").exists());
        assert!(!marker_dir.join("my-private-process").exists());
    }

    #[test]
    fn version_eight_upgrade_replaces_review_builtins_and_routes_from_migrated_disk_content() {
        let dir = tempdir().unwrap();
        let skills_dir = dir.path();
        let marker_dir = skills_dir.join(BUILTIN_MARKER_DIR);
        fs::create_dir_all(&marker_dir).unwrap();
        write(
            &skills_dir.join("options-review.md"),
            "---\nname: old options\ndescription: old\ntrigger: 期权交易,Campaign复盘\nenabled: true\n---\nold options\n",
        );
        write(
            &skills_dir.join("stock-review.md"),
            "---\nname: old stock\ndescription: old\ntrigger: 股票历史表现\nenabled: true\n---\nold stock\n",
        );
        write(&marker_dir.join("options-review"), "8");
        write(&marker_dir.join("stock-review"), "8");
        let custom = "---\nname: private\ndescription: user owned\ntrigger: private\nenabled: true\n---\ncustom body\n";
        write(&skills_dir.join("my-private-process.md"), custom);

        export_builtin_skills_to_dir(skills_dir).unwrap();

        for stem in ["options-review", "stock-review"] {
            let embedded = BUILTIN_SKILLS
                .iter()
                .find(|(candidate, _)| *candidate == stem)
                .unwrap()
                .1;
            assert_eq!(
                fs::read_to_string(skills_dir.join(format!("{stem}.md"))).unwrap(),
                embedded
            );
            assert_eq!(fs::read_to_string(marker_dir.join(stem)).unwrap(), "9");
        }
        assert_eq!(
            fs::read_to_string(skills_dir.join("my-private-process.md")).unwrap(),
            custom
        );
        assert!(!marker_dir.join("my-private-process").exists());

        let migrated = ["trade-review", "options-review", "stock-review"]
            .into_iter()
            .map(|stem| parse_skill_file(&skills_dir.join(format!("{stem}.md"))).unwrap())
            .collect::<Vec<_>>();
        let disk_ids = |text: &str| {
            match_triggers(&migrated, text)
                .into_iter()
                .map(|skill| skill.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(disk_ids("股票 Campaign 复盘"), vec!["stock-review"]);
        assert_eq!(disk_ids("复盘期权交易"), vec!["options-review"]);
        assert_eq!(
            disk_ids("一起复盘股票调仓和期权交易"),
            vec!["options-review", "stock-review"]
        );
        assert!(!disk_ids("这只股票历史表现怎么样")
            .iter()
            .any(|id| id == "stock-review"));
    }

    // --- parse_frontmatter ---------------------------------------------------

    #[test]
    fn parses_full_frontmatter() {
        let raw = "---\nname: my-skill\ndescription: hello world\ntrigger: 风险,回撤, 集中度\nenabled: false\n---\n\n# Body\nLine 2\n";
        let (fm, body) = split_frontmatter(raw);
        let fm = parse_frontmatter(fm).unwrap();
        assert_eq!(fm.name.as_deref(), Some("my-skill"));
        assert_eq!(fm.description.as_deref(), Some("hello world"));
        assert_eq!(fm.trigger, vec!["风险", "回撤", "集中度"]);
        assert_eq!(fm.enabled, Some(false));
        assert!(body.trim_start().starts_with("# Body"));
    }

    #[test]
    fn parses_without_frontmatter() {
        let raw = "# Just a title\nNo frontmatter here.\n";
        let (fm, body) = split_frontmatter(raw);
        assert!(fm.is_empty());
        assert_eq!(body, raw);
    }

    #[test]
    fn parses_with_missing_optional_fields() {
        let raw = "---\nname: only-name\n---\nbody\n";
        let (fm, body) = split_frontmatter(raw);
        let fm = parse_frontmatter(fm).unwrap();
        assert_eq!(fm.name.as_deref(), Some("only-name"));
        assert!(fm.description.is_none());
        assert!(fm.trigger.is_empty());
        assert!(fm.enabled.is_none()); // defaults to true downstream
        assert_eq!(body.trim(), "body");
    }

    #[test]
    fn handles_quoted_values_and_chinese_comma() {
        let raw =
            "---\nname: \"quoted name\"\ndescription: 'has：colon'\ntrigger: a，b，c\n---\nx\n";
        let (fm, _) = split_frontmatter(raw);
        let fm = parse_frontmatter(fm).unwrap();
        assert_eq!(fm.name.as_deref(), Some("quoted name"));
        assert_eq!(fm.description.as_deref(), Some("has：colon"));
        assert_eq!(fm.trigger, vec!["a", "b", "c"]);
    }

    #[test]
    fn ignores_unknown_keys_and_comments() {
        let raw = "---\n# a comment\nname: x\nunknown: ignored\n---\nbody\n";
        let (fm, _) = split_frontmatter(raw);
        let fm = parse_frontmatter(fm).unwrap();
        assert_eq!(fm.name.as_deref(), Some("x"));
    }

    // --- match_triggers ------------------------------------------------------

    #[test]
    fn matches_case_insensitive_keywords() {
        let s = Skill {
            id: "x".into(),
            name: "X".into(),
            description: "".into(),
            trigger: vec!["Risk".into(), "Drawdown".into()],
            enabled: true,
            content: "".into(),
            source: SkillSource::User,
            updated_at: "".into(),
        };
        assert_eq!(match_triggers([&s], "what is my risk profile?").len(), 1);
        assert!(match_triggers([&s], "nothing relevant").is_empty());
    }

    #[test]
    fn skips_disabled_skills() {
        let s = Skill {
            id: "x".into(),
            name: "X".into(),
            description: "".into(),
            trigger: vec!["risk".into()],
            enabled: false,
            content: "".into(),
            source: SkillSource::User,
            updated_at: "".into(),
        };
        assert!(match_triggers([&s], "risk!").is_empty());
    }

    #[test]
    fn ascii_trigger_uses_whole_word_matching() {
        let s = Skill {
            id: "x".into(),
            name: "X".into(),
            description: "".into(),
            trigger: vec!["risk".into()],
            enabled: true,
            content: "".into(),
            source: SkillSource::User,
            updated_at: "".into(),
        };
        // Whole-word hits.
        assert_eq!(match_triggers([&s], "what is my risk?").len(), 1);
        assert_eq!(match_triggers([&s], "risk").len(), 1);
        assert_eq!(match_triggers([&s], "my risk profile").len(), 1);
        // Should NOT match substrings inside larger words.
        assert!(match_triggers([&s], "this is riskier than before").is_empty());
        assert!(match_triggers([&s], "assessing").is_empty());
        // Boundary includes punctuation — `risks` still won't match `risk`.
        assert!(match_triggers([&s], "risks are high").is_empty());
    }

    #[test]
    fn cjk_trigger_uses_substring_matching() {
        let s = Skill {
            id: "x".into(),
            name: "X".into(),
            description: "".into(),
            trigger: vec!["夏普".into()],
            enabled: true,
            content: "".into(),
            source: SkillSource::User,
            updated_at: "".into(),
        };
        // CJK has no word boundaries — substring is the right model.
        assert_eq!(match_triggers([&s], "我的夏普比率是多少").len(), 1);
        assert!(match_triggers([&s], "没提到").is_empty());
    }

    #[test]
    fn whole_word_match_is_case_insensitive() {
        let s = Skill {
            id: "x".into(),
            name: "X".into(),
            description: "".into(),
            trigger: vec!["Drawdown".into()],
            enabled: true,
            content: "".into(),
            source: SkillSource::User,
            updated_at: "".into(),
        };
        assert_eq!(match_triggers([&s], "max DRAWDOWN today").len(), 1);
    }

    // --- build_skill_system_message -----------------------------------------

    #[test]
    fn system_message_lists_activated_skills() {
        let s1 = Skill {
            id: "a".into(),
            name: "Alpha".into(),
            description: "".into(),
            trigger: vec![],
            enabled: true,
            content: "Step 1\nStep 2".into(),
            source: SkillSource::User,
            updated_at: "".into(),
        };
        let msg = build_skill_system_message(&[s1]);
        assert!(msg.contains("已激活"));
        assert!(msg.contains("## Alpha"));
        assert!(msg.contains("Step 1"));
    }

    #[test]
    fn system_message_empty_for_no_skills() {
        assert_eq!(build_skill_system_message(&[]), "");
    }

    // --- save / get / list round-trip ---------------------------------------

    #[test]
    fn serialize_then_parse_round_trip() {
        let skill = Skill {
            id: "round-trip".into(),
            name: "Round Trip".into(),
            description: "desc".into(),
            trigger: vec!["a".into(), "b".into()],
            enabled: true,
            content: "# Heading\n\nbody text".into(),
            source: SkillSource::User,
            updated_at: "".into(),
        };
        let body = serialize_skill(&skill);
        // Use a fake path whose metadata lookup will fail; parse_skill_from_str
        // tolerates that and falls back to an empty updated_at.
        let fake_path = Path::new("round-trip.md");
        let parsed = parse_skill_from_str("round-trip", &body, fake_path).unwrap();
        assert_eq!(parsed.id, "round-trip");
        assert_eq!(parsed.name, "Round Trip");
        assert_eq!(parsed.description, "desc");
        assert_eq!(parsed.trigger, vec!["a", "b"]);
        assert!(parsed.enabled);
        assert!(parsed.content.contains("# Heading"));
        assert!(parsed.content.contains("body text"));
    }

    #[test]
    fn parse_real_file_via_filesystem() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("demo.md");
        write(
            &path,
            "---\nname: demo\ndescription: a demo\ntrigger: foo,bar\nenabled: true\n---\n\nBody here\n",
        );
        let parsed = parse_skill_file(&path).unwrap();
        assert_eq!(parsed.id, "demo");
        assert_eq!(parsed.name, "demo");
        assert_eq!(parsed.trigger, vec!["foo", "bar"]);
        assert_eq!(parsed.content, "Body here\n");
    }

    #[test]
    fn parse_rejects_missing_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nope.md");
        assert!(parse_skill_file(&path).is_err());
    }

    #[test]
    fn parse_handles_body_with_horizontal_rules() {
        // A line that is exactly `---` inside the body should not confuse the
        // frontmatter splitter because we only search for the *first* closing
        // fence.
        let raw = "---\nname: t\n---\n\nintro\n\n---\n\nmore\n";
        let (fm, body) = split_frontmatter(raw);
        let fm = parse_frontmatter(fm).unwrap();
        assert_eq!(fm.name.as_deref(), Some("t"));
        assert!(body.contains("intro"));
        assert!(body.contains("more"));
        assert!(body.contains("---"));
    }

    // --- validate_id --------------------------------------------------------

    #[test]
    fn validate_id_accepts_kebab_case() {
        assert!(validate_id("my-skill").is_ok());
        assert!(validate_id("a").is_ok());
        assert!(validate_id("skill-2").is_ok());
        assert!(validate_id("q3-report").is_ok());
        assert!(validate_id("portfolio2").is_ok());
    }

    #[test]
    fn validate_id_rejects_empty_and_traversal() {
        assert!(validate_id("").is_err());
        assert!(validate_id("  ").is_err());
        assert!(validate_id("../escape").is_err());
        assert!(validate_id("a/b").is_err());
        assert!(validate_id("a\\b").is_err());
        assert!(validate_id("..").is_err());
    }

    #[test]
    fn validate_id_rejects_non_kebab_case() {
        // Uppercase, underscores, leading digit, leading/consecutive/trailing
        // hyphens are all rejected under strict kebab-case.
        assert!(validate_id("My-Skill").is_err());
        assert!(validate_id("skill_2").is_err());
        assert!(validate_id("2-fast").is_err());
        assert!(validate_id("-lead").is_err());
        assert!(validate_id("trail-").is_err());
        assert!(validate_id("double--hyphen").is_err());
    }

    // --- export_skill_to_path round-trip -----------------------------------

    #[test]
    fn export_then_import_body_round_trip() {
        let skill = Skill {
            id: "exportable".into(),
            name: "Exportable".into(),
            description: "for export".into(),
            trigger: vec!["x".into(), "y".into()],
            enabled: true,
            content: "# Body\n\nline".into(),
            source: SkillSource::User,
            updated_at: "".into(),
        };
        let dir = tempdir().unwrap();
        let path = dir.path().join("exportable.md");
        let written_path = export_skill_to_path(&skill, &path).unwrap();
        assert!(written_path.ends_with("exportable.md"));

        // The exported file should re-parse to the same shape (id comes from
        // the file stem, which matches).
        let parsed = parse_skill_file(&path).unwrap();
        assert_eq!(parsed.name, "Exportable");
        assert_eq!(parsed.trigger, vec!["x", "y"]);
        assert!(parsed.content.contains("# Body"));
    }
}
