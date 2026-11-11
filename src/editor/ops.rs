//! 编辑操作：输入、删除、回车、Tab、格式、块类型转换、Markdown 快捷语法、斜杠菜单、表格。

use super::*;
use crate::convert::normalize_lang;
use crate::line_input::{InputEvent, LineInput};
use crate::model::CellMerge;
use crate::model::{Block, BlockKind, Marks, RichText};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone)]
pub enum SlashAction {
    Kind(BlockKind),
    Table,
    Columns,
    Divider,
    Image,
    Date,
    Link,
    File,
    Drawio,
    Sync,
    Math,
    Template,
    ButtonUrl,
    ButtonDup,
    Embed(&'static str),
    Diagram(&'static str),
    WebCard,
    Countdown,
    Html,
    Toc,
    Timeline,
}

/// 图标：SVG 名称或文字徽标，外加主题色。
#[derive(Clone, Copy)]
pub enum SlashIcon {
    Svg(&'static str),
    Text(&'static str),
}

pub struct SlashItem {
    pub group: &'static str,
    pub icon: SlashIcon,
    pub color: u32,
    pub label: &'static str,
    pub hint: &'static str,
    pub keys: &'static str,
    pub action: SlashAction,
}

const FLOW_TPL: &str = "flowchart LR\n    A[开始] --> B{是否满足条件?}\n    B -->|是| C[执行任务]\n    B -->|否| D[跳过]\n    C --> E([结束])\n    D --> E";
const SEQ_TPL: &str = "sequenceDiagram\n    autonumber\n    participant 用户\n    participant 前端\n    participant 服务端\n    用户->>前端: 点击登录\n    前端->>+服务端: POST /login\n    服务端-->>-前端: 返回 token\n    alt 登录成功\n        前端-->>用户: 进入首页\n    else 登录失败\n        前端-->>用户: 提示错误\n    end";
const CLASS_TPL: &str = "classDiagram\n    class Animal {\n        <<abstract>>\n        +String name\n        +eat() void\n    }\n    class Dog {\n        +bark() void\n    }\n    class Cat {\n        +meow() void\n    }\n    Animal <|-- Dog\n    Animal <|-- Cat\n    Dog \"1\" o-- \"*\" Toy : 玩具";
const STATE_TPL: &str =
    "stateDiagram-v2\n    [*] --> 草稿\n    草稿 --> 审核中 : 提交\n    审核中 --> 已发布 : 通过\n    审核中 --> 草稿 : 驳回\n    已发布 --> [*]";
const GANTT_TPL: &str = "gantt\n    title 项目计划\n    dateFormat YYYY-MM-DD\n    section 设计\n    需求评审 :done, a1, 2026-11-02, 3d\n    交互设计 :active, a2, after a1, 5d\n    section 开发\n    后端接口 :b1, after a2, 7d\n    前端页面 :b2, after a2, 6d\n    上线 :milestone, after b1, 0d";
const PIE_TPL: &str = "pie title 时间都去哪儿了\n    \"写作\" : 42\n    \"阅读\" : 25\n    \"开会\" : 18\n    \"摸鱼\" : 15";
const ER_TPL: &str = "erDiagram\n    USER ||--o{ ORDER : 下单\n    ORDER ||--|{ ITEM : 包含\n    USER {\n        int id PK\n        string name\n    }\n    ORDER {\n        int id PK\n        date created\n    }";
const MIND_TPL: &str = "mindmap\n  root((石渠))\n    块编辑器\n      表格\n      代码块\n    绘图\n      流程图\n      时序图\n    本地优先";
pub(super) const HTML_TPL: &str = "<div style=\"padding:20px;border-radius:14px;background:linear-gradient(135deg,#3b6cf6,#8b5cf6);color:white\">\n  <h3 style=\"margin:0 0 6px\">Hello HTML 👋</h3>\n  <p style=\"margin:0;opacity:.9\">在下方编辑源码，预览实时更新</p>\n</div>";

pub fn slash_items() -> Vec<SlashItem> {
    use BlockKind::*;
    use SlashIcon::{Svg, Text as Tx};
    let it = |group, icon, color, label, hint, keys, action| SlashItem { group, icon, color, label, hint, keys, action };
    vec![
        it("基础", Svg("type"), 0x3b6cf6, "文本", "", "text wb wenben zhengwen p", SlashAction::Kind(BlockKind::Text)),
        it("基础", Tx("H1"), 0x3b6cf6, "一级标题", "#", "h1 heading bt biaoti yjbt", SlashAction::Kind(Heading { level: 1 })),
        it("基础", Tx("H2"), 0x3b6cf6, "二级标题", "##", "h2 heading bt biaoti ejbt", SlashAction::Kind(Heading { level: 2 })),
        it("基础", Tx("H3"), 0x3b6cf6, "三级标题", "###", "h3 heading bt biaoti sjbt", SlashAction::Kind(Heading { level: 3 })),
        it("基础", Tx("H4"), 0x3b6cf6, "四级标题", "####", "h4 heading bt biaoti", SlashAction::Kind(Heading { level: 4 })),
        it("基础", Svg("list-ordered"), 0x5b5bd6, "有序列表", "1.", "ol ordered number list yxlb liebiao", SlashAction::Kind(Ordered)),
        it("基础", Svg("list"), 0x5b5bd6, "无序列表", "-", "ul bullet list wxlb liebiao", SlashAction::Kind(Bullet)),
        it("基础", Svg("braces"), 0x2ea121, "代码块", "```", "code dmk daima", SlashAction::Kind(Code { lang: "Plain Text".into() })),
        it("基础", Svg("quote"), 0x3b6cf6, "引用", ">", "quote yy yinyong", SlashAction::Kind(Quote)),
        it("基础", Svg("divider"), 0xf5a339, "分隔线", "---", "hr divider line fgx fenge fengexian fenge", SlashAction::Divider),
        it("基础", Svg("link"), 0x3b6cf6, "链接", "⌘K", "link lj lianjie url chaolianjie", SlashAction::Link),
        it(
            "常用",
            Svg("square-check"),
            0x5b5bd6,
            "任务",
            "[]",
            "todo task check rw renwu db daiban daibanshixiang",
            SlashAction::Kind(Todo { checked: false }),
        ),
        it("常用", Svg("image"), 0xf5a339, "图片", "", "image img tp tupian picture", SlashAction::Image),
        it("常用", Svg("paperclip"), 0x0ea5e9, "视频或文件", "", "file video sp wj shipin wenjian fujian attachment shipinhuowenjian", SlashAction::File),
        it("常用", Svg("table"), 0x14b8a6, "表格", "", "table bg biaoge", SlashAction::Table),
        it("常用", Svg("columns"), 0x5b5bd6, "分栏", "", "columns fl fenlan grid", SlashAction::Columns),
        it(
            "常用",
            Svg("callout"),
            0xf5a339,
            "高亮块",
            "",
            "callout glk gaoliang highlight gaoliangkuai",
            SlashAction::Kind(Callout { emoji: "💡".into(), color: 5 }),
        ),
        it("常用", Svg("presentation"), 0x8f959e, "演讲者备注", "", "notes speaker bz beizhu yanjiang yjzbz yanjiangzhebeizhu", SlashAction::Kind(Notes)),
        it("常用", Svg("sync"), 0x3b6cf6, "同步块", "", "sync tbk tongbu tongbukuai", SlashAction::Sync),
        it("常用", Svg("sigma"), 0x1f2329, "公式", "$$", "math formula gs gongshi latex tex shuxue", SlashAction::Math),
        it("常用", Svg("template"), 0xec4899, "模板", "", "template mb muban", SlashAction::Template),
        it("按钮", Svg("external"), 0x5b5bd6, "打开超链接", "", "button an anniu link open dakai chaolianjie dakaichaolianjie", SlashAction::ButtonUrl),
        it("按钮", Svg("copy"), 0x5b5bd6, "创建副本", "", "button an anniu copy duplicate fuben chuangjian chuangjianfuben", SlashAction::ButtonDup),
        it("数据", Svg("sheet"), 0x2ea121, "电子表格", "", "sheet excel dzbg biaoge spreadsheet dianzibiaoge", SlashAction::Embed("sheet")),
        it(
            "多维表格",
            Svg("v-grid"),
            0x3370ff,
            "表格",
            "",
            "base bitable dwbg duowei duoweibiaoge biaoge bg table grid shujubiao",
            SlashAction::Embed("base:grid"),
        ),
        it("多维表格", Svg("v-kanban"), 0x2ea121, "看板", "", "kanban kb kanban board renwu", SlashAction::Embed("base:kanban")),
        it("多维表格", Svg("square-check"), 0x3b6cf6, "任务看板", "", "task kanban rwkb renwu renwukanban todo daiban", SlashAction::Embed("base:tasks")),
        it("多维表格", Svg("calendar"), 0xff8800, "日程", "", "schedule calendar rc richeng rili riqi", SlashAction::Embed("base:schedule")),
        it("多维表格", Svg("gantt"), 0xf14c9d, "甘特图", "", "gantt gtt gante gantetu xiangmu", SlashAction::Embed("base:gantt")),
        it("多维表格", Svg("v-gallery"), 0x7f3bf5, "画册", "", "gallery hc huace album tupian", SlashAction::Embed("base:gallery")),
        it("多维表格", Svg("base"), 0x7f3bf5, "关联已有多维表格", "", "base link glyy guanlian yiyou duoweibiaoge", SlashAction::Embed("base:link")),
        it("绘图", Svg("download"), 0xf08c00, "导入 draw.io", "", "drawio draw.io diagrams dr daoru import mxgraph", SlashAction::Drawio),
        it("绘图", Svg("pen-tool"), 0x2ea121, "画板", "", "board whiteboard hb huaban draw", SlashAction::Embed("board")),
        it("绘图", Svg("mindmap"), 0x0ea5e9, "思维导图", "", "mindmap swdt siwei naotu siweidaotu", SlashAction::Embed("mindmap")),
        it("绘图", Svg("workflow"), 0xf5a339, "流程图", "", "flow lct liucheng flowchart liuchengtu", SlashAction::Embed("flow")),
        it("绘图", Svg("pencil"), 0x6965db, "草图", "Excalidraw", "excalidraw sketch caotu cao tu shouhui", SlashAction::Embed("sketch")),
        it("绘图", Tx("UML"), 0x5b5bd6, "UML 时序图", "", "uml sequence sxt shixu shixutu", SlashAction::Diagram(SEQ_TPL)),
        it("绘图", Tx("UML"), 0x5b5bd6, "UML 类图", "", "uml class lt leitu", SlashAction::Diagram(CLASS_TPL)),
        it("绘图", Tx("UML"), 0x5b5bd6, "UML 状态图", "", "uml state ztt zhuangtai zhuangtaitu", SlashAction::Diagram(STATE_TPL)),
        it("绘图", Svg("shapes"), 0x8b5cf6, "文本绘图", "mermaid", "mermaid wbht text diagram huitu wenbenhuitu wenben", SlashAction::Diagram(FLOW_TPL)),
        it("绘图", Svg("gantt"), 0x14b8a6, "文本甘特图", "", "gantt wbgtt text gantt gantetu wenben", SlashAction::Diagram(GANTT_TPL)),
        it("绘图", Svg("chart-pie"), 0xec4899, "饼图", "", "pie bt bingtu chart", SlashAction::Diagram(PIE_TPL)),
        it("绘图", Svg("database"), 0x64748b, "ER 图", "", "er erdiagram shiti database", SlashAction::Diagram(ER_TPL)),
        it("绘图", Svg("mindmap"), 0x8b5cf6, "文本思维导图", "", "mindmap text wbsw", SlashAction::Diagram(MIND_TPL)),
        it("进阶", Svg("globe"), 0x14b8a6, "网页卡片", "", "web card wykp wangye bookmark url wangyekapian kapian", SlashAction::WebCard),
        it("进阶", Svg("hourglass"), 0xf5a339, "倒计时", "", "countdown djs daojishi timer", SlashAction::Countdown),
        it("进阶", Svg("calendar"), 0xe5484d, "今天日期", "", "date today rq riqi", SlashAction::Date),
        it("文档小组件", Svg("code-xml"), 0x3b6cf6, "HTML 块", "", "html wy code htmlkuai", SlashAction::Html),
        it("文档小组件", Svg("list-tree"), 0x5b5bd6, "目录导航", "", "toc ml mulu daohang muludaohang", SlashAction::Toc),
        it("文档小组件", Svg("timeline"), 0x3b6cf6, "时间轴", "", "timeline sjz shijianzhou", SlashAction::Timeline),
    ]
}

pub fn filter_slash(query: &str) -> Vec<SlashItem> {
    let q = query.to_lowercase();
    if q.is_empty() {
        return slash_items();
    }
    // 排序：关键字完全匹配 > 标题包含 > 关键字前缀匹配
    let mut scored: Vec<(u8, SlashItem)> = slash_items()
        .into_iter()
        .filter_map(|it| {
            let score = if it.keys.split(' ').any(|k| k == q) {
                0
            } else if it.label.contains(&q) || crate::i18n::tr(it.label).to_lowercase().contains(&q) {
                1
            } else if it.keys.split(' ').any(|k| k.starts_with(&q)) {
                2
            } else if q.chars().count() >= 3 && it.keys.split(' ').any(|k| k.len() >= 4 && k.contains(&q)) {
                3
            } else {
                return None;
            };
            Some((score, it))
        })
        .collect();
    scored.sort_by_key(|(s, _)| *s);
    scored.into_iter().map(|(_, it)| it).collect()
}

impl Editor {
    pub(super) fn current_marks(&self) -> Marks {
        if let Some(m) = &self.core.pending_marks {
            return m.clone();
        }
        self.slot_text(self.core.head.slot()).map(|t| t.marks_at(self.core.head.offset)).unwrap_or_default()
    }

    /// 所有被选中的 (文本槽, 区间)。
    pub fn selected_slots(&self) -> Vec<((usize, usize), Range<usize>)> {
        let (s, e) = self.sel();
        let mut out = vec![];
        for b in s.block..=e.block.min(self.content.blocks.len() - 1) {
            for c in 0..self.content.blocks[b].slot_count() {
                if let Some(r) = self.sel_in_slot((b, c)) {
                    if !r.is_empty() {
                        out.push(((b, c), r));
                    }
                }
            }
        }
        out
    }

    // ───────────── 输入 ─────────────

    pub fn insert_text(&mut self, s: &str, cx: &mut Context<Self>) {
        if s.is_empty() {
            return;
        }
        self.sess.url_paste = None;
        self.checkpoint(EditKind::Typing);
        if self.has_selection() {
            self.delete_selection_inner();
        }
        let h = self.core.head;
        if self.content.blocks[h.block].kind.is_void() {
            // 后面紧跟空段落时直接复用它，否则新建段落
            let next_empty = self.content.blocks.get(h.block + 1).is_some_and(|b| matches!(b.kind, BlockKind::Text) && b.text.text.is_empty());
            if next_empty {
                self.slot_mut((h.block + 1, 0)).insert(0, s, Default::default());
            } else {
                self.insert_blocks(h.block + 1, vec![Block::text(s)]);
            }
            self.core.head = Pos::new(h.block + 1, 0, s.len());
            self.core.anchor = self.core.head;
            let mut chars = s.chars();
            if let (Some(ch), None) = (chars.next(), chars.next()) {
                self.after_char_typed(ch);
            }
            self.update_slash();
            self.changed(cx);
            self.reveal_head();
            return;
        }
        let s = if h.block == 0 { s.replace('\n', " ") } else { s.to_string() };
        let marks = self.current_marks();
        self.slot_mut(h.slot()).insert(h.offset, &s, marks);
        self.core.head.offset += s.len();
        self.core.anchor = self.core.head;
        self.typed_today += s.chars().filter(|c| !c.is_whitespace()).count();
        let mut chars = s.chars();
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            self.after_char_typed(ch);
        }
        self.update_slash();
        self.changed(cx);
        self.reveal_head();
    }

    fn after_char_typed(&mut self, ch: char) {
        let h = self.core.head;
        let kind = self.content.blocks[h.block].kind.clone();
        if matches!(ch, '@' | '＠') {
            self.open_mention();
            return;
        }
        if (ch == '/' || ch == '、') && h.block > 0 && kind.is_textual() && !matches!(kind, BlockKind::Code { .. }) {
            self.pop.slash = Some(SlashMenu { block: h.block, start: h.offset - ch.len_utf8(), selected: 0, scrolled: Default::default() });
            return;
        }
        if ch == ' ' && kind == BlockKind::Text && h.cell == 0 {
            self.try_block_shortcut();
            return;
        }
        if matches!(ch, '*' | '`' | '~') && !matches!(kind, BlockKind::Code { .. }) {
            self.try_inline_shortcut();
        }
        if ch == '$' && !kind.is_source() && !matches!(kind, BlockKind::Code { .. }) {
            self.try_inline_math();
        }
        // 代码块：在只有缩进的行里输入右括号，自动回退一级缩进
        if matches!(ch, '}' | ']' | ')') && matches!(kind, BlockKind::Code { .. }) {
            let t = &self.content.blocks[h.block].text.text;
            let before = &t[..h.offset - 1];
            let line = before.rsplit('\n').next().unwrap_or("");
            if line.len() >= 4 && line.chars().all(|c| c == ' ') {
                let at = h.offset - 1 - 4;
                self.block_mut(h.block).text.delete(at..at + 4);
                self.core.head.offset -= 4;
                self.core.anchor = self.core.head;
            }
        }
    }

    /// 行首 Markdown 语法：`# `、`- `、`1. `、`[] `、`> `、```` ``` ````……
    fn try_block_shortcut(&mut self) {
        let h = self.core.head;
        let text = &self.content.blocks[h.block].text.text;
        let prefix = &text[..h.offset - 1];
        let kind = match prefix {
            "#" => BlockKind::Heading { level: 1 },
            "##" => BlockKind::Heading { level: 2 },
            "###" => BlockKind::Heading { level: 3 },
            "####" => BlockKind::Heading { level: 4 },
            "#####" => BlockKind::Heading { level: 5 },
            "######" => BlockKind::Heading { level: 6 },
            "-" | "*" | "+" | "·" => BlockKind::Bullet,
            "[]" | "[ ]" | "【】" => BlockKind::Todo { checked: false },
            "[x]" | "[X]" => BlockKind::Todo { checked: true },
            ">" | "》" => BlockKind::Quote,
            "```" | "···" => BlockKind::Code { lang: "Plain Text".into() },
            p if p.len() >= 2
                && (p.ends_with('.') || p.ends_with('、') || p.ends_with(')'))
                && p.trim_end_matches(['.', '、', ')']).chars().all(|c| c.is_ascii_digit())
                && !p.trim_end_matches(['.', '、', ')']).is_empty() =>
            {
                BlockKind::Ordered
            }
            _ => return,
        };
        let b = self.block_mut(h.block);
        b.text.delete(0..h.offset);
        b.kind = kind;
        self.core.head.offset = 0;
        self.core.anchor = self.core.head;
        self.core.pending_marks = None;
    }

    /// 行内 Markdown：`**粗体**`、`*斜体*`、`~~删除线~~`、`` `代码` ``。
    fn try_inline_shortcut(&mut self) {
        let h = self.core.head;
        let Some(t) = self.slot_text(h.slot()) else { return };
        let before = &t.text[..h.offset];
        let rules: [(&str, fn(&mut Marks)); 4] = [("**", |m| m.bold = true), ("~~", |m| m.strike = true), ("`", |m| m.code = true), ("*", |m| m.italic = true)];
        for (d, apply) in rules {
            if !before.ends_with(d) {
                continue;
            }
            let inner_end = h.offset - d.len();
            let Some(open) = before[..inner_end].rfind(d) else { continue };
            let inner_start = open + d.len();
            if inner_start >= inner_end {
                continue;
            }
            let inner = &before[inner_start..inner_end];
            if inner.starts_with(' ') || inner.ends_with(' ') || inner.contains(d) {
                continue;
            }
            if d == "*" && (before[..open].ends_with('*') || inner.starts_with('*')) {
                continue;
            }
            let inner_len = inner.len();
            let slot = self.slot_mut(h.slot());
            slot.delete(inner_end..h.offset);
            slot.delete(open..inner_start);
            slot.update_marks(open..open + inner_len, apply);
            self.core.head.offset = open + inner_len;
            self.core.anchor = self.core.head;
            // 之后输入的文字不再继承该样式
            let mut m = self.slot_text(h.slot()).unwrap().marks_at(self.core.head.offset);
            let mut probe = Marks::default();
            apply(&mut probe);
            if probe.bold {
                m.bold = false
            }
            if probe.italic {
                m.italic = false
            }
            if probe.strike {
                m.strike = false
            }
            if probe.code {
                m.code = false
            }
            self.core.pending_marks = Some(m);
            return;
        }
    }

    /// `$E=mc^2$` → 行内公式
    fn try_inline_math(&mut self) {
        let h = self.core.head;
        let Some(t) = self.slot_text(h.slot()) else { return };
        let before = &t.text[..h.offset - 1];
        let Some(open) = before.rfind('$') else { return };
        let inner = &before[open + 1..];
        if inner.is_empty() || inner.starts_with(' ') || inner.ends_with(' ') || before[..open].ends_with('$') {
            return;
        }
        let tex = inner.to_string();
        let slot = self.slot_mut(h.slot());
        slot.delete(open..h.offset);
        slot.insert_math(open, &tex);
        self.core.head.offset = open + 3;
        self.core.anchor = self.core.head;
        self.core.pending_marks = Some(Marks::default());
    }

    pub(super) fn update_slash(&mut self) {
        self.update_mention();
        let Some(sm) = &self.pop.slash else { return };
        let h = self.core.head;
        let close = (|| -> Option<bool> {
            if h.block != sm.block || h.cell != 0 || h.offset <= sm.start {
                return Some(true);
            }
            let t = &self.content.blocks.get(sm.block)?.text.text;
            if !t[sm.start..].starts_with(['/', '、']) {
                return Some(true);
            }
            let first_len = t[sm.start..].chars().next()?.len_utf8();
            let q = t.get(sm.start + first_len..h.offset)?;
            Some(q.contains(char::is_whitespace) || q.chars().count() > 12 || filter_slash(q).is_empty())
        })()
        .unwrap_or(true);
        if close {
            self.pop.slash = None;
        } else {
            let n = filter_slash(&self.slash_query()).len();
            let sm = self.pop.slash.as_mut().unwrap();
            sm.selected = sm.selected.min(n.saturating_sub(1));
        }
    }

    pub fn slash_query(&self) -> String {
        let Some(sm) = &self.pop.slash else { return String::new() };
        let t = &self.content.blocks[sm.block].text.text;
        let first_len = t[sm.start..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        t.get(sm.start + first_len..self.core.head.offset).unwrap_or("").to_string()
    }

    pub fn apply_slash(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let items = filter_slash(&self.slash_query());
        let Some(sm) = self.pop.slash.take() else { return };
        let Some(item) = items.into_iter().nth(index) else { return };
        self.checkpoint(EditKind::Other);
        let bi = sm.block;
        let end = self.core.head.offset.max(sm.start);
        self.block_mut(bi).text.delete(sm.start..end);
        self.core.head = Pos::new(bi, 0, sm.start);
        self.core.anchor = self.core.head;
        let current_empty = self.content.blocks[bi].text.is_empty() && self.content.blocks[bi].kind == BlockKind::Text;
        let insert_new = |this: &mut Self, blk: Block| -> usize {
            let at = if current_empty {
                let indent = this.content.blocks[bi].indent;
                let sync = this.content.blocks[bi].sync.clone();
                let b = this.block_mut(bi);
                *b = blk;
                b.indent = indent;
                b.sync = sync;
                bi
            } else {
                this.insert_blocks(bi + 1, vec![blk]);
                bi + 1
            };
            if at + 1 >= this.content.blocks.len() {
                this.insert_blocks(at + 1, vec![Block::text("")]);
            }
            at
        };
        let caret_pos = self.frame.borrow().cursor.map(|c| c.origin).unwrap_or_default();
        match item.action {
            SlashAction::Kind(k) => {
                self.block_mut(bi).kind = k;
            }
            SlashAction::Divider => {
                let at = insert_new(self, Block::new(BlockKind::Divider, RichText::default()));
                self.core.head = Pos::new(at + 1, 0, 0);
            }
            SlashAction::Table => {
                // 鼠标悬停时右侧已弹出行列选择器（见 show_slash_table_picker）；
                // 直接点击「表格」则插入默认 3×3。
                self.pop.table_picker = None;
                let at = insert_new(self, Block::table(3, 3));
                self.core.head = Pos::new(at, 0, 0);
            }
            SlashAction::Columns => {
                let at = insert_new(self, Block::columns(2));
                self.core.head = Pos::new(at, 0, 0);
            }
            SlashAction::Date => {
                let now = chrono::Local::now();
                let wd = ["一", "二", "三", "四", "五", "六", "日"][now.format("%u").to_string().parse::<usize>().unwrap_or(1) - 1];
                let s = format!("{} 周{}", now.format("%Y-%m-%d"), wd);
                let m = self.current_marks();
                self.block_mut(bi).text.insert(sm.start, &s, m);
                self.core.head.offset = sm.start + s.len();
            }
            SlashAction::Image => {
                self.core.anchor = self.core.head;
                self.changed(cx);
                self.pick_images(window, cx);
                return;
            }
            SlashAction::File => {
                self.core.anchor = self.core.head;
                self.changed(cx);
                self.pick_files(window, cx);
                return;
            }
            SlashAction::Drawio => {
                self.core.anchor = self.core.head;
                self.changed(cx);
                self.pick_drawio(cx);
                return;
            }
            SlashAction::Link => {
                self.core.anchor = self.core.head;
                self.changed(cx);
                self.open_prop_edit(bi, "insert_link", "", "粘贴或输入网址，回车插入链接", caret_pos, window, cx);
                return;
            }
            SlashAction::Math => {
                let at = insert_new(self, Block::new(BlockKind::Math, RichText::default()));
                self.core.head = Pos::new(at, 0, 0);
            }
            SlashAction::Diagram(tpl) => {
                let at = insert_new(self, Block::new(BlockKind::Diagram { view: 0, offsets: Default::default() }, RichText::plain(tpl)));
                self.core.head = Pos::new(at, 0, 0);
            }
            SlashAction::Html => {
                let at = insert_new(self, Block::new(BlockKind::Html { scripts: false, file: String::new(), network: true }, RichText::default()));
                self.core.head = Pos::new(at, 0, 0);
            }
            SlashAction::Toc => {
                let at = insert_new(self, Block::new(BlockKind::Toc, RichText::default()));
                self.core.head = Pos::new(at + 1, 0, 0);
            }
            SlashAction::Timeline => {
                // 两个空节点，水平交替排列，光标进第一个节点的标题
                let items = vec![crate::model::TimelineItem::default(), crate::model::TimelineItem::default()];
                let at = insert_new(self, Block::new(BlockKind::Timeline { items, vertical: false, same_side: false, hide: 0 }, RichText::default()));
                self.core.head = Pos::new(at, 0, 0);
            }
            SlashAction::Countdown => {
                // 插入后显示 00:00:00:00，悬停点「设置」再填时长或日期
                let target = chrono::Local::now().timestamp_millis();
                let at = insert_new(self, Block::new(BlockKind::Countdown { target, color: 1, notify: false, dur: Some(0) }, RichText::default()));
                self.core.head = Pos::new(at, 0, 0);
            }
            SlashAction::ButtonUrl | SlashAction::ButtonDup => {
                let (action, label) =
                    if matches!(item.action, SlashAction::ButtonUrl) { ("open_url", "打开超链接") } else { ("duplicate_doc", "创建副本") };
                let style = crate::model::button_style(crate::model::BUTTON_DEFAULT_COLOR);
                let at = insert_new(self, Block::new(BlockKind::Button { action: action.into(), url: String::new(), style }, RichText::plain(label)));
                self.core.head = Pos::new(at, 0, 0);
                self.core.anchor = self.core.head;
                self.changed(cx);
                self.open_button_edit(at, true, window, cx);
                return;
            }
            SlashAction::WebCard => {
                let at = insert_new(self, Block::new(BlockKind::LinkCard { url: String::new() }, RichText::default()));
                self.core.head = Pos::new(at, 0, 0);
                self.core.anchor = self.core.head;
                self.changed(cx);
                self.open_prop_edit(at, "link_url", "", "输入网址，回车生成网页卡片", caret_pos, window, cx);
                return;
            }
            SlashAction::Embed("base:link") => {
                self.core.anchor = self.core.head;
                self.changed(cx);
                cx.emit(EditorEvent::PickBase);
                return;
            }
            SlashAction::Embed(kind) if kind.starts_with("base:") => {
                use crate::base::ViewKind;
                let id = match &kind[5..] {
                    p @ ("tasks" | "schedule") => crate::objects::create_base_preset(p),
                    k => crate::objects::create_base(match k {
                        "kanban" => ViewKind::Kanban,
                        "gallery" => ViewKind::Gallery,
                        "gantt" => ViewKind::Gantt,
                        _ => ViewKind::Grid,
                    }),
                };
                let at = insert_new(self, Block::new(BlockKind::Embed { kind: "base".into(), id: id.clone() }, RichText::default()));
                self.core.head = Pos::new(at, 0, 0);
                self.core.anchor = self.core.head;
                self.changed(cx);
                cx.emit(EditorEvent::OpenEmbed { kind: "base".into(), id });
                return;
            }
            SlashAction::Embed("mindmap") => {
                // 思维导图就是一个预置了导图的画板，插入后直接进入画板编辑
                let doc = crate::board::model::BoardDoc::with_mind(None, 0, crate::theme::theme(cx).dark);
                let id = crate::objects::create(crate::objects::ObjKind::Board, Some(crate::objects::ObjData::Board(Arc::new(doc))));
                let at = insert_new(self, Block::new(BlockKind::Embed { kind: "board".into(), id: id.clone() }, RichText::default()));
                self.core.head = Pos::new(at, 0, 0);
                self.core.anchor = self.core.head;
                self.changed(cx);
                cx.emit(EditorEvent::OpenEmbed { kind: "board".into(), id });
                return;
            }
            SlashAction::Embed(kind) => {
                let Some(k) = crate::objects::ObjKind::parse(kind) else { return };
                let id = crate::objects::create(k, None);
                let at = insert_new(self, Block::new(BlockKind::Embed { kind: kind.into(), id: id.clone() }, RichText::default()));
                self.core.head = Pos::new(at, 0, 0);
                self.core.anchor = self.core.head;
                self.changed(cx);
                cx.emit(EditorEvent::OpenEmbed { kind: kind.into(), id });
                return;
            }
            SlashAction::Template => {
                self.core.anchor = self.core.head;
                self.changed(cx);
                cx.emit(EditorEvent::PickTemplate);
                return;
            }
            SlashAction::Sync => {
                self.core.anchor = self.core.head;
                self.changed(cx);
                self.pop.sync_menu = true;
                cx.notify();
                return;
            }
        }
        self.core.anchor = self.core.head;
        self.changed(cx);
        self.reveal_head();
    }

    // ───────────── 删除 ─────────────

    fn clear_cells(b: &mut Block, from: (usize, usize), to: (usize, usize)) {
        for c in from.0..=to.0 {
            let Some(t) = b.slot_mut(c) else { continue };
            let s = if c == from.0 { from.1 } else { 0 };
            let e = if c == to.0 { to.1 } else { t.len() };
            t.delete(s..e.max(s));
        }
    }

    /// 删除选区（不记录撤销点、不通知）。
    pub(super) fn delete_selection_inner(&mut self) {
        let (s, e) = self.sel();
        self.core.marked = None;
        if s == e {
            return;
        }
        if s.slot() == e.slot() {
            self.slot_mut(s.slot()).delete(s.offset..e.offset);
            self.core.head = s;
            self.core.anchor = s;
            return;
        }
        if let Some((r1, c1, r2, c2)) = self.table_sel_rect(s.block) {
            // 表格多格选区：清空矩形内每一格（不删行列）
            let mut origin = s.cell;
            if let BlockKind::Table { cols, cells, .. } = &mut self.block_mut(s.block).kind {
                for r in r1..=r2 {
                    for c in c1..=c2 {
                        if let Some(t) = cells.get_mut(r * *cols + c) {
                            *t = RichText::default();
                        }
                    }
                }
                origin = r1 * *cols + c1;
            }
            self.core.head = Pos::new(s.block, origin, 0);
            self.core.anchor = self.core.head;
            return;
        }
        if s.block == e.block {
            Self::clear_cells(self.block_mut(s.block), (s.cell, s.offset), (e.cell, e.offset));
            self.core.head = s;
            self.core.anchor = s;
            return;
        }
        let (sb, eb) = (s.block, e.block);
        let sblk = (*self.content.blocks[sb]).clone();
        let eblk = (*self.content.blocks[eb]).clone();
        let mut start_keep = None;
        if sblk.kind.is_textual() {
            let mut b = sblk;
            let len = b.text.len();
            b.text.delete(s.offset..len);
            start_keep = Some(b);
        } else if sblk.kind.is_multi() && !(s.cell == 0 && s.offset == 0) {
            let mut b = sblk;
            let last = b.slot_count() - 1;
            let last_len = b.slot(last).map(|t| t.len()).unwrap_or(0);
            Self::clear_cells(&mut b, (s.cell, s.offset), (last, last_len));
            start_keep = Some(b);
        }
        let mut end_keep = None;
        if eblk.kind.is_textual() {
            let mut b = eblk;
            b.text.delete(0..e.offset);
            end_keep = Some(b);
        } else if eblk.kind.is_multi() {
            let last = eblk.slot_count() - 1;
            let at_end = e.cell == last && e.offset >= eblk.slot(last).map(|t| t.len()).unwrap_or(0);
            if !at_end {
                let mut b = eblk;
                Self::clear_cells(&mut b, (0, 0), (e.cell, e.offset));
                end_keep = Some(b);
            }
        }
        let start_kept = start_keep.is_some();
        let mut out: Vec<Block> = Vec::new();
        match (start_keep, end_keep) {
            (Some(mut a), Some(b)) if a.kind.is_textual() && b.kind.is_textual() => {
                let mut tail = b.text;
                if a.kind == BlockKind::Title {
                    tail = RichText::plain(tail.text.replace('\n', " "));
                }
                a.text.append(&tail);
                out.push(a);
            }
            (a, b) => {
                out.extend(a);
                out.extend(b);
            }
        }
        if out.is_empty() {
            out.push(Block::text(""));
        }
        let n = out.len();
        self.content.blocks.splice(sb..=eb, out.into_iter().map(Arc::new));
        self.list.splice(sb..eb + 1, n);
        // 从标题选到最后一个块的末尾时只剩标题：和 remove_blocks 一样补一个空段落
        if self.content.blocks.len() == 1 {
            self.insert_blocks(1, vec![Block::text("")]);
        }
        let cursor = if start_kept { s } else { Pos::new(sb, 0, 0) };
        self.core.head = self.clamp(cursor);
        self.core.anchor = self.core.head;
    }

    pub fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.has_selection() {
            self.checkpoint(EditKind::Delete);
            self.delete_selection_inner();
            self.changed(cx);
            return;
        }
        let h = self.core.head;
        let kind = self.content.blocks[h.block].kind.clone();
        if kind.is_void() {
            self.checkpoint(EditKind::Other);
            self.remove_blocks(h.block..h.block + 1);
            let p = if h.block > 1 { self.slot_end(h.block - 1) } else { Pos::new(1, 0, 0) };
            self.core.head = self.clamp(p);
            self.core.anchor = self.core.head;
            self.changed(cx);
            return;
        }
        if h.offset > 0 {
            self.checkpoint(EditKind::Delete);
            let t = self.slot_text(h.slot()).unwrap();
            let prev = t.text[..h.offset].grapheme_indices(true).next_back().map(|(i, _)| i).unwrap_or(0);
            self.slot_mut(h.slot()).delete(prev..h.offset);
            self.core.head.offset = prev;
            self.core.anchor = self.core.head;
            self.update_slash();
            self.changed(cx);
            return;
        }
        // 位于文本槽开头
        if kind.is_multi() {
            if h.cell > 0 {
                let p = Pos::new(h.block, h.cell - 1, self.slot_len((h.block, h.cell - 1)));
                self.set_cursor(p, false, cx);
            } else if h.block > 1 {
                let p = self.slot_end(h.block - 1);
                self.set_cursor(p, false, cx);
            }
            return;
        }
        if h.block == 0 {
            return;
        }
        self.checkpoint(EditKind::Other);
        if kind != BlockKind::Text {
            self.block_mut(h.block).kind = BlockKind::Text;
            self.changed(cx);
            return;
        }
        if self.content.blocks[h.block].indent > 0 {
            self.block_mut(h.block).indent -= 1;
            self.changed(cx);
            return;
        }
        let prev = h.block - 1;
        let pk = self.content.blocks[prev].kind.clone();
        let cur_text = self.content.blocks[h.block].text.clone();
        if pk.is_textual() {
            let plen = self.content.blocks[prev].text.len();
            let tail = if pk == BlockKind::Title { RichText::plain(cur_text.text.replace('\n', " ")) } else { cur_text };
            self.block_mut(prev).text.append(&tail);
            self.remove_blocks(h.block..h.block + 1);
            self.core.head = Pos::new(prev, 0, plen);
        } else if pk.is_void() {
            if cur_text.is_empty() {
                self.remove_blocks(h.block..h.block + 1);
                self.core.head = Pos::new(prev, 0, 0);
            } else {
                self.remove_blocks(prev..prev + 1);
                self.core.head = Pos::new(prev, 0, 0);
            }
        } else {
            if cur_text.is_empty() {
                self.remove_blocks(h.block..h.block + 1);
            }
            self.core.head = self.slot_end(prev);
        }
        self.core.head = self.clamp(self.core.head);
        self.core.anchor = self.core.head;
        self.changed(cx);
    }

    pub fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.has_selection() {
            self.checkpoint(EditKind::Delete);
            self.delete_selection_inner();
            self.changed(cx);
            return;
        }
        let h = self.core.head;
        let kind = self.content.blocks[h.block].kind.clone();
        if kind.is_void() {
            self.checkpoint(EditKind::Other);
            self.remove_blocks(h.block..h.block + 1);
            self.core.head = self.clamp(Pos::new(h.block, 0, 0));
            self.core.anchor = self.core.head;
            self.changed(cx);
            return;
        }
        let len = self.slot_len(h.slot());
        if h.offset < len {
            self.checkpoint(EditKind::Delete);
            let t = self.slot_text(h.slot()).unwrap();
            let next = t.text[h.offset..].grapheme_indices(true).nth(1).map(|(i, _)| h.offset + i).unwrap_or(len);
            self.slot_mut(h.slot()).delete(h.offset..next);
            self.changed(cx);
            return;
        }
        if kind.is_multi() {
            if h.cell + 1 < self.stops(h.block) {
                self.set_cursor(Pos::new(h.block, h.cell + 1, 0), false, cx);
            }
            return;
        }
        let next = h.block + 1;
        if next >= self.content.blocks.len() {
            return;
        }
        let nk = self.content.blocks[next].kind.clone();
        self.checkpoint(EditKind::Other);
        if nk.is_textual() {
            let mut t = self.content.blocks[next].text.clone();
            if h.block == 0 {
                t = RichText::plain(t.text.replace('\n', " "));
            }
            self.block_mut(h.block).text.append(&t);
            self.remove_blocks(next..next + 1);
        } else if nk.is_void() {
            self.remove_blocks(next..next + 1);
        }
        self.changed(cx);
    }

    pub fn delete_word_back(&mut self, _: &DeleteWordBack, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_selection() {
            let p = self.word_left_pos();
            if p.slot() == self.core.head.slot() {
                self.core.anchor = p;
            }
        }
        self.backspace(&Backspace, window, cx);
    }

    pub fn delete_line_back(&mut self, _: &DeleteLineBack, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_selection() {
            let p = self.line_edge(false);
            self.core.anchor = p;
        }
        self.backspace(&Backspace, window, cx);
    }

    // ───────────── 回车 / Tab ─────────────

    pub fn enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if self.sess.image_crop.is_some() {
            self.crop_commit(cx);
            return;
        }
        if let Some(sm) = &self.pop.slash {
            let ix = sm.selected;
            self.apply_slash(ix, window, cx);
            return;
        }
        if let Some(m) = &self.pop.mention {
            let ix = m.selected;
            self.apply_mention(ix, cx);
            return;
        }
        self.checkpoint(EditKind::Other);
        if self.has_selection() {
            self.delete_selection_inner();
        }
        let h = self.core.head;
        let blk = self.content.blocks[h.block].clone();
        let insert_after = |this: &mut Self, b: Block| {
            this.insert_blocks(h.block + 1, vec![b]);
            this.core.head = Pos::new(h.block + 1, 0, 0);
        };
        match &blk.kind {
            k if k.is_void() => insert_after(self, Block::text("")),
            BlockKind::Title => {
                let tail = self.block_mut(0).text.split_off(h.offset);
                let next_empty = self.content.blocks.get(1).is_some_and(|b| b.kind == BlockKind::Text && b.text.is_empty());
                if next_empty {
                    // 新文档：标题下方已经有一个空段落，直接进入它
                    self.block_mut(1).text = RichText::plain(tail.text);
                } else {
                    self.insert_blocks(1, vec![Block::new(BlockKind::Text, tail)]);
                }
                self.core.head = Pos::new(1, 0, 0);
            }
            BlockKind::Timeline { hide, .. } => {
                // 回车跳到同节点下一个可见字段；已是最后一个字段时在其后新增节点
                use crate::model::TL_SLOTS;
                let (k, f) = (h.cell / TL_SLOTS, h.cell % TL_SLOTS);
                match (f + 1..TL_SLOTS).find(|i| hide & (1 << i) == 0) {
                    Some(nf) => self.core.head = Pos::new(h.block, k * TL_SLOTS + nf, 0),
                    None => {
                        if let BlockKind::Timeline { items, .. } = &mut self.block_mut(h.block).kind {
                            items.insert(k + 1, crate::model::TimelineItem::default());
                        }
                        let first = (0..TL_SLOTS).find(|i| hide & (1 << i) == 0).unwrap_or(0);
                        self.core.head = Pos::new(h.block, (k + 1) * TL_SLOTS + first, 0);
                    }
                }
            }
            BlockKind::Code { .. } | BlockKind::Table { .. } | BlockKind::Columns { .. } => {
                // 代码块自动继承上一行缩进
                let mut s = "\n".to_string();
                if matches!(blk.kind, BlockKind::Code { .. }) {
                    let t = &blk.text.text[..h.offset];
                    let line = t.rsplit('\n').next().unwrap_or("");
                    s.push_str(&line[..line.len() - line.trim_start().len()]);
                }
                let m = self.current_marks();
                self.slot_mut(h.slot()).insert(h.offset, &s, m);
                self.core.head.offset += s.len();
            }
            BlockKind::Text if matches!(blk.text.text.as_str(), "---" | "***" | "___" | "——") => {
                let b = self.block_mut(h.block);
                b.kind = BlockKind::Divider;
                b.text = RichText::default();
                insert_after(self, Block::text(""));
            }
            BlockKind::Text if blk.text.text.starts_with("```") && h.offset == blk.text.len() => {
                let lang = normalize_lang(&blk.text.text[3..]);
                let b = self.block_mut(h.block);
                b.kind = BlockKind::Code { lang };
                b.text = RichText::default();
                self.core.head = Pos::new(h.block, 0, 0);
            }
            BlockKind::Quote | BlockKind::Callout { .. } | BlockKind::Notes => {
                let t = &blk.text;
                if t.is_empty() {
                    self.block_mut(h.block).kind = BlockKind::Text;
                } else if h.offset == t.len() && t.text.ends_with('\n') {
                    // 末尾空行再回车：跳出块
                    let len = t.len();
                    self.block_mut(h.block).text.delete(len - 1..len);
                    insert_after(self, Block::text(""));
                } else {
                    let m = self.current_marks();
                    self.slot_mut(h.slot()).insert(h.offset, "\n", m);
                    self.core.head.offset += 1;
                }
            }
            k if k.is_list() && blk.text.is_empty() => {
                let b = self.block_mut(h.block);
                if b.indent > 0 {
                    b.indent -= 1;
                } else {
                    b.kind = BlockKind::Text;
                }
            }
            BlockKind::Heading { .. } if h.offset == 0 && !blk.text.is_empty() => {
                self.insert_blocks(h.block, vec![Block::text("")]);
                self.core.head = Pos::new(h.block + 1, 0, 0);
            }
            // 行首回车：在上方插一个同类型的空块，原块（连同它的评论、同步关系）整体下移，不被拆成新块
            k if h.offset == 0 && !blk.text.is_empty() && (*k == BlockKind::Text || k.is_list()) => {
                let kind = match k {
                    BlockKind::Todo { .. } => BlockKind::Todo { checked: false },
                    k => k.clone(),
                };
                let mut nb = Block::new(kind, RichText::default());
                nb.indent = blk.indent;
                // 指定编号属于「列表在这里的位置」：交给插在上方的新项，原项接着往下数
                if blk.num.is_some() {
                    nb.num = blk.num;
                    self.block_mut(h.block).num = None;
                }
                self.insert_blocks(h.block, vec![nb]);
                self.core.head = Pos::new(h.block + 1, 0, 0);
            }
            _ => {
                let tail = self.block_mut(h.block).text.split_off(h.offset);
                let kind = match &blk.kind {
                    BlockKind::Todo { .. } => BlockKind::Todo { checked: false },
                    k if k.is_list() => k.clone(),
                    _ => BlockKind::Text,
                };
                let mut nb = Block::new(kind, tail);
                nb.indent = blk.indent;
                if blk.collapsed && !matches!(blk.kind, BlockKind::Heading { .. }) {
                    // 收起的项：新的同级项接在整棵子树后面（Workflowy 的行为）
                    let at = super::fold::subtree(&self.content.blocks, h.block).end;
                    self.insert_blocks(at, vec![nb]);
                    self.core.head = Pos::new(at, 0, 0);
                } else {
                    insert_after(self, nb);
                }
            }
        }
        self.core.anchor = self.core.head;
        self.changed(cx);
        self.reveal_head();
    }

    pub fn soft_enter(&mut self, _: &SoftEnter, _: &mut Window, cx: &mut Context<Self>) {
        if self.core.head.block == 0 || self.content.blocks[self.core.head.block].kind.is_void() {
            return;
        }
        self.insert_text("\n", cx);
    }

    pub fn tab(&mut self, _: &Tab, _: &mut Window, cx: &mut Context<Self>) {
        let h = self.core.head;
        match self.content.blocks[h.block].kind.clone() {
            BlockKind::Table { rows, cols, .. } => {
                // 后面的格子可能全被合并覆盖：没有可见格时同样在末尾加一行
                let mut next = self.next_cell_visible(h.block, h.cell + 1);
                if next >= rows * cols {
                    self.checkpoint(EditKind::Other);
                    self.table_insert_row(h.block, rows, cx);
                    next = rows * cols;
                }
                let p = Pos::new(h.block, next, 0);
                let len = self.slot_len(p.slot());
                self.core.anchor = p;
                self.set_cursor(Pos { offset: len, ..p }, true, cx);
            }
            BlockKind::Columns { cols } => {
                if h.cell + 1 < cols.len() {
                    self.set_cursor(Pos::new(h.block, h.cell + 1, 0), false, cx);
                }
            }
            BlockKind::Code { .. } => self.insert_text("    ", cx),
            _ => self.shift_indent(1, cx),
        }
    }

    pub fn shift_tab(&mut self, _: &ShiftTab, _: &mut Window, cx: &mut Context<Self>) {
        let h = self.core.head;
        match self.content.blocks[h.block].kind {
            BlockKind::Table { .. } | BlockKind::Columns { .. } => {
                if h.cell > 0 {
                    let mut prev = h.cell - 1;
                    while prev > 0 && self.cell_covered(h.block, prev) {
                        prev -= 1;
                    }
                    self.set_cursor(Pos::new(h.block, prev, 0), false, cx);
                }
            }
            BlockKind::Code { .. } => {
                let t = &self.content.blocks[h.block].text.text;
                let line_start = t[..h.offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
                let n = t[line_start..].chars().take(4).take_while(|c| *c == ' ').count();
                if n > 0 {
                    self.checkpoint(EditKind::Other);
                    self.block_mut(h.block).text.delete(line_start..line_start + n);
                    self.core.head.offset = self.core.head.offset.saturating_sub(n).max(line_start);
                    self.core.anchor = self.core.head;
                    self.changed(cx);
                }
            }
            _ => self.shift_indent(-1, cx),
        }
    }

    pub(super) fn shift_indent(&mut self, d: i32, cx: &mut Context<Self>) {
        let (s, e) = self.sel();
        self.checkpoint(EditKind::Other);
        let mut changed = false;
        for b in s.block.max(1)..=e.block {
            let blk = &self.content.blocks[b];
            if blk.kind.is_multi() || matches!(blk.kind, BlockKind::Heading { .. }) {
                continue;
            }
            let ni = (blk.indent as i32 + d).clamp(0, MAX_INDENT as i32) as u8;
            if ni != blk.indent {
                self.block_mut(b).indent = ni;
                changed = true;
            }
        }
        if changed {
            self.changed(cx);
        }
    }

    // ───────────── 格式 ─────────────

    fn toggle_mark(&mut self, get: fn(&Marks) -> bool, set: fn(&mut Marks, bool), cx: &mut Context<Self>) {
        if !self.has_selection() {
            let mut m = self.current_marks();
            let v = !get(&m);
            set(&mut m, v);
            self.core.pending_marks = Some(m);
            cx.notify();
            return;
        }
        let slots = self.selected_slots();
        if slots.is_empty() {
            return;
        }
        self.checkpoint(EditKind::Other);
        let all = slots.iter().all(|(s, r)| self.slot_text(*s).is_some_and(|t| t.all_marks(r.clone(), get)));
        for (s, r) in slots {
            self.slot_mut(s).update_marks(r, |m| set(m, !all));
        }
        self.changed(cx);
    }

    pub fn bold(&mut self, _: &Bold, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_mark(|m| m.bold, |m, v| m.bold = v, cx)
    }
    pub fn italic(&mut self, _: &Italic, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_mark(|m| m.italic, |m, v| m.italic = v, cx)
    }
    pub fn underline(&mut self, _: &Underline, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_mark(|m| m.underline, |m, v| m.underline = v, cx)
    }
    pub fn strike(&mut self, _: &Strike, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_mark(|m| m.strike, |m, v| m.strike = v, cx)
    }
    pub fn inline_code(&mut self, _: &InlineCode, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_mark(|m| m.code, |m, v| m.code = v, cx)
    }

    pub fn set_color(&mut self, fg: Option<u8>, bg: Option<u8>, cx: &mut Context<Self>) {
        let slots = self.selected_slots();
        if slots.is_empty() {
            return;
        }
        self.checkpoint(EditKind::Other);
        for (s, r) in slots {
            self.slot_mut(s).update_marks(r, |m| {
                if let Some(c) = fg {
                    m.color = c;
                }
                if let Some(c) = bg {
                    m.bg = c;
                }
            });
        }
        self.changed(cx);
    }

    pub fn clear_format(&mut self, cx: &mut Context<Self>) {
        let slots = self.selected_slots();
        self.checkpoint(EditKind::Other);
        for (s, r) in slots {
            self.slot_mut(s).update_marks(r, |m| *m = Marks::default());
        }
        self.changed(cx);
    }

    /// 选区涉及的表格单元格 / 分栏是否存在（决定浮动工具栏是否显示对齐按钮）。
    pub fn sel_in_cells(&self) -> bool {
        self.selected_slots().iter().any(|((b, _), _)| matches!(self.content.blocks[*b].kind, BlockKind::Table { .. } | BlockKind::Columns { .. }))
    }

    /// 选区内某个样式取值是否一致；一致返回该值（颜色面板高亮当前色用）。
    pub fn sel_uniform(&self, f: impl Fn(&Marks) -> u8) -> Option<u8> {
        let mut out = None;
        for (s, r) in self.selected_slots() {
            let Some(t) = self.slot_text(s) else { continue };
            for (run, m) in t.runs() {
                if run.end > r.start && run.start < r.end {
                    let v = f(m);
                    if out.is_some_and(|o| o != v) {
                        return None;
                    }
                    out = Some(v);
                }
            }
        }
        out
    }

    /// 选区涉及的文本块（不含表格 / 分栏）。
    fn sel_text_blocks(&self) -> Vec<usize> {
        let (s, e) = self.sel();
        (s.block.max(1)..=e.block.min(self.content.blocks.len() - 1)).filter(|b| self.content.blocks[*b].kind.is_textual()).collect()
    }

    /// 当前对齐（单元格看格子自身，普通块看块对齐）；不一致时为 None。
    pub fn sel_align(&self) -> Option<u8> {
        let vals: Vec<u8> = if self.sel_in_cells() {
            self.selected_slots().iter().filter_map(|(s, _)| self.slot_text(*s)).map(|t| t.align).collect()
        } else {
            self.sel_text_blocks().iter().map(|b| self.content.blocks[*b].align).collect()
        };
        vals.first().copied().filter(|f| vals.iter().all(|v| v == f))
    }

    /// 表格单元格的垂直对齐；不一致或不在表格内时为 None。
    pub fn sel_valign(&self) -> Option<u8> {
        let vals: Vec<u8> = self
            .selected_slots()
            .iter()
            .filter(|((b, _), _)| matches!(self.content.blocks[*b].kind, BlockKind::Table { .. }))
            .filter_map(|(s, _)| self.slot_text(*s))
            .map(|t| t.valign)
            .collect();
        vals.first().copied().filter(|f| vals.iter().all(|v| v == f))
    }

    /// 设置文字对齐：0 左、1 居中、2 右。选区在表格 / 分栏内时按格设置，否则设置所在块。
    pub fn set_sel_align(&mut self, align: u8, cx: &mut Context<Self>) {
        let align = align.min(2);
        if self.sel_in_cells() {
            let slots: Vec<_> = self
                .selected_slots()
                .into_iter()
                .map(|(s, _)| s)
                .filter(|(b, _)| matches!(self.content.blocks[*b].kind, BlockKind::Table { .. } | BlockKind::Columns { .. }))
                .collect();
            self.checkpoint(EditKind::Other);
            for s in slots {
                self.slot_mut(s).align = align;
            }
        } else {
            let blocks = self.sel_text_blocks();
            if blocks.is_empty() {
                return;
            }
            self.checkpoint(EditKind::Other);
            for b in blocks {
                self.block_mut(b).align = align;
            }
        }
        self.changed(cx);
    }

    /// 设置表格单元格的垂直对齐：0 顶部、1 居中、2 底部。
    pub fn set_cell_valign(&mut self, valign: u8, cx: &mut Context<Self>) {
        let slots: Vec<_> =
            self.selected_slots().into_iter().map(|(s, _)| s).filter(|(b, _)| matches!(self.content.blocks[*b].kind, BlockKind::Table { .. })).collect();
        if slots.is_empty() {
            return;
        }
        self.checkpoint(EditKind::Other);
        for s in slots {
            self.slot_mut(s).valign = valign.min(2);
        }
        self.changed(cx);
    }

    /// 选区所在文本块的类型（多个块类型不同时为 None）。
    pub fn sel_kind(&self) -> Option<BlockKind> {
        let blocks = self.sel_text_blocks();
        let first = self.content.blocks.get(*blocks.first()?)?.kind.clone();
        blocks.iter().all(|b| self.content.blocks[*b].kind == first).then_some(first)
    }

    /// 浮动工具栏「文本类型」：已是该类型时不做切换（set_kind 同类型会切回正文）。
    pub fn set_sel_kind(&mut self, kind: BlockKind, cx: &mut Context<Self>) {
        if self.sel_kind().as_ref() == Some(&kind) {
            return;
        }
        self.set_kind(kind, cx);
    }

    /// 把选中的块（或光标所在块）转换为指定类型。
    pub fn set_kind(&mut self, kind: BlockKind, cx: &mut Context<Self>) {
        let (s, e) = self.sel();
        self.checkpoint(EditKind::Other);
        for b in s.block.max(1)..=e.block {
            let cur = &self.content.blocks[b].kind;
            if !cur.is_textual() {
                continue;
            }
            let toggled_off = std::mem::discriminant(cur) == std::mem::discriminant(&kind) && !matches!(kind, BlockKind::Text) && *cur == kind;
            let blk = self.block_mut(b);
            blk.kind = if toggled_off { BlockKind::Text } else { kind.clone() };
            if matches!(blk.kind, BlockKind::Heading { .. }) {
                blk.indent = 0;
            }
        }
        self.core.head = self.clamp(self.core.head);
        self.core.anchor = self.clamp(self.core.anchor);
        self.changed(cx);
    }

    pub fn set_text_kind(&mut self, _: &SetText, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind(BlockKind::Text, cx)
    }
    pub fn set_h1(&mut self, _: &SetH1, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind(BlockKind::Heading { level: 1 }, cx)
    }
    pub fn set_h2(&mut self, _: &SetH2, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind(BlockKind::Heading { level: 2 }, cx)
    }
    pub fn set_h3(&mut self, _: &SetH3, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind(BlockKind::Heading { level: 3 }, cx)
    }
    pub fn set_h4(&mut self, _: &SetH4, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind(BlockKind::Heading { level: 4 }, cx)
    }
    pub fn set_bullet(&mut self, _: &SetBullet, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind(BlockKind::Bullet, cx)
    }
    pub fn set_ordered(&mut self, _: &SetOrdered, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind(BlockKind::Ordered, cx)
    }
    pub fn set_todo(&mut self, _: &SetTodo, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind(BlockKind::Todo { checked: false }, cx)
    }
    pub fn set_quote(&mut self, _: &SetQuote, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind(BlockKind::Quote, cx)
    }
    pub fn set_code(&mut self, _: &SetCode, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind(BlockKind::Code { lang: "Plain Text".into() }, cx)
    }

    pub fn toggle_todo_at(&mut self, b: usize, cx: &mut Context<Self>) {
        if let BlockKind::Todo { checked } = self.content.blocks[b].kind {
            self.checkpoint(EditKind::Other);
            self.block_mut(b).kind = BlockKind::Todo { checked: !checked };
            self.changed(cx);
        }
    }

    pub fn toggle_todo(&mut self, _: &ToggleTodo, _: &mut Window, cx: &mut Context<Self>) {
        let b = self.core.head.block;
        if matches!(self.content.blocks[b].kind, BlockKind::Todo { .. }) {
            self.toggle_todo_at(b, cx);
        } else {
            self.set_kind(BlockKind::Todo { checked: false }, cx);
        }
    }

    // ───────────── 块操作 ─────────────

    pub fn duplicate_block_at(&mut self, b: usize, cx: &mut Context<Self>) {
        if b == 0 {
            return;
        }
        self.checkpoint(EditKind::Other);
        let mut copy = (*self.content.blocks[b]).clone();
        copy.id = crate::model::next_id();
        copy.sync = None;
        // 副本紧跟在原块后面：接着往下编号，而不是重复同一个指定编号
        copy.num = None;
        if let BlockKind::Embed { id, .. } = &mut copy.kind {
            if let Some(n) = crate::objects::duplicate(id) {
                *id = n;
            }
        }
        self.insert_blocks(b + 1, vec![copy]);
        self.core.head = Pos::new(b + 1, 0, 0);
        self.core.anchor = self.core.head;
        self.pop.block_menu = None;
        self.changed(cx);
    }

    pub fn duplicate_block(&mut self, _: &DuplicateBlock, _: &mut Window, cx: &mut Context<Self>) {
        self.duplicate_block_at(self.core.head.block, cx)
    }

    pub fn delete_block_at(&mut self, b: usize, cx: &mut Context<Self>) {
        if b == 0 {
            return;
        }
        self.checkpoint(EditKind::Other);
        self.remove_blocks(b..b + 1);
        self.core.head = self.clamp(Pos::new(b.min(self.content.blocks.len() - 1), 0, 0));
        self.core.anchor = self.core.head;
        self.pop.block_menu = None;
        self.changed(cx);
    }

    /// 把块 from 移动到 to 之前（拖拽排序）。
    pub fn move_block_to(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if from == 0 || to == 0 || from == to || from + 1 == to {
            return;
        }
        self.checkpoint(EditKind::Other);
        let blk = self.content.blocks.remove(from);
        self.list.splice(from..from + 1, 0);
        let dest = if from < to { to - 1 } else { to };
        self.content.blocks.insert(dest, blk);
        self.list.splice(dest..dest, 1);
        self.core.head = Pos::new(dest, 0, 0);
        self.core.anchor = self.core.head;
        self.changed(cx);
    }

    pub(crate) fn move_block(&mut self, d: i32, cx: &mut Context<Self>) {
        let (s, e) = self.sel();
        let (s, e) = (s.block.max(1), e.block);
        // 选区只在标题里（e == 0）时没有可移动的块；否则 e - s 会下溢
        if e < s || (d < 0 && s <= 1) || (d > 0 && e + 1 >= self.content.blocks.len()) {
            return;
        }
        self.checkpoint(EditKind::Other);
        if d < 0 {
            let blk = self.content.blocks.remove(s - 1);
            self.content.blocks.insert(e, blk);
            self.list.splice(s - 1..e + 1, e - s + 2);
        } else {
            let blk = self.content.blocks.remove(e + 1);
            self.content.blocks.insert(s, blk);
            self.list.splice(s..e + 2, e - s + 2);
        }
        // 标题不随之移动：只平移落在被移动区间里的端点
        for p in [&mut self.core.head, &mut self.core.anchor] {
            if p.block >= s {
                p.block = (p.block as i32 + d) as usize;
            }
        }
        self.changed(cx);
        self.reveal_head();
    }

    pub fn move_block_up(&mut self, _: &MoveBlockUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_block(-1, cx)
    }
    pub fn move_block_down(&mut self, _: &MoveBlockDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_block(1, cx)
    }

    pub fn set_block_kind_at(&mut self, b: usize, kind: BlockKind, cx: &mut Context<Self>) {
        self.core.head = Pos::new(b, 0, 0);
        self.core.anchor = self.core.head;
        self.pop.block_menu = None;
        self.set_kind(kind, cx);
    }

    pub fn set_code_lang(&mut self, b: usize, lang: &str, cx: &mut Context<Self>) {
        self.checkpoint(EditKind::Other);
        if let BlockKind::Code { lang: l } = &mut self.block_mut(b).kind {
            *l = lang.to_string();
        }
        self.pop.lang_menu = None;
        self.changed(cx);
    }

    pub fn set_callout(&mut self, b: usize, emoji: Option<&str>, color: Option<u8>, cx: &mut Context<Self>) {
        self.checkpoint(EditKind::Other);
        if let BlockKind::Callout { emoji: e, color: c } = &mut self.block_mut(b).kind {
            if let Some(x) = emoji {
                *e = x.to_string();
            }
            if let Some(x) = color {
                *c = x;
            }
        }
        self.changed(cx);
    }

    // ───────────── 表格 / 分栏 ─────────────

    /// 从 from 起找到下一个未被合并覆盖的单元格（不含终点越界）。
    fn next_cell_visible(&self, b: usize, from: usize) -> usize {
        if let BlockKind::Table { rows, cols, merges, .. } = &self.content.blocks[b].kind {
            let cols = (*cols).max(1);
            let total = rows * cols;
            let mut i = from;
            while i < total {
                let (r, c) = (i / cols, i % cols);
                match crate::model::merge_at(merges, r, c) {
                    Some(m) if (m.row, m.col) != (r, c) => i += 1,
                    _ => return i,
                }
            }
            total
        } else {
            from
        }
    }

    /// 该单元格是否被合并覆盖（不是合并区左上角）。
    fn cell_covered(&self, b: usize, cell: usize) -> bool {
        if let BlockKind::Table { cols, merges, .. } = &self.content.blocks[b].kind {
            let cols = (*cols).max(1);
            let (r, c) = (cell / cols, cell % cols);
            matches!(crate::model::merge_at(merges, r, c), Some(m) if (m.row, m.col) != (r, c))
        } else {
            false
        }
    }

    pub fn table_insert_row(&mut self, b: usize, at: usize, cx: &mut Context<Self>) {
        if let BlockKind::Table { rows, cols, cells, merges, .. } = &mut self.block_mut(b).kind {
            let at = at.min(*rows);
            let c = *cols;
            cells.splice(at * c..at * c, std::iter::repeat_n(RichText::default(), c));
            *rows += 1;
            for m in merges.iter_mut() {
                if m.row >= at {
                    m.row += 1;
                } else if m.row + m.row_span > at {
                    // 合并区跨过插入点：跟着长一行
                    m.row_span += 1;
                }
            }
        }
        self.changed(cx);
    }

    pub fn table_insert_col(&mut self, b: usize, at: usize, cx: &mut Context<Self>) {
        if let BlockKind::Table { rows, cols, cells, widths, merges, .. } = &mut self.block_mut(b).kind {
            let at = at.min(*cols);
            for r in (0..*rows).rev() {
                cells.insert(r * *cols + at, RichText::default());
            }
            *cols += 1;
            if let Some(w) = widths {
                // 新列沿用相邻列的宽度比例
                let v = w.get(at.saturating_sub(1)).copied().unwrap_or(1.);
                w.insert(at.min(w.len()), v);
            }
            for m in merges.iter_mut() {
                if m.col >= at {
                    m.col += 1;
                } else if m.col + m.col_span > at {
                    m.col_span += 1;
                }
            }
        }
        self.changed(cx);
    }

    pub fn table_delete_row(&mut self, b: usize, row: usize, cx: &mut Context<Self>) {
        let mut remove_all = false;
        if let BlockKind::Table { rows, cols, cells, merges, .. } = &mut self.block_mut(b).kind {
            if *rows <= 1 {
                remove_all = true;
            } else {
                // 删掉的是跨行合并区的首行：内容挪到下一行同列，它将成为新的左上角
                for m in merges.iter().filter(|m| m.row == row && m.row_span > 1) {
                    let (from, to) = (row * *cols + m.col, (row + 1) * *cols + m.col);
                    cells[to] = std::mem::take(&mut cells[from]);
                }
                cells.drain(row * *cols..(row + 1) * *cols);
                *rows -= 1;
                merges.retain_mut(|m| {
                    if m.row_span == 1 && m.row == row {
                        return false;
                    }
                    if m.row > row {
                        m.row -= 1;
                    } else if m.row + m.row_span > row {
                        m.row_span -= 1;
                    }
                    true
                });
                merges.retain(|m| m.row_span > 1 || m.col_span > 1);
            }
        }
        if remove_all {
            self.remove_blocks(b..b + 1);
        }
        self.core.head = self.clamp(Pos::new(b, self.core.head.cell, 0));
        self.core.anchor = self.core.head;
        self.changed(cx);
    }

    pub fn table_delete_col(&mut self, b: usize, col: usize, cx: &mut Context<Self>) {
        let mut remove_all = false;
        if let BlockKind::Table { rows, cols, cells, widths, merges, .. } = &mut self.block_mut(b).kind {
            if *cols <= 1 {
                remove_all = true;
            } else {
                // 删掉的是跨列合并区的首列：内容挪到右侧一列
                for m in merges.iter().filter(|m| m.col == col && m.col_span > 1) {
                    let (from, to) = (m.row * *cols + col, m.row * *cols + col + 1);
                    cells[to] = std::mem::take(&mut cells[from]);
                }
                for r in (0..*rows).rev() {
                    cells.remove(r * *cols + col);
                }
                *cols -= 1;
                if let Some(w) = widths {
                    if col < w.len() {
                        w.remove(col);
                    }
                }
                merges.retain_mut(|m| {
                    if m.col_span == 1 && m.col == col {
                        return false;
                    }
                    if m.col > col {
                        m.col -= 1;
                    } else if m.col + m.col_span > col {
                        m.col_span -= 1;
                    }
                    true
                });
                merges.retain(|m| m.row_span > 1 || m.col_span > 1);
            }
        }
        if remove_all {
            self.remove_blocks(b..b + 1);
        }
        self.core.head = self.clamp(Pos::new(b, 0, 0));
        self.core.anchor = self.core.head;
        self.changed(cx);
    }

    /// 鼠标移到斜杠菜单的「表格」项上：右侧弹出行列选择器。
    pub fn show_slash_table_picker(&mut self, mouse: Point<Pixels>, cx: &mut Context<Self>) {
        // 抵消离开时排的延迟关闭
        self.table_hover_epoch = self.table_hover_epoch.wrapping_add(1);
        let Some(sm) = &self.pop.slash else { return };
        let block = sm.block;
        if self.pop.table_picker.as_ref().is_some_and(|tp| tp.block == block) {
            return;
        }
        // 让浮层顶边对齐到鼠标所在行，鼠标水平右移即可进入浮层
        self.pop.table_picker = Some(TablePicker { block, rows: 3, cols: 3, y: mouse.y - px(18.) });
        cx.notify();
    }

    /// 鼠标移出菜单项 / 浮层：稍后再关，给「菜单 → 浮层」的移动留出时间。
    pub fn schedule_close_table_picker(&mut self, cx: &mut Context<Self>) {
        if !self.pop.table_picker.is_some() {
            return;
        }
        self.table_hover_epoch = self.table_hover_epoch.wrapping_add(1);
        let epoch = self.table_hover_epoch;
        cx.spawn(async move |this: WeakEntity<Editor>, cx: &mut AsyncApp| {
            cx.background_executor().timer(std::time::Duration::from_millis(200)).await;
            this.update(cx, |this, cx| {
                if this.table_hover_epoch != epoch {
                    return;
                }
                if this.pop.table_picker.take().is_some() {
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// 鼠标进入浮层：取消待执行的关闭。
    pub fn keep_table_picker(&mut self) {
        self.table_hover_epoch = self.table_hover_epoch.wrapping_add(1);
    }

    /// 取消行列选择器（Esc / 点外部）。
    pub fn cancel_table_picker(&mut self, cx: &mut Context<Self>) {
        self.table_hover_epoch = self.table_hover_epoch.wrapping_add(1);
        if self.pop.table_picker.take().is_some() {
            cx.notify();
        }
    }

    /// 在行列选择器里点选 N×M：删掉斜杠触发文本，插入表格。
    pub fn insert_table_from_slash(&mut self, rows: usize, cols: usize, cx: &mut Context<Self>) {
        let Some(sm) = self.pop.slash.take() else { return };
        self.pop.table_picker = None;
        self.table_hover_epoch = self.table_hover_epoch.wrapping_add(1);
        let bi = sm.block;
        if bi >= self.content.blocks.len() {
            return;
        }
        self.checkpoint(EditKind::Other);
        let end = self.core.head.offset.max(sm.start);
        let end = end.min(self.content.blocks[bi].text.len());
        self.block_mut(bi).text.delete(sm.start..end);
        self.core.head = Pos::new(bi, 0, sm.start);
        self.core.anchor = self.core.head;
        let current_empty = self.content.blocks[bi].text.is_empty() && self.content.blocks[bi].kind == BlockKind::Text;
        let at = if current_empty {
            let indent = self.content.blocks[bi].indent;
            let sync = self.content.blocks[bi].sync.clone();
            let b = self.block_mut(bi);
            *b = Block::table(rows, cols);
            b.indent = indent;
            b.sync = sync;
            bi
        } else {
            self.insert_blocks(bi + 1, vec![Block::table(rows, cols)]);
            bi + 1
        };
        if at + 1 >= self.content.blocks.len() {
            self.insert_blocks(at + 1, vec![Block::text("")]);
        }
        self.core.head = Pos::new(at, 0, 0);
        self.core.anchor = self.core.head;
        self.changed(cx);
        self.reveal_head();
    }

    /// 合并选区覆盖的单元格（保留左上角内容，其余清空）。
    pub fn table_merge_selection(&mut self, b: usize, cx: &mut Context<Self>) {
        let (s, e) = self.sel();
        if s.block != b || e.block != b || s.cell == e.cell {
            cx.emit(EditorEvent::Toast("先选中多个单元格再合并".into()));
            return;
        }
        self.checkpoint(EditKind::Other);
        let mut landed: Option<(usize, usize)> = None;
        if let BlockKind::Table { cols, cells, merges, .. } = &mut self.block_mut(b).kind {
            let cols = *cols;
            let (r1, c1, r2, c2) = crate::model::table_rect(cols, merges, s.cell, e.cell);
            if r2 > r1 || c2 > c1 {
                // 被合并掉的格子里有内容时，按通用做法拼到左上角（换行分隔），不丢字
                let origin = r1 * cols + c1;
                let mut joined = cells[origin].clone();
                for r in r1..=r2 {
                    for c in c1..=c2 {
                        let i = r * cols + c;
                        if i != origin && !cells[i].is_empty() {
                            if !joined.is_empty() {
                                joined.insert(joined.len(), "\n", Marks::default());
                            }
                            joined.append(&cells[i]);
                        }
                    }
                }
                cells[origin] = joined;
                // 与新区间相交的旧合并先拆掉
                merges.retain(|m| !(m.row <= r2 && m.row + m.row_span > r1 && m.col <= c2 && m.col + m.col_span > c1));
                merges.push(CellMerge { row: r1, col: c1, row_span: r2 - r1 + 1, col_span: c2 - c1 + 1 });
                let origin = r1 * cols + c1;
                for r in r1..=r2 {
                    for c in c1..=c2 {
                        let i = r * cols + c;
                        if i != origin {
                            cells[i] = RichText::default();
                        }
                    }
                }
                let len = cells[origin].len();
                landed = Some((origin, len));
            }
        }
        match landed {
            Some((origin, len)) => {
                self.core.head = Pos::new(b, origin, len);
                self.core.anchor = self.core.head;
                self.changed(cx);
            }
            None => cx.emit(EditorEvent::Toast("先选中多个单元格再合并".into())),
        }
    }

    /// 拆分光标所在的合并单元格。
    pub fn table_split_cell(&mut self, b: usize, cx: &mut Context<Self>) {
        let cell = self.core.head.cell;
        self.checkpoint(EditKind::Other);
        if let BlockKind::Table { cols, merges, .. } = &mut self.block_mut(b).kind {
            let (r, c) = (cell / *cols, cell % *cols);
            merges.retain(|m| !(r >= m.row && r < m.row + m.row_span && c >= m.col && c < m.col + m.col_span));
        }
        self.changed(cx);
    }

    /// 切换「标题行」：首行加粗 + 底色。
    pub fn table_toggle_header_row(&mut self, b: usize, cx: &mut Context<Self>) {
        self.checkpoint(EditKind::Other);
        if let BlockKind::Table { header_row, .. } = &mut self.block_mut(b).kind {
            *header_row = !*header_row;
        }
        self.changed(cx);
    }

    /// 切换「标题列」：首列加粗 + 底色。
    pub fn table_toggle_header_col(&mut self, b: usize, cx: &mut Context<Self>) {
        self.checkpoint(EditKind::Other);
        if let BlockKind::Table { header_col, .. } = &mut self.block_mut(b).kind {
            *header_col = !*header_col;
        }
        self.changed(cx);
    }

    /// 均分列宽：清掉手动宽度，回到等分。
    pub fn table_even_widths(&mut self, b: usize, cx: &mut Context<Self>) {
        self.checkpoint(EditKind::Other);
        if let BlockKind::Table { widths, .. } = &mut self.block_mut(b).kind {
            *widths = None;
        }
        self.changed(cx);
    }

    /// 写入各列宽度比例（拖动列宽用）。
    pub fn table_set_col_widths(&mut self, b: usize, mut w: Vec<f32>, cx: &mut Context<Self>) {
        if let BlockKind::Table { cols, widths, .. } = &mut self.block_mut(b).kind {
            w.resize(*cols, 1.);
            let v: Vec<f32> = w.iter().map(|x| x.max(0.08)).collect();
            *widths = Some(v);
        }
        self.changed(cx);
    }

    pub fn table_op(&mut self, b: usize, op: &str, cx: &mut Context<Self>) {
        match op {
            "header_row" => return self.table_toggle_header_row(b, cx),
            "header_col" => return self.table_toggle_header_col(b, cx),
            "even_widths" => return self.table_even_widths(b, cx),
            "merge" => return self.table_merge_selection(b, cx),
            "split" => return self.table_split_cell(b, cx),
            _ => {}
        }
        let Some((r1, c1, r2, c2)) = self.table_target(b) else { return };
        self.checkpoint(EditKind::Other);
        match op {
            "row_above" => self.table_insert_row(b, r1, cx),
            "row_below" => self.table_insert_row(b, r2 + 1, cx),
            "col_left" => self.table_insert_col(b, c1, cx),
            "col_right" => self.table_insert_col(b, c2 + 1, cx),
            // 选中多行 / 多列时一起删（从后往前删，下标不受影响）
            "del_row" => {
                for r in (r1..=r2).rev() {
                    if self.content.blocks.get(b).is_some_and(|x| matches!(x.kind, BlockKind::Table { .. })) {
                        self.table_delete_row(b, r, cx);
                    }
                }
            }
            "del_col" => {
                for c in (c1..=c2).rev() {
                    if self.content.blocks.get(b).is_some_and(|x| matches!(x.kind, BlockKind::Table { .. })) {
                        self.table_delete_col(b, c, cx);
                    }
                }
            }
            "del_table" => self.delete_block_at(b, cx),
            _ => {}
        }
    }

    /// 工具栏操作的对象：多格选区；没有选区时是光标所在格（连同它的合并区）。
    pub fn table_target(&self, b: usize) -> Option<(usize, usize, usize, usize)> {
        if let Some(r) = self.table_sel_rect(b) {
            return Some(r);
        }
        let BlockKind::Table { cols, merges, .. } = &self.content.blocks.get(b)?.kind else { return None };
        let cols = (*cols).max(1);
        let cell = if self.core.head.block == b { self.core.head.cell } else { 0 };
        let (r, c, rs, cs) = crate::model::merge_origin(merges, cell / cols, cell % cols);
        Some((r, c, r + rs - 1, c + cs - 1))
    }

    /// 选中表格里的一个矩形区域（整行 / 整列 / 全表）。
    pub fn table_select(&mut self, b: usize, (r1, c1, r2, c2): (usize, usize, usize, usize), cx: &mut Context<Self>) {
        let BlockKind::Table { cols, .. } = self.content.blocks[b].kind else { return };
        let (a, z) = (r1 * cols + c1, r2 * cols + c2);
        self.core.anchor = Pos::new(b, a, 0);
        self.core.head = Pos::new(b, z, self.slot_len((b, z)));
        // 1×1 的「整行 / 整列」：退化成选中这一格的全部文字
        if a == z {
            self.core.anchor = Pos::new(b, a, 0);
        }
        self.pop.slash = None;
        self.break_coalescing();
        self.touch_activity();
        cx.notify();
    }

    /// 按下列边线：记下各列当前像素宽度，开始拖动。
    pub fn start_table_col_drag(&mut self, b: usize, col: usize, at: Point<Pixels>, cx: &mut Context<Self>) {
        let id = self.content.blocks[b].id;
        let Some(geo) = self.caches.table_geo.borrow().get(&id).cloned() else { return };
        let start: Vec<f32> = geo.cols.iter().map(|c| f32::from(c.size.width)).collect();
        if col + 1 >= start.len() {
            return;
        }
        self.sess.table_col_drag = Some(TableColDrag { block: b, col, start_x: at.x, cur: start.clone(), start });
        cx.notify();
    }

    /// 拖动中：只在相邻两列之间挪宽度，表格总宽不变。
    pub fn drag_table_col(&mut self, x: Pixels, cx: &mut Context<Self>) {
        const MIN_COL: f32 = 44.;
        let Some(d) = self.sess.table_col_drag.as_mut() else { return };
        let (l, r) = (d.start[d.col], d.start[d.col + 1]);
        let left = (l + f32::from(x - d.start_x)).clamp(MIN_COL.min(l), (l + r - MIN_COL).max(l.min(MIN_COL)));
        d.cur[d.col] = left;
        d.cur[d.col + 1] = l + r - left;
        cx.notify();
    }

    /// 松开列边线：宽度有变化才写入（并记一步撤销）。
    pub fn end_table_col_drag(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.sess.table_col_drag.take() else { return };
        if d.cur.iter().zip(&d.start).any(|(a, b)| (a - b).abs() >= 1.) {
            let total: f32 = d.cur.iter().sum::<f32>().max(1.);
            let n = d.cur.len() as f32;
            self.checkpoint(EditKind::Other);
            self.table_set_col_widths(d.block, d.cur.iter().map(|w| w / total * n).collect(), cx);
        } else {
            cx.notify();
        }
    }

    pub fn columns_op(&mut self, b: usize, add: bool, cx: &mut Context<Self>) {
        self.checkpoint(EditKind::Other);
        if let BlockKind::Columns { cols } = &mut self.block_mut(b).kind {
            if add && cols.len() < 5 {
                cols.push(RichText::default());
            } else if !add && cols.len() > 1 {
                let last = cols.pop().unwrap();
                if !last.is_empty() {
                    let prev = cols.last_mut().unwrap();
                    prev.insert(prev.len(), "\n", Marks::default());
                    prev.append(&last);
                }
            }
        }
        self.core.head = self.clamp(self.core.head);
        self.core.anchor = self.clamp(self.core.anchor);
        self.changed(cx);
    }

    // ───────────── 其他 ─────────────

    pub fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let slot = self.core.head.slot();
        let len = self.slot_len(slot);
        let (s, e) = self.sel();
        let whole_slot = s == Pos::new(slot.0, slot.1, 0) && e == Pos::new(slot.0, slot.1, len);
        if whole_slot || s.slot() != e.slot() || len == 0 {
            // 聚焦时只选聚焦的子树；终点落在最后一个可见块上（折叠的子项随父项一起被选中）
            let (first, end) = match self.focus_ix() {
                Some(r) => (r, super::fold::subtree(&self.content.blocks, r).end.max(r + 1)),
                None => (0, self.content.blocks.len()),
            };
            let last = self.visible_from(end - 1, false).filter(|l| *l >= first).unwrap_or(first);
            self.core.anchor = Pos::new(first, 0, 0);
            self.core.head = self.slot_end(last);
        } else {
            self.core.anchor = Pos::new(slot.0, slot.1, 0);
            self.core.head = Pos::new(slot.0, slot.1, len);
        }
        self.pop.slash = None;
        cx.notify();
    }

    pub fn escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.sess.image_preview.take().is_some() || self.sess.diagram_preview.take().is_some() || self.pop.diagram_theme_menu.take().is_some() {
            cx.notify();
            return;
        }
        if self.sess.image_crop.is_some() {
            self.crop_cancel(cx);
            return;
        }
        if self.pop.table_picker.is_some() {
            self.cancel_table_picker(cx);
            return;
        }
        if self.pop.countdown_edit.is_some() {
            self.countdown_cancel(window, cx);
            return;
        }
        if self.sess.file_full.take().is_some() {
            crate::webview::hide_live();
            cx.notify();
            return;
        }
        if self.sess.video_full.take().is_some()
            || self.pop.prop_edit.take().is_some()
            || self.pop.tl_menu.take().is_some()
            || self.pop.countdown_color.take().is_some()
        {
            cx.notify();
            return;
        }
        if self.sess.html_live.take().is_some() {
            crate::webview::hide_live();
            cx.notify();
            return;
        }
        if self.pop.slash.take().is_some()
            || self.pop.block_menu.take().is_some()
            || self.pop.lang_menu.take().is_some()
            || self.pop.ordered_menu.take().is_some()
            || self.pop.mention.take().is_some()
        {
            cx.notify();
        } else if self.pop.link_edit.is_some() {
            self.pop.link_edit = None;
            cx.notify();
        } else if self.find.is_some() {
            self.close_find(window, cx);
        } else if self.has_selection() {
            self.core.anchor = self.core.head;
            cx.notify();
        } else {
            self.exit_focus(cx);
        }
    }

    // ───────────── 链接 ─────────────

    pub fn insert_link(&mut self, _: &InsertLink, window: &mut Window, cx: &mut Context<Self>) {
        let (s, e) = self.sel();
        if s.slot() != e.slot() || s == e {
            cx.emit(EditorEvent::Toast("先选中一段文字，再添加链接".into()));
            return;
        }
        let existing = self
            .slot_text(s.slot())
            .and_then(|t| t.runs().find(|(r, _)| r.start <= s.offset && s.offset < r.end).and_then(|(_, m)| m.link.clone()))
            .unwrap_or_default();
        let input = cx.new(|cx| LineInput::new("粘贴或输入链接，回车确认", cx));
        input.update(cx, |i, cx| i.set_text(&existing, cx));
        let slot = s.slot();
        let range = s.offset..e.offset;
        let sub = cx.subscribe_in(&input, window, move |this, input, ev: &InputEvent, window, cx| match ev {
            InputEvent::Submit { .. } => {
                let url = input.read(cx).text().trim().to_string();
                this.apply_link(slot, range.clone(), &url, cx);
                this.pop.link_edit = None;
                window.focus(&this.focus);
            }
            InputEvent::Cancel => {
                this.pop.link_edit = None;
                window.focus(&this.focus);
                cx.notify();
            }
            _ => {}
        });
        window.focus(&input.focus_handle(cx));
        self.pop.link_edit = Some(LinkEdit { input, _sub: sub });
        cx.notify();
    }

    pub fn apply_link(&mut self, slot: (usize, usize), range: Range<usize>, url: &str, cx: &mut Context<Self>) {
        self.checkpoint(EditKind::Other);
        let url = url.to_string();
        let url = if url.is_empty() || url.contains("://") || url.starts_with("mailto:") { url } else { format!("https://{url}") };
        self.slot_mut(slot).update_marks(range, |m| m.link = (!url.is_empty()).then(|| url.clone()));
        self.changed(cx);
    }
}
