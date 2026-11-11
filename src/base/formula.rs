//! 公式引擎：`[字段名]` 引用字段，按记录逐行求值。
//!
//! 常用公式子集：
//! - 运算：`+ - * / ^`、`&`（拼接）、`= != <> > >= < <=`、`&& ||`
//! - 函数：数学、逻辑、文本、日期四大类约 50 个（见 `FUNCS`）
//! - 日期参与加减按天计算：`[截止日期] - TODAY()` 得到天数

use chrono::{Datelike, Local, NaiveDate, TimeZone, Timelike};

#[derive(Clone, Debug, PartialEq)]
pub enum FVal {
    Null,
    Num(f64),
    Str(String),
    Bool(bool),
    /// 毫秒时间戳（本地时区）
    Date(i64),
    List(Vec<FVal>),
    Err(String),
    /// 一条记录（数据表 id, 记录 id）：跨表 `[表名]`、FILTER 的结果元素
    Rec(String, String),
}

impl FVal {
    pub fn is_null(&self) -> bool {
        match self {
            FVal::Null => true,
            FVal::Str(s) => s.is_empty(),
            FVal::List(l) => l.iter().all(|x| x.is_null()),
            _ => false,
        }
    }

    pub fn num(&self) -> Option<f64> {
        match self {
            FVal::Num(n) => Some(*n),
            FVal::Bool(b) => Some(if *b { 1. } else { 0. }),
            FVal::Str(s) => s.trim().replace([',', '¥', '$', '%'], "").parse().ok(),
            FVal::Date(d) => Some(*d as f64 / 86_400_000.),
            FVal::List(l) if l.len() == 1 => l[0].num(),
            FVal::Null => Some(0.),
            _ => None,
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            FVal::Bool(b) => *b,
            FVal::Num(n) => *n != 0.,
            FVal::Str(s) => !s.is_empty() && s != "false" && s != "FALSE",
            FVal::List(l) => !l.is_empty(),
            FVal::Date(_) | FVal::Rec(..) => true,
            FVal::Null | FVal::Err(_) => false,
        }
    }

    pub fn text(&self) -> String {
        match self {
            FVal::Null => String::new(),
            FVal::Num(n) => fmt_num(*n),
            FVal::Str(s) => s.clone(),
            FVal::Bool(b) => {
                if *b {
                    "是".into()
                } else {
                    "否".into()
                }
            }
            FVal::Date(d) => fmt_date(*d, 0),
            FVal::List(l) => l.iter().map(|x| x.text()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", "),
            FVal::Err(e) => format!("#错误：{e}"),
            FVal::Rec(_, r) => r.clone(),
        }
    }

    /// 展开成一维列表（SUM 等函数用）。
    fn flat(&self, out: &mut Vec<FVal>) {
        match self {
            FVal::List(l) => l.iter().for_each(|x| x.flat(out)),
            v => out.push(v.clone()),
        }
    }
}

/// 数字显示：整数不带小数，其余最多保留 10 位有效小数并去掉尾随 0。
pub fn fmt_num(n: f64) -> String {
    if !n.is_finite() {
        return "#错误".into();
    }
    if n.fract() == 0. && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    let s = format!("{:.10}", n);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

pub const DATE_FMTS: [&str; 6] = ["2026/01/30", "2026/01/30 14:00", "2026-01-30", "01-30", "2026年01月30日", "1月30日 周五"];

pub fn fmt_date(ms: i64, fmt: u8) -> String {
    let Some(d) = Local.timestamp_millis_opt(ms).single() else { return String::new() };
    match fmt {
        1 => d.format("%Y/%m/%d %H:%M").to_string(),
        2 => d.format("%Y-%m-%d").to_string(),
        3 => d.format("%m-%d").to_string(),
        4 => d.format("%Y年%m月%d日").to_string(),
        5 => format!("{}月{}日 {}", d.month(), d.day(), ["周一", "周二", "周三", "周四", "周五", "周六", "周日"][d.weekday().num_days_from_monday() as usize]),
        _ => d.format("%Y/%m/%d").to_string(),
    }
}

/// 解析各种常见日期写法，返回当天 0 点（或带时间）的毫秒时间戳。
pub fn parse_date(s: &str) -> Option<i64> {
    let s = s.trim().replace(['年', '月'], "-").replace('日', "").replace(['/', '.'], "-");
    let (d, t) = s.split_once(' ').map(|(a, b)| (a.to_string(), Some(b.trim().to_string()))).unwrap_or((s.clone(), None));
    let parts: Vec<i64> = d.split('-').filter(|x| !x.is_empty()).filter_map(|x| x.parse().ok()).collect();
    let (y, m, day) = match parts.as_slice() {
        [y, m, d] if *y > 31 => (*y as i32, *m as u32, *d as u32),
        [m, d] => (Local::now().year(), *m as u32, *d as u32),
        _ => return None,
    };
    let date = NaiveDate::from_ymd_opt(y, m, day)?;
    let (hh, mm) = t
        .and_then(|t| {
            let (h, m) = t.split_once(':')?;
            Some((h.parse::<u32>().ok()?, m.get(..2).unwrap_or(m).parse::<u32>().ok()?))
        })
        .unwrap_or((0, 0));
    let dt = date.and_hms_opt(hh.min(23), mm.min(59), 0)?;
    Local.from_local_datetime(&dt).single().map(|d| d.timestamp_millis())
}

pub fn day_start(ms: i64) -> i64 {
    let Some(d) = Local.timestamp_millis_opt(ms).single() else { return ms };
    Local.from_local_datetime(&d.date_naive().and_hms_opt(0, 0, 0).unwrap()).single().map(|d| d.timestamp_millis()).unwrap_or(ms)
}

pub fn today() -> i64 {
    day_start(crate::store::now_ms())
}

fn ymd(ms: i64) -> Option<chrono::DateTime<Local>> {
    Local.timestamp_millis_opt(ms).single()
}

fn from_ymd(y: i32, m: i32, d: i32) -> Option<i64> {
    // 月份溢出自动进位（与 Excel DATE 一致）
    let total = y * 12 + (m - 1);
    let (y, m) = (total.div_euclid(12), total.rem_euclid(12) + 1);
    let first = NaiveDate::from_ymd_opt(y, m as u32, 1)?;
    let date = first + chrono::Duration::days((d - 1) as i64);
    Local.from_local_datetime(&date.and_hms_opt(0, 0, 0)?).single().map(|d| d.timestamp_millis())
}

// ───────────── 词法与语法 ─────────────

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    Field(String),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
    Dot,
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let cs: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = vec![];
    while i < cs.len() {
        let c = cs[i];
        match c {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '.' if !cs.get(i + 1).is_some_and(|c| c.is_ascii_digit()) => {
                out.push(Tok::Dot);
                i += 1;
            }
            '0'..='9' | '.' => {
                let st = i;
                while i < cs.len() && (cs[i].is_ascii_digit() || cs[i] == '.') {
                    i += 1;
                }
                let s: String = cs[st..i].iter().collect();
                out.push(Tok::Num(s.parse().map_err(|_| format!("无法识别的数字 {s}"))?));
            }
            '"' | '“' | '”' | '\'' => {
                let close = if c == '“' { '”' } else { c };
                i += 1;
                let st = i;
                while i < cs.len() && cs[i] != close && !(c == '"' && cs[i] == '”') {
                    i += 1;
                }
                if i >= cs.len() {
                    return Err("字符串缺少结束引号".into());
                }
                out.push(Tok::Str(cs[st..i].iter().collect()));
                i += 1;
            }
            '[' => {
                i += 1;
                let st = i;
                while i < cs.len() && cs[i] != ']' {
                    i += 1;
                }
                if i >= cs.len() {
                    return Err("字段引用缺少 ]".into());
                }
                out.push(Tok::Field(cs[st..i].iter().collect::<String>().trim().to_string()));
                i += 1;
            }
            '(' | '（' => {
                out.push(Tok::LParen);
                i += 1;
            }
            ')' | '）' => {
                out.push(Tok::RParen);
                i += 1;
            }
            ',' | '，' => {
                out.push(Tok::Comma);
                i += 1;
            }
            _ => {
                let two: String = cs[i..(i + 2).min(cs.len())].iter().collect();
                if let Some(op) = ["<=", ">=", "!=", "<>", "==", "&&", "||"].into_iter().find(|o| *o == two) {
                    out.push(Tok::Op(op));
                    i += 2;
                    continue;
                }
                if let Some(op) = ["+", "-", "*", "/", "^", "&", "=", "<", ">"].into_iter().find(|o| o.starts_with(c)) {
                    out.push(Tok::Op(op));
                    i += 1;
                    continue;
                }
                if c.is_alphabetic() || c == '_' {
                    let st = i;
                    while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '_') {
                        i += 1;
                    }
                    // WORKDAY.INTL 这类带点的函数名；其余的点是链式调用
                    if cs.get(i) == Some(&'.') {
                        let mut j = i + 1;
                        while j < cs.len() && (cs[j].is_alphanumeric() || cs[j] == '_') {
                            j += 1;
                        }
                        let name: String = cs[st..j].iter().collect::<String>().to_uppercase();
                        if j > i + 1 && FUNCS.iter().any(|f| f.0 == name) {
                            i = j;
                        }
                    }
                    out.push(Tok::Ident(cs[st..i].iter().collect()));
                    continue;
                }
                return Err(format!("无法识别的字符 {c}"));
            }
        }
    }
    Ok(out)
}

