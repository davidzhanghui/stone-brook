//! 首次启动的欢迎文档 + 百万字压测文档生成器。

use crate::convert::md_to_blocks;
use crate::model::{Block, BlockKind, DocContent, Marks, RichText};

const WELCOME_MD: &str = r#"
## 藏书于石，引水成渠

**石渠**是一款纯本地的块级文档编辑器：Rust 编写，GPUI 驱动，Metal 直接绘制。没有账号、没有云端，你的每一个字都只存在这台 Mac 上（只有「网页卡片」会按需联网，可在设置里一键关闭）。

> 💡 试试在空行输入 `/`，或者直接用 Markdown 语法写作——`# 空格` 变标题，`- 空格` 变列表，`**粗体**` 自动加粗。

## 快速上手

- [x] 打开石渠
- [ ] 在空行输入 `/` 唤起插入菜单
- [ ] 选中一段文字，试试浮动工具栏里的颜色
- [ ] 从网页复制一段带格式的内容，`⌘V` 粘贴，格式原样还原
- [ ] 按 `⌘P` 在所有文档中全文搜索
- [ ] 打开右上角 `···` → 生成百万字压测文档，感受一下帧耗时

## 常用快捷键

| 操作 | 快捷键 | 说明 |
| --- | --- | --- |
| 新建文档 | ⌘N | 在侧边栏顶层创建 |
| 快速打开 / 全文搜索 | ⌘P | 标题与正文都会被搜索 |
| 文档内查找 | ⌘F | 回车下一个，⇧回车上一个 |
| 粗体 / 斜体 / 下划线 | ⌘B / ⌘I / ⌘U | 无选区时作用于接下来输入的文字 |
| 删除线 / 行内代码 / 链接 | ⇧⌘X / ⌘E / ⌘K | 选中网址粘贴到文字上也能变链接 |
| 一 ~ 四级标题 | ⌥⌘1 ~ ⌥⌘4 | ⌥⌘0 恢复正文 |
| 有序 / 无序 / 待办 | ⇧⌘7 / ⇧⌘8 / ⇧⌘9 | ⌘↵ 勾选待办 |
| 上移 / 下移块 | ⌃⌘↑ / ⌃⌘↓ | 也可以拖动块左侧的 ⋮⋮ |
| 缩进 / 反缩进 | Tab / ⇧Tab | 表格里用于切换单元格 |
| 侧边栏 / 大纲 / 专注模式 | ⌘\ / ⇧⌘O / ⇧⌘F | 专注模式隐藏一切干扰 |
| 深浅色 / 性能面板 | ⇧⌘D / ⌥⌘P | 主题默认跟随系统 |
| 命令面板 / 设置 | ⇧⌘P / ⌘, | 所有功能都能在命令面板里搜到 |
| 演示模式 | F5 | 按二级标题自动分页，← → 翻页 |

## 支持的块

1. 正文、一至六级标题
2. 有序列表、无序列表、待办事项，支持多级缩进
    1. 有序列表的第二层用字母编号
        1. 第三层用罗马数字
3. 引用、高亮块、分割线
4. 代码块（带语法高亮）、表格、分栏、图片

```rust
// 代码块支持 18 种语言的语法高亮，点左上角切换语言
fn main() {
    let words = ["一方砚台", "研墨成文"];
    for w in words {
        println!("{w}");
    }
}
```

## 2.0 新能力

输入 `/` 可以插入这些新块：**图表**（Mermaid 流程图 / 时序图 / 类图 / 状态图 / ER 图 / 甘特图 / 饼图 / 思维导图 / 时间线）、**公式**、**画板**、**流程图**、**思维导图**、**电子表格**、**网页卡片**、**视频**、**文件**、**HTML**、**目录**、**倒计时**、**按钮**、**时间线**、**同步块**、**模板**。

```mermaid
flowchart LR
    A[在空行输入 /] --> B{想插入什么}
    B -->|图表| C[写 Mermaid 源码]
    B -->|公式| D[写 LaTeX]
    B -->|画板 / 表格| E[双击全屏编辑]
    C --> F[实时渲染]
    D --> F
```

公式既可以独占一块，也可以写在行内，比如质能方程 $E = mc^2$，输入 `$...$` 即可。

$$
\int_{-\infty}^{\infty} e^{-x^2}\,dx = \sqrt{\pi}
$$

- 粘贴一个网址，可以选择显示为「链接 / 标题 / 卡片」
- 块菜单里「设为同步块」，再在别的文档 `/同步块` 插入，改一处、处处更新
- 右上角 `···` → 以思维导图查看：标题和列表自动变成导图
- 右上角 `···` → 从模板新建：周报、会议纪要、OKR、读书笔记、项目计划……
- 设置里可以看到最近 20 周的写作热力图

---

## 写在最后

