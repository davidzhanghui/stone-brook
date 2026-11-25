//! 侧边栏文档树的纯计算（不碰 UI，方便单独测试）。

use crate::store::DocMeta;
use std::collections::{HashMap, HashSet};

/// 侧边栏拖拽移动时的落点类型。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DropZone {
    /// 移到目标文档之前（与目标同级）
    Before(String),
    /// 移入目标文档内部，成为其子文档（排在末尾）
    Into(String),
    /// 移到目标文档之后（与目标同级）
    After(String),
    /// 移到根目录最前面
    RootStart,
    /// 移到根目录末尾
    RootEnd,
}

/// 一次合法移动的计算结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MovePlan {
    pub new_parent: Option<String>,
    /// 目标父节点下所有未删除子文档移动后的完整顺序（含被移动文档）
    pub sibling_order: Vec<String>,
}

/// 「移动到…」选择器中的可选项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MoveTarget {
    /// `None` 表示根目录（顶层）
    pub parent: Option<String>,
    pub title: String,
    /// 树形缩进层级（根目录为 0）
    pub depth: usize,
    /// 祖先路径，用于搜索时区分同名文档
    pub path: String,
    /// 是否已是当前所在父位置
    pub is_current: bool,
}

/// 子孙文档统计：(子孙总数, 其中不在回收站的篇数)。
/// `Store::purge` 会递归删掉整棵子树（不管子孙是否在回收站），永久删除的确认弹窗用它说明连带影响。
pub(super) fn subtree_stats(docs: &[DocMeta], id: &str) -> (usize, usize) {
    let (mut kids, mut alive) = (0, 0);
    let mut stack = vec![id.to_string()];
    while let Some(cur) = stack.pop() {
        for d in docs.iter().filter(|d| d.parent.as_deref() == Some(cur.as_str())) {
            stack.push(d.id.clone());
            kids += 1;
            if !d.deleted {
                alive += 1;
            }
        }
    }
    (kids, alive)
}

/// 判断 `candidate` 是否是 `ancestor` 的子孙文档（防成环）。
pub(super) fn is_descendant(docs: &[DocMeta], ancestor: &str, candidate: &str) -> bool {
    let by_id: HashMap<&str, Option<&str>> = docs.iter().map(|d| (d.id.as_str(), d.parent.as_deref())).collect();
    let mut visited = HashSet::new();
    let mut cur = by_id.get(candidate).copied().flatten();
    while let Some(pid) = cur {
        if pid == ancestor {
            return true;
        }
        if !visited.insert(pid) {
            break;
        }
        cur = by_id.get(pid).copied().flatten();
    }
    false
}

/// 活文档的有效父节点（与侧边栏 `children_of` 保持一致：父文档已删或不存在时视为根目录）。
fn effective_parent(docs: &[DocMeta], alive: &HashSet<&str>, id: &str) -> Option<String> {
    docs.iter().find(|d| d.id == id && !d.deleted).and_then(|d| d.parent.clone()).filter(|p| alive.contains(p.as_str()))
}

/// 检查能否把 `doc_id` 移动到 `new_parent` 下（不能移到自身、已删除文档或自身子孙下）。
pub(super) fn can_move_to_parent(docs: &[DocMeta], doc_id: &str, new_parent: Option<&str>) -> bool {
    if !docs.iter().any(|d| d.id == doc_id && !d.deleted) {
        return false;
    }
    match new_parent {
        None => true,
        Some(pid) => {
            if pid == doc_id {
                return false;
            }
            if !docs.iter().any(|d| d.id == pid && !d.deleted) {
                return false;
            }
            !is_descendant(docs, doc_id, pid)
        }
    }
}