#[derive(Clone, Debug)]
pub enum Expr {
    Lit(FVal),
    Field(String),
    Call(String, Vec<Expr>),
    Bin(&'static str, Box<Expr>, Box<Expr>),
    Neg(Box<Expr>),
    /// `CurrentValue`：FILTER / MAP 里当前遍历到的元素
    Current,
    /// `x.[字段]`：取记录（集）的字段值
    Proj(Box<Expr>, String),
}

struct Parser {
    toks: Vec<Tok>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.i).cloned();
        self.i += 1;
        t
    }
    fn op_in(&mut self, ops: &[&str]) -> Option<&'static str> {
        if let Some(Tok::Op(o)) = self.peek() {
            if ops.contains(o) {
                let o = *o;
                self.i += 1;
                return Some(o);
            }
        }
        None
    }
    fn expr(&mut self) -> Result<Expr, String> {
        self.logic()
    }
    fn logic(&mut self) -> Result<Expr, String> {
        let mut l = self.cmp()?;
        while let Some(o) = self.op_in(&["&&", "||"]) {
            l = Expr::Bin(o, Box::new(l), Box::new(self.cmp()?));
        }
        Ok(l)
    }
    fn cmp(&mut self) -> Result<Expr, String> {
        let mut l = self.concat()?;
        while let Some(o) = self.op_in(&["=", "==", "!=", "<>", "<", ">", "<=", ">="]) {
            l = Expr::Bin(o, Box::new(l), Box::new(self.concat()?));
        }
        Ok(l)
    }
    fn concat(&mut self) -> Result<Expr, String> {
        let mut l = self.add()?;
        while let Some(o) = self.op_in(&["&"]) {
            l = Expr::Bin(o, Box::new(l), Box::new(self.add()?));
        }
        Ok(l)
    }
    fn add(&mut self) -> Result<Expr, String> {
        let mut l = self.mul()?;
        while let Some(o) = self.op_in(&["+", "-"]) {
            l = Expr::Bin(o, Box::new(l), Box::new(self.mul()?));
        }
        Ok(l)
    }
    fn mul(&mut self) -> Result<Expr, String> {
        let mut l = self.pow()?;
        while let Some(o) = self.op_in(&["*", "/"]) {
            l = Expr::Bin(o, Box::new(l), Box::new(self.pow()?));
        }
        Ok(l)
    }
    fn pow(&mut self) -> Result<Expr, String> {
        let l = self.unary()?;
        if let Some(o) = self.op_in(&["^"]) {
            return Ok(Expr::Bin(o, Box::new(l), Box::new(self.pow()?)));
        }
        Ok(l)
    }
    fn unary(&mut self) -> Result<Expr, String> {
        if self.op_in(&["-"]).is_some() {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.op_in(&["+"]).is_some() {
            return self.unary();
        }
        self.postfix()
    }
    /// 链式调用：`x.[字段]`、`x.FUNC(参数…)` 等价于 `FUNC(x, 参数…)`。
    fn postfix(&mut self) -> Result<Expr, String> {
        let mut e = self.primary()?;
        while self.peek() == Some(&Tok::Dot) {
            self.i += 1;
            match self.next() {
                Some(Tok::Field(f)) => e = Expr::Proj(Box::new(e), f),
                Some(Tok::Ident(name)) => {
                    let up = name.to_uppercase();
                    if self.peek() != Some(&Tok::LParen) {
                        return Err(format!("{up} 后面缺少括号"));
                    }
                    self.i += 1;
                    let mut args = vec![e];
                    if self.peek() == Some(&Tok::RParen) {
                        self.i += 1;
                    } else {
                        loop {
                            args.push(self.expr()?);
                            match self.next() {
                                Some(Tok::Comma) => continue,
                                Some(Tok::RParen) => break,
                                _ => return Err(format!("{up} 的参数缺少右括号")),
                            }
                        }
                    }
                    if !FUNCS.iter().any(|f| f.0 == up) {
                        return Err(format!("未知函数 {up}"));
                    }
                    e = Expr::Call(up, args);
                }
                _ => return Err("「.」后面应该是 [字段] 或函数".into()),
            }
        }
        Ok(e)
    }
    fn primary(&mut self) -> Result<Expr, String> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(Expr::Lit(FVal::Num(n))),
            Some(Tok::Str(s)) => Ok(Expr::Lit(FVal::Str(s))),
            Some(Tok::Field(f)) => Ok(Expr::Field(f)),
            Some(Tok::LParen) => {
                let e = self.expr()?;
                match self.next() {
                    Some(Tok::RParen) => Ok(e),
                    _ => Err("缺少右括号".into()),
                }
            }
            Some(Tok::Ident(name)) => {
                let up = name.to_uppercase();
                if self.peek() == Some(&Tok::LParen) {
                    self.i += 1;
                    let mut args = vec![];
                    if self.peek() == Some(&Tok::RParen) {
                        self.i += 1;
                    } else {
                        loop {
                            args.push(self.expr()?);
                            match self.next() {
                                Some(Tok::Comma) => continue,
                                Some(Tok::RParen) => break,
                                _ => return Err(format!("{up} 的参数缺少右括号")),
                            }
                        }
                    }
                    if !FUNCS.iter().any(|f| f.0 == up) {
                        return Err(format!("未知函数 {up}"));
                    }
                    Ok(Expr::Call(up, args))
                } else {
                    match up.as_str() {
                        "TRUE" => Ok(Expr::Lit(FVal::Bool(true))),
                        "FALSE" => Ok(Expr::Lit(FVal::Bool(false))),
                        "CURRENTVALUE" => Ok(Expr::Current),
                        _ => Err(format!("无法识别的名称 {name}（字段请用 [字段名]）")),
                    }
                }
            }
            Some(t) => Err(format!("意外的符号 {:?}", t)),
            None => Err("公式不完整".into()),
        }
    }
}

