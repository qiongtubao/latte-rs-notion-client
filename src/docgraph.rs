//! 知识库笔记树 → doc-graph markdown 工作区物化 + 图谱扩展搜索。
//!
//! Latte 笔记存在 SQLite/Notion；本模块把整棵笔记树导出为
//! `~/.config/latte/docgraph/docs/` 下的 markdown 目录（doc-graph 的工作区格式），
//! 用 latte-doc-review-graph 的图谱（TF-IDF 召回 + wikilink 图扩展）做「相关文档」搜索。
//! 导出文件名即笔记标题（wikilink 按文件名解析），目录的 _index.md 列出子条目链接，
//! 让目录树本身成为图边。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use latte_doc_review_graph::context::{self, ContextConfig};
use latte_doc_review_graph::graph::builder::scan_docs;
use latte_doc_review_graph::util::config::Config;

use crate::config;
use crate::models::{Note, NoteKind};

/// doc-graph 工作区根目录
pub fn docs_root() -> PathBuf {
    config::config_dir().join("docgraph").join("docs")
}

fn mapping_path() -> PathBuf {
    config::config_dir().join("docgraph").join("mapping.json")
}

/// 文件名清洗：去掉路径非法字符，截断，兜底 untitled
fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    let truncated: String = cleaned.chars().take(60).collect();
    if truncated.is_empty() {
        "untitled".to_string()
    } else {
        truncated
    }
}

/// 图谱「相关文档」命中
#[derive(Clone, Debug, PartialEq)]
pub struct RelatedDoc {
    /// Latte 笔记 id（无法映射回笔记时为 None，如非 latte 导出文件）
    pub note_id: Option<String>,
    pub title: String,
    pub score: f32,
}

/// 整树导出（知识库量级小，全量重写最简单可靠）。
/// 目录 → 子目录 + `_index.md`（正文列子条目 wikilink，构成图边）；
/// 文档 → `<标题>.md`（frontmatter 带 title；同级重名加 `--2` 等后缀）。
/// 同时写 mapping.json：doc-graph 节点 id（相对路径）→ latte 笔记 id。
pub fn export_notes(notes: &[Note]) -> Result<()> {
    export_notes_to(&docs_root(), &mapping_path(), notes)
}

/// 同 [`export_notes`]，路径可注入（测试用）
pub fn export_notes_to(root: &Path, mapping_file: &Path, notes: &[Note]) -> Result<()> {
    if root.exists() {
        std::fs::remove_dir_all(root).context("清空 docgraph 工作区失败")?;
    }
    std::fs::create_dir_all(root).context("创建 docgraph 工作区失败")?;

    let mut children: HashMap<Option<String>, Vec<&Note>> = HashMap::new();
    for n in notes.iter().filter(|n| !n.deleted) {
        children.entry(n.parent_id.clone()).or_default().push(n);
    }

    let mut mapping: HashMap<String, String> = HashMap::new();
    export_level(root, "", None, &children, &mut mapping)?;

    let text = serde_json::to_string(&mapping)?;
    if let Some(dir) = mapping_file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(mapping_file, text).context("写 docgraph mapping 失败")?;
    Ok(())
}

/// 导出一层：在 dir_path（相对 root 为 rel）下写 parent 的全部子节点
fn export_level(
    dir_path: &Path,
    rel: &str,
    parent: Option<&str>,
    children: &HashMap<Option<String>, Vec<&Note>>,
    mapping: &mut HashMap<String, String>,
) -> Result<()> {
    let kids = children.get(&parent.map(str::to_string));
    let Some(kids) = kids else { return Ok(()) };

    // 同级文件名去重
    let mut used: HashMap<String, usize> = HashMap::new();
    let mut links: Vec<(String, String)> = Vec::new(); // (显示标题, 文件名 stem)
    for n in kids {
        let base = sanitize(&n.title);
        let count = used.entry(base.clone()).or_insert(0);
        *count += 1;
        let stem = if *count > 1 {
            format!("{base}--{count}")
        } else {
            base
        };
        match n.kind {
            NoteKind::Dir => {
                let sub = dir_path.join(&stem);
                std::fs::create_dir_all(&sub)?;
                let sub_rel = if rel.is_empty() {
                    stem.clone()
                } else {
                    format!("{rel}/{stem}")
                };
                export_level(&sub, &sub_rel, Some(&n.id), children, mapping)?;
                links.push((n.title.clone(), format!("{stem}/_index")));
            }
            NoteKind::Doc => {
                let file = dir_path.join(format!("{stem}.md"));
                let text = format!("---\ntitle: {}\ntype: note\n---\n{}", yaml_escape(&n.title), n.content_md);
                std::fs::write(&file, text)?;
                let path_id = if rel.is_empty() {
                    stem.clone()
                } else {
                    format!("{rel}/{stem}")
                };
                mapping.insert(path_id, n.id.clone());
                links.push((n.title.clone(), stem));
            }
        }
    }

    // 本层 _index.md：列出子条目链接（根层也写，作为图入口）
    if !links.is_empty() {
        let mut body = String::new();
        for (title, stem) in &links {
            body.push_str(&format!("- [[{stem}|{title}]]\n"));
        }
        let index_title = match parent {
            Some(_) => dir_path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "index".into()),
            None => "知识库".to_string(),
        };
        let text = format!("---\ntitle: {}\ntype: note\n---\n{}", yaml_escape(&index_title), body);
        std::fs::write(dir_path.join("_index.md"), text)?;
    }
    Ok(())
}