/// 根据落点计算移动计划；非法落点或移动后位置无变化时返回 `None`。
pub(super) fn plan_move(docs: &[DocMeta], doc_id: &str, zone: &DropZone) -> Option<MovePlan> {
    let alive: HashSet<&str> = docs.iter().filter(|d| !d.deleted).map(|d| d.id.as_str()).collect();
    if !alive.contains(doc_id) {
        return None;
    }
    let new_parent = match zone {
        DropZone::Before(target) | DropZone::After(target) => {
            if target == doc_id || !alive.contains(target.as_str()) {
                return None;
            }
            effective_parent(docs, &alive, target)
        }
        DropZone::Into(target) => Some(target.clone()),
        DropZone::RootStart | DropZone::RootEnd => None,
    };
    if !can_move_to_parent(docs, doc_id, new_parent.as_deref()) {
        return None;
    }

    let cur_siblings: Vec<String> =
        docs.iter().filter(|d| !d.deleted && d.parent.as_deref().filter(|p| alive.contains(*p)) == new_parent.as_deref()).map(|d| d.id.clone()).collect();

    let mut next_siblings: Vec<String> = cur_siblings.iter().filter(|s| s.as_str() != doc_id).cloned().collect();
    match zone {
        DropZone::Before(target) => {
            let pos = next_siblings.iter().position(|s| s == target)?;
            next_siblings.insert(pos, doc_id.to_string());
        }
        DropZone::After(target) => {
            let pos = next_siblings.iter().position(|s| s == target)?;
            next_siblings.insert(pos + 1, doc_id.to_string());
        }
        DropZone::Into(_) | DropZone::RootEnd => {
            next_siblings.push(doc_id.to_string());
        }
        DropZone::RootStart => {
            next_siblings.insert(0, doc_id.to_string());
        }
    }

    let old_parent = effective_parent(docs, &alive, doc_id);
    if old_parent == new_parent && cur_siblings == next_siblings {
        return None;
    }

    Some(MovePlan { new_parent, sibling_order: next_siblings })
}