pub fn parse(src: &str) -> Result<Expr, String> {
    let src = src.trim().trim_start_matches('=');
    if src.trim().is_empty() {
        return Err("公式为空".into());
    }
    let mut p = Parser { toks: lex(src)?, i: 0 };
    let e = p.expr()?;
    if p.i < p.toks.len() {
        return Err("公式末尾有多余内容".into());
    }
    Ok(e)
}

/// 把公式里的字段引用 `[旧名]` 改成 `[新名]`（字符串字面量里的不动）。
pub fn rename_ref(src: &str, old: &str, new: &str) -> String {
    let cs: Vec<char> = src.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if matches!(c, '"' | '“' | '\'') {
            let close = if c == '“' { '”' } else { c };
            out.push(c);
            i += 1;
            while i < cs.len() && cs[i] != close && !(c == '"' && cs[i] == '”') {
                out.push(cs[i]);
                i += 1;
            }
            if i < cs.len() {
                out.push(cs[i]);
                i += 1;
            }
        } else if c == '[' {
            let st = i + 1;
            let mut j = st;
            while j < cs.len() && cs[j] != ']' {
                j += 1;
            }
            let name: String = cs[st..j.min(cs.len())].iter().collect();
            if j < cs.len() && name.trim() == old {
                out.push_str(&format!("[{new}]"));
            } else {
                out.extend(&cs[i..(j + 1).min(cs.len())]);
            }
            i = j + 1;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// 公式里引用到的字段名。
pub fn refs(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Field(f) => {
            if !out.contains(f) {
                out.push(f.clone())
            }
        }
        Expr::Call(_, a) => a.iter().for_each(|x| refs(x, out)),
        Expr::Bin(_, a, b) => {
            refs(a, out);
            refs(b, out);
        }
        Expr::Neg(a) | Expr::Proj(a, _) => refs(a, out),
        Expr::Lit(_) | Expr::Current => {}
    }
}

// ───────────── 求值 ─────────────

/// (函数名, 分类, 说明)：公式编辑器里的函数列表也用它。
pub const FUNCS: &[(&str, &str, &str)] = &[
    ("SUM", "数学", "SUM(数值1, 数值2, …) 求和"),
    ("AVERAGE", "数学", "AVERAGE(数值1, …) 平均值"),
    ("MAX", "数学", "MAX(数值1, …) 最大值"),
    ("MIN", "数学", "MIN(数值1, …) 最小值"),
    ("COUNT", "数学", "COUNT(值1, …) 数值的个数"),
    ("COUNTA", "数学", "COUNTA(值1, …) 非空值的个数"),
    ("ROUND", "数学", "ROUND(数值, 小数位) 四舍五入"),
    ("ROUNDUP", "数学", "ROUNDUP(数值, 小数位) 向上舍入"),
    ("ROUNDDOWN", "数学", "ROUNDDOWN(数值, 小数位) 向下舍入"),
    ("CEILING", "数学", "CEILING(数值) 向上取整"),
    ("FLOOR", "数学", "FLOOR(数值) 向下取整"),
    ("INT", "数学", "INT(数值) 取整数部分"),
    ("ABS", "数学", "ABS(数值) 绝对值"),
    ("MOD", "数学", "MOD(被除数, 除数) 余数"),
    ("POWER", "数学", "POWER(底数, 指数) 乘方"),
    ("SQRT", "数学", "SQRT(数值) 平方根"),
    ("IF", "逻辑", "IF(条件, 成立时, 不成立时)"),
    ("IFS", "逻辑", "IFS(条件1, 值1, 条件2, 值2, …) 取第一个成立的"),
    ("SWITCH", "逻辑", "SWITCH(表达式, 值1, 结果1, …, 默认结果)"),
    ("AND", "逻辑", "AND(条件1, 条件2, …) 全部成立"),
    ("OR", "逻辑", "OR(条件1, 条件2, …) 任一成立"),
    ("NOT", "逻辑", "NOT(条件) 取反"),
    ("ISBLANK", "逻辑", "ISBLANK(值) 是否为空"),
    ("IFERROR", "逻辑", "IFERROR(值, 出错时的值)"),
    ("CONCATENATE", "文本", "CONCATENATE(文本1, 文本2, …) 拼接"),
    ("LEN", "文本", "LEN(文本) 字符数"),
    ("LEFT", "文本", "LEFT(文本, 个数) 从左截取"),
    ("RIGHT", "文本", "RIGHT(文本, 个数) 从右截取"),
    ("MID", "文本", "MID(文本, 开始位置, 个数) 从中间截取"),
    ("UPPER", "文本", "UPPER(文本) 转大写"),
    ("LOWER", "文本", "LOWER(文本) 转小写"),
    ("TRIM", "文本", "TRIM(文本) 去掉首尾空格"),
    ("SUBSTITUTE", "文本", "SUBSTITUTE(文本, 旧文本, 新文本) 替换"),
    ("REPLACE", "文本", "REPLACE(文本, 开始位置, 个数, 新文本)"),
    ("FIND", "文本", "FIND(要找的文本, 文本) 位置，找不到为 0"),
    ("CONTAINTEXT", "文本", "CONTAINTEXT(文本, 要找的文本) 是否包含"),
    ("REPT", "文本", "REPT(文本, 次数) 重复"),
    ("TEXT", "文本", "TEXT(值, \"YYYY-MM-DD\" 或 \"0.00\") 格式化"),
    ("VALUE", "文本", "VALUE(文本) 转为数字"),
    ("TODAY", "日期", "TODAY() 今天"),
    ("NOW", "日期", "NOW() 当前时间"),
    ("DATE", "日期", "DATE(年, 月, 日)"),
    ("YEAR", "日期", "YEAR(日期) 年份"),
    ("MONTH", "日期", "MONTH(日期) 月份"),
    ("DAY", "日期", "DAY(日期) 日"),
    ("HOUR", "日期", "HOUR(日期) 小时"),
    ("MINUTE", "日期", "MINUTE(日期) 分钟"),
    ("WEEKDAY", "日期", "WEEKDAY(日期) 星期几（周一为 1）"),
    ("WEEKNUM", "日期", "WEEKNUM(日期) 一年中的第几周"),
    ("DAYS", "日期", "DAYS(结束日期, 开始日期) 相差天数"),
    ("DATEDIF", "日期", "DATEDIF(开始, 结束, \"Y\"/\"M\"/\"D\") 相差年 / 月 / 天"),
    ("EDATE", "日期", "EDATE(日期, 月数) 几个月之后"),
    ("WORKDAY", "日期", "WORKDAY(开始日期, 工作日数) 若干工作日之后"),
    ("NETWORKDAYS", "日期", "NETWORKDAYS(开始, 结束) 之间的工作日数"),
    ("WORKDAY.INTL", "日期", "WORKDAY.INTL(开始日期, 工作日数, \"0000011\") 自定义周末"),
    ("CREATED_TIME", "日期", "CREATED_TIME() 记录的创建时间"),
    ("MEDIAN", "数学", "MEDIAN(数值1, …) 中位数"),
    ("RANK", "数学", "RANK(数值, 列表, [升序]) 在列表中的排名"),
    ("EXP", "数学", "EXP(数值) e 的乘方"),
    ("LN", "数学", "LN(数值) 自然对数"),
    ("LOG", "数学", "LOG(数值, [底数=10]) 对数"),
    ("PI", "数学", "PI() 圆周率"),
    ("ISNUMBER", "逻辑", "ISNUMBER(值) 是否为数字"),
    ("ISERROR", "逻辑", "ISERROR(值) 是否出错"),
    ("RECORD_ID", "逻辑", "RECORD_ID() 当前记录的 id"),
    ("REGEXMATCH", "文本", "REGEXMATCH(文本, 正则) 是否匹配"),
    ("REGEXEXTRACT", "文本", "REGEXEXTRACT(文本, 正则) 取出第一个匹配"),
    ("REGEXREPLACE", "文本", "REGEXREPLACE(文本, 正则, 替换为) 替换所有匹配"),
    ("LIST", "列表", "LIST(值1, 值2, …) 组成列表"),
    ("ARRAYJOIN", "列表", "ARRAYJOIN(列表, [分隔符]) 拼接成文本"),
    ("FILTER", "列表", "[表名].FILTER(CurrentValue.[字段] = [字段]) 按条件筛选记录 / 列表"),
    ("MAP", "列表", "列表.MAP(CurrentValue * 2) 逐个转换"),
    ("SORT", "列表", "SORT(列表, [升序=TRUE]) 排序"),
    ("SORTBY", "列表", "SORTBY(列表, 排序依据列表, [升序=TRUE]) 按另一列表排序"),
    ("FIRST", "列表", "FIRST(列表) 第一个"),
    ("LAST", "列表", "LAST(列表) 最后一个"),
    ("NTH", "列表", "NTH(列表, 序号) 第 N 个（从 1 开始）"),
];

/// 公式求值的外部环境：字段取值、跨表取记录。
pub trait Env {
    /// 当前记录的字段值；字段不存在而有同名数据表时，返回该表全部记录（`FVal::Rec` 列表）。
    fn field(&mut self, name: &str) -> FVal;
    /// 某条记录的字段值（CurrentValue.[字段]、记录集.[字段]）
    fn rec_field(&mut self, _tid: &str, _rid: &str, name: &str) -> FVal {
        FVal::Err(format!("字段「{name}」不存在"))
    }
    fn record_id(&mut self) -> FVal {
        FVal::Null
    }
    fn created(&mut self) -> FVal {
        FVal::Null
    }
}

struct FnEnv<'a>(&'a mut dyn FnMut(&str) -> FVal);