/// YAML 双引号字符串转义
fn yaml_escape(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// 图谱扩展搜索：对 query 做 TF-IDF 召回 + 图扩展，返回相关文档（含 score 排序）。
/// 每次都现扫工作区（知识库量级小，省掉索引生命周期管理；导出总是最新的）。
pub fn related_docs(query: &str, limit: usize) -> Result<Vec<RelatedDoc>> {
    related_docs_in(&docs_root(), &mapping_path(), query, limit)
}

/// 导出当前全部笔记到 docgraph 工作区；失败仅打日志，不影响主流程
pub fn export_notes_logged(notes: &[Note]) {
    if let Err(e) = export_notes(notes) {
        eprintln!("docgraph 导出失败：{e:#}");
    }
}

/// 同 [`related_docs`]，路径可注入（测试用）
pub fn related_docs_in(
    root: &Path,
    mapping_file: &Path,
    query: &str,
    limit: usize,
) -> Result<Vec<RelatedDoc>> {
    if !root.exists() {
        return Ok(vec![]);
    }
    let graph = scan_docs(root, &Config::default()).context("doc-graph 扫描失败")?;
    if graph.nodes.is_empty() {
        return Ok(vec![]);
    }
    let bundle = context::build(&graph, root, query, 2000, &ContextConfig::default())
        .context("doc-graph context 构建失败")?;

    let mapping: HashMap<String, String> = std::fs::read_to_string(mapping_file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();

    let mut out: Vec<RelatedDoc> = bundle
        .pages
        .iter()
        .map(|p| RelatedDoc {
            note_id: mapping.get(&p.id).cloned(),
            title: if p.title.is_empty() { p.id.clone() } else { p.title.clone() },
            score: p.score,
        })
        .collect();
    // _index 目录页噪音大，排在文档后面
    out.sort_by_key(|p| (p.note_id.is_none(), -(p.score * 1000.0) as i64));
    out.truncate(limit);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str, parent: Option<&str>, kind: NoteKind, title: &str, content: &str) -> Note {
        Note {
            id: id.into(),
            parent_id: parent.map(str::to_string),
            kind,
            title: title.into(),
            content_md: content.into(),
            created_ts: 0,
            updated_ts: 0,
            notion_page_id: None,
            dirty: false,
            deleted: false,
        }
    }

    #[test]
    fn export_tree_and_related() {
        let tmp = std::env::temp_dir().join(format!("latte-docgraph-test-{}", std::process::id()));
        let root = tmp.join("docs");
        let mapping = tmp.join("mapping.json");
        let notes = vec![
            note("b1", None, NoteKind::Dir, "linux", ""),
            note("d1", Some("b1"), NoteKind::Doc, "端口查看应用", "# 端口\nss -tlnp 查看监听端口"),
            note("d2", Some("b1"), NoteKind::Doc, "内存碎片", "用 active 而非 resident 计算碎片率"),
            note("d3", None, NoteKind::Doc, "菜谱", "番茄炒蛋"),
        ];
        export_notes_to(&root, &mapping, &notes).unwrap();

        // 文件结构：书名目录 + _index + 文档 md
        assert!(root.join("linux/_index.md").exists());
        assert!(root.join("linux/端口查看应用.md").exists());
        assert!(root.join("菜谱.md").exists());
        // _index 里是指向文件名 stem 的 wikilink
        let idx = std::fs::read_to_string(root.join("linux/_index.md")).unwrap();
        assert!(idx.contains("[[端口查看应用|端口查看应用]]"));

        // 图谱扩展：能召回相关文档并映射回 latte id
        let hits = related_docs_in(&root, &mapping, "端口", 10).unwrap();
        let d1 = hits.iter().find(|h| h.title == "端口查看应用");
        assert!(d1.is_some());
        assert_eq!(d1.unwrap().note_id.as_deref(), Some("d1"));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