/// 为「移动到…」弹窗生成按树形顺序排列的合法目标列表（自动排除自身及全部子孙）。
pub(super) fn move_targets(docs: &[DocMeta], doc_id: &str, query: &str) -> Vec<MoveTarget> {
    let alive: HashSet<&str> = docs.iter().filter(|d| !d.deleted).map(|d| d.id.as_str()).collect();
    if !alive.contains(doc_id) {
        return vec![];
    }
    let old_parent = effective_parent(docs, &alive, doc_id);
    let mut children: HashMap<Option<String>, Vec<usize>> = HashMap::new();
    for (i, d) in docs.iter().enumerate() {
        if d.deleted {
            continue;
        }
        let p = d.parent.clone().filter(|p| alive.contains(p.as_str()));
        children.entry(p).or_default().push(i);
    }

    let mut all = Vec::new();
    all.push(MoveTarget { parent: None, title: String::new(), depth: 0, path: String::new(), is_current: old_parent.is_none() });

    fn walk(
        parent: Option<String>,
        depth: usize,
        prefix: &str,
        doc_id: &str,
        old_parent: &Option<String>,
        docs: &[DocMeta],
        children: &HashMap<Option<String>, Vec<usize>>,
        out: &mut Vec<MoveTarget>,
    ) {
        let Some(kids) = children.get(&parent) else { return };
        for &i in kids {
            let d = &docs[i];
            // 跳过自身及整棵子树，防止成环
            if d.id == doc_id {
                continue;
            }
            let title = if d.title.trim().is_empty() { "未命名文档".to_string() } else { d.title.trim().to_string() };
            let next_prefix = if prefix.is_empty() { title.clone() } else { format!("{prefix} / {title}") };
            out.push(MoveTarget {
                parent: Some(d.id.clone()),
                title,
                depth,
                path: prefix.to_string(),
                is_current: old_parent.as_deref() == Some(d.id.as_str()),
            });
            walk(Some(d.id.clone()), depth + 1, &next_prefix, doc_id, old_parent, docs, children, out);
        }
    }
    walk(None, 0, "", doc_id, &old_parent, docs, &children, &mut all);

    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return all;
    }
    all.into_iter()
        .filter(|item| {
            if item.parent.is_none() {
                "根目录 顶层 root top level".contains(&q) || "根目录（顶层）".contains(&q)
            } else {
                item.title.to_lowercase().contains(&q) || item.path.to_lowercase().contains(&q)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: &str, parent: Option<&str>, deleted: bool) -> DocMeta {
        DocMeta { id: id.into(), parent: parent.map(Into::into), title: id.into(), sort: 0.0, updated: 0, deleted }
    }

    /// 含已恢复的子文档：purge 不看 deleted 标志，恢复过的子文档也会被一并删掉。
    #[test]
    fn counts_descendants_and_live_ones() {
        let docs = vec![meta("a", None, true), meta("b", Some("a"), true), meta("c", Some("b"), true), meta("d", Some("a"), false), meta("e", None, true)];
        assert_eq!(subtree_stats(&docs, "a"), (3, 1));
        assert_eq!(subtree_stats(&docs, "b"), (1, 0));
        assert_eq!(subtree_stats(&docs, "e"), (0, 0));
        assert_eq!(subtree_stats(&docs, "missing"), (0, 0));
    }

    #[test]
    fn plan_move_across_levels_and_reorders_siblings() {
        let docs = vec![meta("a", None, false), meta("a1", Some("a"), false), meta("a2", Some("a"), false), meta("b", None, false), meta("c", None, false)];

        // 根目录文档 c 移入 a 成为子文档（排在末尾）
        let p = plan_move(&docs, "c", &DropZone::Into("a".into())).unwrap();
        assert_eq!(p.new_parent.as_deref(), Some("a"));
        assert_eq!(p.sibling_order, vec!["a1", "a2", "c"]);

        // 子文档 a2 移到根目录 b 之前
        let p = plan_move(&docs, "a2", &DropZone::Before("b".into())).unwrap();
        assert_eq!(p.new_parent, None);
        assert_eq!(p.sibling_order, vec!["a", "a2", "b", "c"]);

        // 同级调序：c 移到 a 之前
        let p = plan_move(&docs, "c", &DropZone::Before("a".into())).unwrap();
        assert_eq!(p.new_parent, None);
        assert_eq!(p.sibling_order, vec!["c", "a", "b"]);

        // 子文档 a1 移到根目录末尾
        let p = plan_move(&docs, "a1", &DropZone::RootEnd).unwrap();
        assert_eq!(p.new_parent, None);
        assert_eq!(p.sibling_order, vec!["a", "b", "c", "a1"]);

        // 无变化的移动返回 None
        assert!(plan_move(&docs, "c", &DropZone::RootEnd).is_none());
        assert!(plan_move(&docs, "a", &DropZone::Before("b".into())).is_none());
        assert!(plan_move(&docs, "b", &DropZone::After("a".into())).is_none());
    }

    #[test]
    fn rejects_self_and_descendant_cycles() {
        let docs = vec![meta("a", None, false), meta("b", Some("a"), false), meta("c", Some("b"), false)];
        assert!(plan_move(&docs, "a", &DropZone::Into("a".into())).is_none());
        assert!(plan_move(&docs, "a", &DropZone::Into("b".into())).is_none());
        assert!(plan_move(&docs, "a", &DropZone::Into("c".into())).is_none());
        assert!(plan_move(&docs, "a", &DropZone::Before("c".into())).is_none());
        assert!(plan_move(&docs, "a", &DropZone::After("b".into())).is_none());
    }

    #[test]
    fn move_targets_excludes_self_and_subtree() {
        let docs =
            vec![meta("a", None, false), meta("a1", Some("a"), false), meta("a1x", Some("a1"), false), meta("b", None, false), meta("b1", Some("b"), false)];
        let targets = move_targets(&docs, "a", "");
        let ids: Vec<Option<&str>> = targets.iter().map(|t| t.parent.as_deref()).collect();
        assert_eq!(ids, vec![None, Some("b"), Some("b1")]);
        assert!(targets[0].is_current);
        assert!(!targets[1].is_current);

        let filtered = move_targets(&docs, "a", "b1");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].parent.as_deref(), Some("b1"));
        assert_eq!(filtered[0].path, "b");
    }
}