impl Env for FnEnv<'_> {
    fn field(&mut self, name: &str) -> FVal {
        (self.0)(name)
    }
}

/// 求值上下文：外部环境 + CurrentValue 栈。
pub struct Ctx<'a> {
    env: &'a mut dyn Env,
    cur: Vec<FVal>,
}

impl Ctx<'_> {
    fn field(&mut self, name: &str) -> FVal {
        self.env.field(name)
    }
}

pub fn eval(e: &Expr, field: &mut dyn FnMut(&str) -> FVal) -> FVal {
    let mut env = FnEnv(field);
    eval_env(e, &mut env)
}

pub fn eval_env(e: &Expr, env: &mut dyn Env) -> FVal {
    let mut ctx = Ctx { env, cur: vec![] };
    ev(e, &mut ctx)
}

/// 记录（集）投影到字段：单条记录得到字段值，列表逐个展开。
fn project(v: FVal, name: &str, f: &mut Ctx) -> FVal {
    match v {
        FVal::Rec(t, r) => f.env.rec_field(&t, &r, name),
        FVal::List(l) => {
            let mut out = vec![];
            for x in l {
                match project(x, name, f) {
                    FVal::Err(e) => return FVal::Err(e),
                    FVal::List(inner) => out.extend(inner),
                    v if v.is_null() => {}
                    v => out.push(v),
                }
            }
            FVal::List(out)
        }
        FVal::Err(e) => FVal::Err(e),
        FVal::Null => FVal::Null,
        v => FVal::Err(format!("「{}」不是记录，不能取 .[{name}]", v.text())),
    }
}