文字的格式可以很丰富：**粗体**、*Italic*、<u>下划线</u>、~~删除线~~、`行内代码`、[链接](https://github.com/zed-industries/zed)。

> 所有数据保存在 `~/Stonebrook/`，SQLite 单文件，随时可以备份带走。
"#;

pub fn welcome() -> DocContent {
    let mut blocks = md_to_blocks(WELCOME_MD);
    // Markdown 没有的块：分栏 + 带颜色的文字
    let mut c1 = RichText::plain("左栏：分栏适合对比两种方案。\n每一栏都可以独立编辑，换行用回车。");
    c1.update_marks(0..9, |m| m.bold = true);
    let mut c2 = RichText::plain("右栏：在分栏里点击，上方会出现「增加一栏 / 减少一栏」。");
    c2.update_marks(0..9, |m| m.bold = true);
    let cols = Block::new(BlockKind::Columns { cols: vec![c1, c2] }, RichText::default());
    let mut colorful = RichText::default();
    for (i, (s, c)) in [("红色", 1u8), ("橙色", 2), ("黄色", 3), ("绿色", 4), ("蓝色", 5), ("紫色", 6)].iter().enumerate() {
        if i > 0 {
            colorful.insert(colorful.len(), " · ", Marks::default());
        }
        colorful.insert(colorful.len(), s, Marks { color: *c, ..Default::default() });
    }
    colorful.insert(colorful.len(), "，以及 ", Marks::default());
    colorful.insert(colorful.len(), "背景高亮", Marks { bg: 3, ..Default::default() });
    colorful.insert(colorful.len(), "。", Marks::default());
    // 插在「支持的块」小节的代码块之前
    let at = blocks.iter().position(|b| matches!(b.kind, BlockKind::Code { .. })).unwrap_or(blocks.len());
    blocks.insert(at, Block::new(BlockKind::Text, colorful));
    blocks.insert(at + 1, cols);
    blocks.insert(0, Block::new(BlockKind::Title, RichText::plain("欢迎使用石渠 ✍️")));
    DocContent::from_blocks(blocks)
}

/// 生成约 `target_chars` 字的压测文档：标题、段落、列表、代码、表格混排。
pub fn stress(target_chars: usize) -> DocContent {
    const POEM: &str = "天地玄黄，宇宙洪荒。日月盈昃，辰宿列张。寒来暑往，秋收冬藏。闰余成岁，律吕调阳。云腾致雨，露结为霜。金生丽水，玉出昆冈。剑号巨阙，珠称夜光。果珍李柰，菜重芥姜。海咸河淡，鳞潜羽翔。龙师火帝，鸟官人皇。始制文字，乃服衣裳。推位让国，有虞陶唐。";
    let chars: Vec<char> = POEM.chars().collect();
    let mut blocks = vec![Block::new(BlockKind::Title, RichText::plain("百万字压测文档"))];
    let mut total = 0usize;
    let mut i = 0usize;
    let mut cursor = 0usize;
    let mut take = |n: usize| -> String {
        let s: String = (0..n).map(|k| chars[(cursor + k) % chars.len()]).collect();
        cursor += n;
        s
    };
    while total < target_chars {
        if i.is_multiple_of(40) {
            let s = format!("第 {} 章 · {}", i / 40 + 1, take(8));
            total += s.chars().count();
            blocks.push(Block::new(BlockKind::Heading { level: 1 }, RichText::plain(s)));
        } else if i.is_multiple_of(13) {
            let s = format!("{}.{} {}", i / 40 + 1, i % 40, take(6));
            total += s.chars().count();
            blocks.push(Block::new(BlockKind::Heading { level: 2 }, RichText::plain(s)));
        } else if i.is_multiple_of(17) {
            for _ in 0..4 {
                let s = take(28);
                total += s.chars().count();
                blocks.push(Block::new(BlockKind::Bullet, RichText::plain(s)));
            }
        } else if i.is_multiple_of(29) {
            let code = "fn render(frame: &mut Frame) -> Result<()> {\n    for block in frame.visible_blocks() {\n        block.paint()?; // 只绘制可见块\n    }\n    Ok(())\n}";
            total += code.chars().count();
            blocks.push(Block::new(BlockKind::Code { lang: "Rust".into(), wrap: false, height: None }, RichText::plain(code)));
        } else if i.is_multiple_of(37) {
            let mut t = Block::table(3, 3);
            if let BlockKind::Table { cells, .. } = &mut t.kind {
                for c in cells.iter_mut() {
                    *c = RichText::plain(take(6));
                    total += 6;
                }
            }
            blocks.push(t);
        } else {
            let mut r = RichText::plain(take(180));
            r.update_marks(0..12, |m| m.bold = true);
            if i.is_multiple_of(5) {
                r.update_marks(30..60, |m| m.bg = 3);
            }
            total += 180;
            blocks.push(Block::new(BlockKind::Text, r));
        }
        i += 1;
    }
    DocContent::from_blocks(blocks)
}

#[cfg(test)]
mod tests {
    #[test]
    fn welcome_and_stress_build() {
        let w = super::welcome();
        assert!(w.blocks.len() > 20);
        assert!(w.blocks.iter().any(|b| matches!(b.kind, crate::model::BlockKind::Columns { .. })));
        let s = super::stress(1_000_000);
        assert!(s.char_count() >= 900_000, "chars = {}", s.char_count());
    }
}
