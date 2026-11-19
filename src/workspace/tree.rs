//! 侧边栏文档树的纯计算（不碰 UI，方便单独测试）。

use crate::store::DocMeta;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: &str, parent: Option<&str>, deleted: bool) -> DocMeta {
        DocMeta { id: id.into(), parent: parent.map(Into::into), title: id.into(), updated: 0, deleted }
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
}