fn ev(e: &Expr, field: &mut Ctx) -> FVal {
    match e {
        Expr::Lit(v) => v.clone(),
        Expr::Field(f) => field.field(f),
        Expr::Current => field.cur.last().cloned().unwrap_or(FVal::Err("CurrentValue 只能用在 FILTER / MAP 里".into())),
        Expr::Proj(a, name) => {
            let v = ev(a, field);
            project(v, name, field)
        }
        Expr::Neg(a) => match ev(a, field) {
            FVal::Err(e) => FVal::Err(e),
            v => v.num().map(|n| FVal::Num(-n)).unwrap_or_else(|| FVal::Err("负号只能用于数字".into())),
        },
        Expr::Bin(op, a, b) => {
            let (x, y) = (ev(a, field), ev(b, field));
            if let FVal::Err(e) = &x {
                return FVal::Err(e.clone());
            }
            if let FVal::Err(e) = &y {
                return FVal::Err(e.clone());
            }
            bin(op, x, y)
        }
        Expr::Call(name, args) => call(name, args, field),
    }
}

fn bin(op: &str, x: FVal, y: FVal) -> FVal {
    use FVal::*;
    match op {
        "&" => Str(x.text() + &y.text()),
        "&&" => Bool(x.truthy() && y.truthy()),
        "||" => Bool(x.truthy() || y.truthy()),
        "+" | "-" => match (&x, &y, op) {
            (Date(d), _, _) if !matches!(y, Date(_)) => {
                y.num().map(|n| Date(d + (if op == "+" { n } else { -n } * 86_400_000.) as i64)).unwrap_or(Err("日期只能加减天数".into()))
            }
            (_, Date(d), "+") => x.num().map(|n| Date(d + (n * 86_400_000.) as i64)).unwrap_or(Err("日期只能加减天数".into())),
            (Date(a), Date(b), "-") => Num(((day_start(*a) - day_start(*b)) as f64 / 86_400_000.).round()),
            _ => match (x.num(), y.num()) {
                (Some(a), Some(b)) => Num(if op == "+" { a + b } else { a - b }),
                _ => Err(format!("「{}」不是数字", if x.num().is_none() { x.text() } else { y.text() })),
            },
        },
        "*" | "/" | "^" => match (x.num(), y.num()) {
            (Some(a), Some(b)) => match op {
                "*" => Num(a * b),
                "/" if b == 0. => Err("除数为 0".into()),
                "/" => Num(a / b),
                _ => Num(a.powf(b)),
            },
            _ => Err(format!("「{}」不是数字", if x.num().is_none() { x.text() } else { y.text() })),
        },
        _ => {
            let ord = compare(&x, &y);
            Bool(match op {
                "=" | "==" => ord == std::cmp::Ordering::Equal,
                "!=" | "<>" => ord != std::cmp::Ordering::Equal,
                "<" => ord == std::cmp::Ordering::Less,
                ">" => ord == std::cmp::Ordering::Greater,
                "<=" => ord != std::cmp::Ordering::Greater,
                _ => ord != std::cmp::Ordering::Less,
            })
        }
    }
}

/// 比较两个值：都能当数字时按数字，否则按文本。
pub fn compare(x: &FVal, y: &FVal) -> std::cmp::Ordering {
    match (x, y) {
        (FVal::Date(a), FVal::Date(b)) => a.cmp(b),
        (FVal::Date(a), FVal::Str(s)) if parse_date(s).is_some() => a.cmp(&parse_date(s).unwrap_or(0)),
        (FVal::Str(s), FVal::Date(b)) if parse_date(s).is_some() => parse_date(s).unwrap_or(0).cmp(b),
        (FVal::Str(a), FVal::Str(b)) => a.cmp(b),
        (FVal::Null, FVal::Null) => std::cmp::Ordering::Equal,
        (FVal::Null, _) if y.is_null() => std::cmp::Ordering::Equal,
        (_, FVal::Null) if x.is_null() => std::cmp::Ordering::Equal,
        _ => match (x.num(), y.num()) {
            (Some(a), Some(b)) if !matches!(x, FVal::Str(_)) || !matches!(y, FVal::Str(_)) => a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal),
            _ => x.text().cmp(&y.text()),
        },
    }
}

fn arg(args: &[Expr], i: usize, f: &mut Ctx) -> FVal {
    args.get(i).map(|e| ev(e, f)).unwrap_or(FVal::Null)
}

fn nums(args: &[Expr], f: &mut Ctx) -> Result<Vec<f64>, String> {
    let mut flat = vec![];
    for a in args {
        match ev(a, f) {
            FVal::Err(e) => return Err(e),
            v => v.flat(&mut flat),
        }
    }
    Ok(flat.into_iter().filter(|v| !v.is_null()).filter_map(|v| v.num()).collect())
}

fn to_date(v: &FVal) -> Option<i64> {
    match v {
        FVal::Date(d) => Some(*d),
        FVal::Str(s) => parse_date(s),
        FVal::Num(n) => Some((*n * 86_400_000.) as i64),
        FVal::List(l) if l.len() == 1 => to_date(&l[0]),
        _ => None,
    }
}

fn is_workday(ms: i64) -> bool {
    ymd(ms).map(|d| d.weekday().num_days_from_monday() < 5).unwrap_or(true)
}

/// 在公式函数里定义取参工具：`need(n)` 与 `num1!` / `date1!` / `text1!`（参数不合法时直接从所在函数返回错误）。
/// 用 `$d` 传入 `$`，在宏里再定义宏。
#[rustfmt::skip]
macro_rules! arg_helpers {
    ($d:tt $args:ident $f:ident $name:ident $need:ident) => {
        #[allow(unused)]
        let $need = |n: usize| -> Option<FVal> { ($args.len() < n).then(|| Err(format!("{} 需要 {n} 个参数", $name))) };
        #[allow(unused_macros)]
        macro_rules! num1 {
            ($d i:expr) => {
                match arg($args, $d i, $f) {
                    Err(e) => return Err(e),
                    v => match v.num() {
                        Some(n) => n,
                        None => return Err(format!("{} 的参数「{}」不是数字", $name, v.text())),
                    },
                }
            };
        }
        #[allow(unused_macros)]
        macro_rules! date1 {
            ($d i:expr) => {
                match arg($args, $d i, $f) {
                    Err(e) => return Err(e),
                    v if v.is_null() => return Null,
                    v => match to_date(&v) {
                        Some(d) => d,
                        None => return Err(format!("{} 的参数「{}」不是日期", $name, v.text())),
                    },
                }
            };
        }
        #[allow(unused_macros)]
        macro_rules! text1 {
            ($d i:expr) => {
                match arg($args, $d i, $f) {
                    Err(e) => return Err(e),
                    v => v.text(),
                }
            };
        }
    };
}

/// 内置函数求值：数学与逻辑；没有命中的交给下一组（文本 → 日期 → 列表 / 其他）。
fn call(name: &str, args: &[Expr], f: &mut Ctx) -> FVal {
    use FVal::*;
    arg_helpers!($ args f name need);
    match name {
        "SUM" | "AVERAGE" | "MAX" | "MIN" | "COUNT" => {
            let v = match nums(args, f) {
                Ok(v) => v,
                Result::Err(e) => return Err(e),
            };
            match name {
                "SUM" => Num(v.iter().sum()),
                "AVERAGE" if v.is_empty() => Null,
                "AVERAGE" => Num(v.iter().sum::<f64>() / v.len() as f64),
                "MAX" => v.iter().cloned().fold(None, |a: Option<f64>, b| Some(a.map_or(b, |a| a.max(b)))).map(Num).unwrap_or(Null),
                "MIN" => v.iter().cloned().fold(None, |a: Option<f64>, b| Some(a.map_or(b, |a| a.min(b)))).map(Num).unwrap_or(Null),
                _ => Num(v.len() as f64),
            }
        }
        "COUNTA" => {
            let mut flat = vec![];
            for a in args {
                ev(a, f).flat(&mut flat);
            }
            Num(flat.iter().filter(|v| !v.is_null()).count() as f64)
        }
        "ROUND" | "ROUNDUP" | "ROUNDDOWN" => {
            if let Some(e) = need(1) {
                return e;
            }
            let x = num1!(0);
            let d = if args.len() > 1 { num1!(1) } else { 0. };
            let k = 10f64.powi(d as i32);
            Num(match name {
                "ROUND" => (x * k).round() / k,
                "ROUNDUP" => (x * k).abs().ceil().copysign(x) / k,
                _ => (x * k).abs().floor().copysign(x) / k,
            })
        }
        "CEILING" => Num(num1!(0).ceil()),
        "FLOOR" | "INT" => Num(num1!(0).floor()),
        "ABS" => Num(num1!(0).abs()),
        "SQRT" => {
            let x = num1!(0);
            if x < 0. {
                Err("负数不能开平方".into())
            } else {
                Num(x.sqrt())
            }
        }
        "MOD" => {
            let (a, b) = (num1!(0), num1!(1));
            if b == 0. {
                Err("除数为 0".into())
            } else {
                Num(a.rem_euclid(b))
            }
        }
        "POWER" => Num(num1!(0).powf(num1!(1))),
        "IF" => {
            if let Some(e) = need(2) {
                return e;
            }
            match arg(args, 0, f) {
                Err(e) => Err(e),
                c if c.truthy() => arg(args, 1, f),
                _ => arg(args, 2, f),
            }
        }
        "IFS" => {
            for pair in args.chunks(2) {
                if pair.len() == 2 && ev(&pair[0], f).truthy() {
                    return ev(&pair[1], f);
                }
            }
            Null
        }
        "SWITCH" => {
            let v = arg(args, 0, f);
            let rest = &args[1.min(args.len())..];
            for pair in rest.chunks(2) {
                if pair.len() == 1 {
                    return ev(&pair[0], f);
                }
                if compare(&v, &ev(&pair[0], f)) == std::cmp::Ordering::Equal {
                    return ev(&pair[1], f);
                }
            }
            Null
        }
        "AND" => Bool(args.iter().all(|a| ev(a, f).truthy())),
        "OR" => Bool(args.iter().any(|a| ev(a, f).truthy())),
        "NOT" => Bool(!arg(args, 0, f).truthy()),
        "ISBLANK" => Bool(arg(args, 0, f).is_null()),
        "IFERROR" => match arg(args, 0, f) {
            Err(_) => arg(args, 1, f),
            v => v,
        },
        _ => call_text(name, args, f),
    }
}

/// 文本函数。
fn call_text(name: &str, args: &[Expr], f: &mut Ctx) -> FVal {
    use FVal::*;
    arg_helpers!($ args f name need);
    match name {
        "CONCATENATE" => Str(args.iter().map(|a| ev(a, f).text()).collect()),
        "LEN" => Num(text1!(0).chars().count() as f64),
        "LEFT" | "RIGHT" => {
            let s: Vec<char> = text1!(0).chars().collect();
            let n = if args.len() > 1 { num1!(1).max(0.) as usize } else { 1 };
            let n = n.min(s.len());
            Str(if name == "LEFT" { s[..n].iter().collect() } else { s[s.len() - n..].iter().collect() })
        }
        "MID" => {
            let s: Vec<char> = text1!(0).chars().collect();
            let st = (num1!(1).max(1.) as usize - 1).min(s.len());
            let n = (num1!(2).max(0.) as usize).min(s.len() - st);
            Str(s[st..st + n].iter().collect())
        }
        "UPPER" => Str(text1!(0).to_uppercase()),
        "LOWER" => Str(text1!(0).to_lowercase()),
        "TRIM" => Str(text1!(0).trim().to_string()),
        "SUBSTITUTE" => Str(text1!(0).replace(&text1!(1), &text1!(2))),
        "REPLACE" => {
            let s: Vec<char> = text1!(0).chars().collect();
            let st = (num1!(1).max(1.) as usize - 1).min(s.len());
            let n = (num1!(2).max(0.) as usize).min(s.len() - st);
            let new = text1!(3);
            Str(s[..st].iter().collect::<String>() + &new + &s[st + n..].iter().collect::<String>())
        }
        "FIND" => {
            let (needle, hay) = (text1!(0), text1!(1));
            Num(hay.find(&needle).map(|b| hay[..b].chars().count() as f64 + 1.).unwrap_or(0.))
        }
        "CONTAINTEXT" => Bool(text1!(0).contains(&text1!(1))),
        "REPT" => Str(text1!(0).repeat(num1!(1).clamp(0., 1000.) as usize)),
        "VALUE" => {
            let s = text1!(0);
            FVal::Str(s.clone()).num().map(Num).unwrap_or(Err(format!("「{s}」不是数字")))
        }
        "TEXT" => {
            let v = arg(args, 0, f);
            let fmt = text1!(1);
            match (&v, to_date(&v)) {
                (Date(_), Some(d)) | (Str(_), Some(d)) if fmt.to_uppercase().contains('Y') || fmt.contains("MM") || fmt.to_uppercase().contains("DD") => {
                    let pat = fmt
                        .replace("YYYY", "%Y")
                        .replace("yyyy", "%Y")
                        .replace("MM", "%m")
                        .replace("DD", "%d")
                        .replace("dd", "%d")
                        .replace("HH", "%H")
                        .replace("hh", "%H")
                        .replace("mm", "%M")
                        .replace("ss", "%S");
                    Str(ymd(d).map(|x| x.format(&pat).to_string()).unwrap_or_default())
                }
                _ => match v.num() {
                    Some(n) if fmt.contains('0') => {
                        let pct = fmt.ends_with('%');
                        let dec = fmt.split('.').nth(1).map(|s| s.chars().filter(|c| *c == '0').count()).unwrap_or(0);
                        let n = if pct { n * 100. } else { n };
                        Str(format!("{:.*}{}", dec, n, if pct { "%" } else { "" }))
                    }
                    _ => Str(v.text()),
                },
            }
        }
        _ => call_date(name, args, f),
    }
}

/// 日期函数。
fn call_date(name: &str, args: &[Expr], f: &mut Ctx) -> FVal {
    use FVal::*;
    arg_helpers!($ args f name need);
    match name {
        "TODAY" => Date(today()),
        "NOW" => Date(crate::store::now_ms()),
        "DATE" => from_ymd(num1!(0) as i32, num1!(1) as i32, num1!(2) as i32).map(Date).unwrap_or(Err("日期无效".into())),
        "YEAR" | "MONTH" | "DAY" | "HOUR" | "MINUTE" | "WEEKDAY" | "WEEKNUM" => {
            let d = date1!(0);
            let Some(t) = ymd(d) else { return Err("日期无效".into()) };
            Num(match name {
                "YEAR" => t.year() as f64,
                "MONTH" => t.month() as f64,
                "DAY" => t.day() as f64,
                "HOUR" => t.hour() as f64,
                "MINUTE" => t.minute() as f64,
                "WEEKDAY" => t.weekday().number_from_monday() as f64,
                _ => t.iso_week().week() as f64,
            })
        }
        "DAYS" => {
            let (a, b) = (date1!(0), date1!(1));
            Num(((day_start(a) - day_start(b)) as f64 / 86_400_000.).round())
        }
        "DATEDIF" => {
            let (a, b) = (date1!(0), date1!(1));
            let unit = text1!(2).to_uppercase();
            let (Some(x), Some(y)) = (ymd(a), ymd(b)) else { return Err("日期无效".into()) };
            Num(match unit.as_str() {
                "Y" => {
                    let mut n = y.year() - x.year();
                    if (y.month(), y.day()) < (x.month(), x.day()) {
                        n -= 1;
                    }
                    n as f64
                }
                "M" => {
                    let mut n = (y.year() - x.year()) * 12 + y.month() as i32 - x.month() as i32;
                    if y.day() < x.day() {
                        n -= 1;
                    }
                    n as f64
                }
                _ => ((day_start(b) - day_start(a)) as f64 / 86_400_000.).round(),
            })
        }
        "EDATE" => {
            let d = date1!(0);
            let m = num1!(1) as i32;
            let Some(t) = ymd(d) else { return Err("日期无效".into()) };
            // 目标月没有这一天时取当月最后一天（1 月 31 日加 1 个月是 2 月 28 日，与 Excel 一致）
            let first = from_ymd(t.year(), t.month() as i32 + m, 1);
            let last = from_ymd(t.year(), t.month() as i32 + m + 1, 0).and_then(ymd).map(|x| x.day() as i32).unwrap_or(28);
            first.map(|d| Date(d + (t.day() as i32).min(last) as i64 * 86_400_000 - 86_400_000)).unwrap_or(Err("日期无效".into()))
        }
        "WORKDAY" => {
            let mut d = day_start(date1!(0));
            let mut n = (num1!(1) as i64).clamp(-100_000, 100_000);
            let step = if n >= 0 { 86_400_000 } else { -86_400_000 };
            while n != 0 {
                d += step;
                if is_workday(d) {
                    n -= step.signum();
                }
            }
            Date(d)
        }
        "NETWORKDAYS" => {
            let (a, b) = (day_start(date1!(0)), day_start(date1!(1)));
            let (lo, hi) = (a.min(b), a.max(b));
            let mut n = 0;
            let mut d = lo;
            while d <= hi && n < 100_000 {
                if is_workday(d) {
                    n += 1;
                }
                d += 86_400_000;
            }
            Num(if a <= b { n as f64 } else { -(n as f64) })
        }
        "WORKDAY.INTL" => {
            let mut d = day_start(date1!(0));
            let mut n = (num1!(1) as i64).clamp(-100_000, 100_000);
            let mask: Vec<bool> = {
                let m = if args.len() > 2 { text1!(2) } else { "0000011".into() };
                let m: Vec<bool> = m.chars().map(|c| c == '1').collect();
                if m.len() == 7 {
                    m
                } else {
                    vec![false, false, false, false, false, true, true]
                }
            };
            if mask.iter().all(|x| *x) {
                return Err("每周至少要有一个工作日".into());
            }
            let step = if n >= 0 { 86_400_000 } else { -86_400_000 };
            while n != 0 {
                d += step;
                let wd = ymd(d).map(|x| x.weekday().num_days_from_monday() as usize).unwrap_or(0);
                if !mask[wd] {
                    n -= step.signum();
                }
            }
            Date(d)
        }
        "CREATED_TIME" => f.env.created(),
        "RECORD_ID" => f.env.record_id(),
        _ => call_list(name, args, f),
    }
}

/// 统计、正则、列表与其他函数。
fn call_list(name: &str, args: &[Expr], f: &mut Ctx) -> FVal {
    use FVal::*;
    arg_helpers!($ args f name need);
    match name {
        "MEDIAN" => {
            let mut v = match nums(args, f) {
                Ok(v) => v,
                Result::Err(e) => return Err(e),
            };
            if v.is_empty() {
                return Null;
            }
            v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let m = v.len() / 2;
            Num(if v.len() % 2 == 1 { v[m] } else { (v[m - 1] + v[m]) / 2. })
        }
        "RANK" => {
            let x = num1!(0);
            let list = match nums(&args[1.min(args.len())..2.min(args.len())], f) {
                Ok(v) => v,
                Result::Err(e) => return Err(e),
            };
            let asc = args.len() > 2 && arg(args, 2, f).truthy();
            if !list.iter().any(|v| (v - x).abs() < 1e-12) {
                return Err("数值不在列表中".into());
            }
            Num(1. + list.iter().filter(|v| if asc { **v < x } else { **v > x }).count() as f64)
        }
        "EXP" => Num(num1!(0).exp()),
        "LN" | "LOG" => {
            let x = num1!(0);
            if x <= 0. {
                return Err("对数的参数必须大于 0".into());
            }
            if name == "LN" {
                Num(x.ln())
            } else {
                let base = if args.len() > 1 { num1!(1) } else { 10. };
                Num(x.ln() / base.ln())
            }
        }
        "PI" => Num(std::f64::consts::PI),
        "ISNUMBER" => Bool(matches!(arg(args, 0, f), Num(_))),
        "ISERROR" => Bool(matches!(arg(args, 0, f), Err(_))),
        "REGEXMATCH" | "REGEXEXTRACT" | "REGEXREPLACE" => {
            let (text, pat) = (text1!(0), text1!(1));
            let Ok(re) = regex::Regex::new(&pat) else { return Err(format!("正则表达式有误：{pat}")) };
            match name {
                "REGEXMATCH" => Bool(re.is_match(&text)),
                "REGEXEXTRACT" => {
                    re.captures(&text).map(|c| Str(c.get(1).or_else(|| c.get(0)).map(|m| m.as_str().to_string()).unwrap_or_default())).unwrap_or(Null)
                }
                _ => Str(re.replace_all(&text, text1!(2).as_str()).to_string()),
            }
        }
        "LIST" => {
            let mut out = vec![];
            for a in args {
                match ev(a, f) {
                    Err(e) => return Err(e),
                    v => v.flat(&mut out),
                }
            }
            List(out.into_iter().filter(|v| !v.is_null()).collect())
        }
        "ARRAYJOIN" => {
            let mut flat = vec![];
            arg(args, 0, f).flat(&mut flat);
            let sep = if args.len() > 1 { text1!(1) } else { ",".into() };
            Str(flat.iter().filter(|v| !v.is_null()).map(|v| v.text()).collect::<Vec<_>>().join(&sep))
        }
        "FILTER" | "MAP" => {
            if let Some(e) = need(2) {
                return e;
            }
            let mut items = vec![];
            match arg(args, 0, f) {
                Err(e) => return Err(e),
                v => v.flat(&mut items),
            }
            let mut out = vec![];
            for it in items {
                f.cur.push(it.clone());
                let r = ev(&args[1], f);
                f.cur.pop();
                match r {
                    Err(e) => return Err(e),
                    r if name == "MAP" => r.flat(&mut out),
                    r if r.truthy() => out.push(it),
                    _ => {}
                }
            }
            List(out)
        }
        "SORT" | "SORTBY" => {
            let mut items = vec![];
            arg(args, 0, f).flat(&mut items);
            let (keys, asc_i) = if name == "SORTBY" {
                let mut k = vec![];
                arg(args, 1, f).flat(&mut k);
                (k, 2)
            } else {
                (items.clone(), 1)
            };
            let asc = args.len() <= asc_i || arg(args, asc_i, f).truthy();
            let mut ix: Vec<usize> = (0..items.len()).collect();
            ix.sort_by(|a, b| {
                let o = compare(keys.get(*a).unwrap_or(&Null), keys.get(*b).unwrap_or(&Null));
                if asc {
                    o
                } else {
                    o.reverse()
                }
            });
            List(ix.into_iter().map(|i| items[i].clone()).collect())
        }
        "FIRST" | "LAST" | "NTH" => {
            let mut items = vec![];
            arg(args, 0, f).flat(&mut items);
            let i = match name {
                "FIRST" => Some(0),
                "LAST" => items.len().checked_sub(1),
                _ => (num1!(1) as usize).checked_sub(1),
            };
            i.and_then(|i| items.get(i).cloned()).unwrap_or(Null)
        }
        _ => Err(format!("未知函数 {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{eval, fmt_num, parse, parse_date, FVal};

    fn run(src: &str) -> FVal {
        let e = parse(src).unwrap();
        eval(&e, &mut |f| match f {
            "数量" => FVal::Num(3.),
            "单价" => FVal::Num(12.5),
            "名称" => FVal::Str("苹果".into()),
            "日期" => FVal::Date(parse_date("2026-10-28").unwrap()),
            "标签" => FVal::List(vec![FVal::Str("甲".into()), FVal::Str("乙".into())]),
            _ => FVal::Null,
        })
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(run("[数量]*[单价]+1"), FVal::Num(38.5));
        assert_eq!(run("(1+2)*3^2"), FVal::Num(27.));
        assert_eq!(run("-[数量] + 10"), FVal::Num(7.));
        assert_eq!(fmt_num(0.1 + 0.2), "0.3");
    }

    #[test]
    fn text_and_logic() {
        assert_eq!(run("[名称] & \"×\" & [数量]"), FVal::Str("苹果×3".into()));
        assert_eq!(run("IF([数量]>2, \"多\", \"少\")"), FVal::Str("多".into()));
        assert_eq!(run("AND([数量]>1, [单价]<20)"), FVal::Bool(true));
        assert_eq!(run("LEN(\"你好世界\")"), FVal::Num(4.));
        assert_eq!(run("MID(\"abcdef\", 2, 3)"), FVal::Str("bcd".into()));
        assert_eq!(run("SWITCH([数量], 1, \"一\", 3, \"三\", \"其他\")"), FVal::Str("三".into()));
        assert_eq!(run("COUNTA([标签])"), FVal::Num(2.));
        assert_eq!(run("ISBLANK([不存在])"), FVal::Bool(true));
    }

    #[test]
    fn dates() {
        assert_eq!(run("YEAR([日期])"), FVal::Num(2026.));
        assert_eq!(run("DAYS([日期]+3, [日期])"), FVal::Num(3.));
        assert_eq!(run("TEXT([日期], \"YYYY年MM月DD日\")"), FVal::Str("2026年10月28日".into()));
        assert_eq!(run("WEEKDAY([日期])"), FVal::Num(3.));
        assert_eq!(run("TEXT(EDATE([日期], 4), \"YYYY-MM-DD\")"), FVal::Str("2027-02-28".into()));
        assert_eq!(run("NETWORKDAYS(DATE(2026,10,26), DATE(2026,11,1))"), FVal::Num(5.));
    }

    #[test]
    fn errors() {
        assert!(parse("SUM(1,").is_err());
        assert!(parse("FOO(1)").is_err());
        assert!(matches!(run("1/0"), FVal::Err(_)));
        assert_eq!(run("IFERROR(1/0, 0)"), FVal::Num(0.));
    }
}
