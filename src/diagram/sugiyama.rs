//! Sugiyama 分层布局：去环 → 分层 → 插入虚拟节点 → 重心法减少交叉 → 坐标分配。
//! 流程图、类图、状态图、ER 图共用。

use crate::canvas::{pt, Pt};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    TB,
    BT,
    LR,
    RL,
}

impl Dir {
    pub fn parse(s: &str) -> Option<Dir> {
        Some(match s.trim().to_uppercase().as_str() {
            "TB" | "TD" => Dir::TB,
            "BT" => Dir::BT,
            "LR" => Dir::LR,
            "RL" => Dir::RL,
            _ => return None,
        })
    }
    pub fn horizontal(self) -> bool {
        matches!(self, Dir::LR | Dir::RL)
    }
}

#[derive(Clone, Debug)]
pub struct LNode {
    pub w: f32,
    pub h: f32,
    /// 所属子图（用于把同一子图的节点排在一起）
    pub cluster: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct LEdge {
    pub from: usize,
    pub to: usize,
    pub minlen: usize,
    /// 边标签估算宽度（用于多跨层虚拟节点预留走廊，0 表示无标签）
    pub label_w: f32,
}

pub struct Layout {
    /// 节点中心
    pub pos: Vec<Pt>,
    /// 每条边的折线（起点为源节点中心、终点为目标节点中心，中间为虚拟节点位置）
    pub edges: Vec<Vec<Pt>>,
    /// 每个实节点所在的层号
    pub node_layer: Vec<usize>,
    /// 每一层沿主轴方向占用的坐标区间 `(min, max)`（与 `pos` 同坐标系），相邻区间之间即为无节点的空走廊
    pub layer_spans: Vec<(f32, f32)>,
}

pub struct Spacing {
    pub node: f32,
    pub rank: f32,
}

impl Default for Spacing {
    fn default() -> Self {
        Spacing { node: 36., rank: 54. }
    }
}

pub fn layout(nodes: &[LNode], edges: &[LEdge], dir: Dir, sp: &Spacing) -> Layout {
    let n = nodes.len();
    if n == 0 {
        return Layout { pos: vec![], edges: edges.iter().map(|_| vec![]).collect(), node_layer: vec![], layer_spans: vec![] };
    }
    // 统一在「从上到下」的坐标系里计算；横向布局时交换宽高
    let size = |i: usize| -> (f32, f32) {
        let nd = &nodes[i];
        if dir.horizontal() {
            (nd.h, nd.w)
        } else {
            (nd.w, nd.h)
        }
    };

    // ── 1. 去环：DFS 找回边并反转 ──
    let mut adj: Vec<Vec<usize>> = vec![vec![]; n];
    for (k, e) in edges.iter().enumerate() {
        if e.from != e.to {
            adj[e.from].push(k);
        }
    }
    let mut reversed = vec![false; edges.len()];
    let mut state = vec![0u8; n]; // 0 未访问 1 栈中 2 完成
    for s in 0..n {
        if state[s] != 0 {
            continue;
        }
        let mut stack = vec![(s, 0usize)];
        state[s] = 1;
        while let Some(&mut (v, ref mut ix)) = stack.last_mut() {
            if *ix < adj[v].len() {
                let k = adj[v][*ix];
                *ix += 1;
                let w = edges[k].to;
                match state[w] {
                    0 => {
                        state[w] = 1;
                        stack.push((w, 0));
                    }
                    1 => reversed[k] = true,
                    _ => {}
                }
            } else {
                state[v] = 2;
                stack.pop();
            }
        }
    }
    // 有向无环的边集（u → v）
    let dag: Vec<(usize, usize, usize, usize)> = edges
        .iter()
        .enumerate()
        .filter(|(_, e)| e.from != e.to)
        .map(|(k, e)| if reversed[k] { (e.to, e.from, e.minlen.max(1), k) } else { (e.from, e.to, e.minlen.max(1), k) })
        .collect();

    // ── 2. 分层：网络单纯形，让连线总长最短 ──
    let layer = rank(n, &dag);

    // ── 3. 插入虚拟节点 ──
    struct V {
        w: f32,
        h: f32,
        cluster: Option<usize>,
        layer: usize,
    }
    let mut vs: Vec<V> = (0..n).map(|i| V { w: size(i).0, h: size(i).1, cluster: nodes[i].cluster, layer: layer[i] }).collect();
    let mut chains: HashMap<usize, Vec<usize>> = HashMap::new(); // 边序号 → 经过的顶点（含首尾）
    let mut vadj_up: Vec<Vec<usize>> = vec![vec![]; n];
    let mut vadj_down: Vec<Vec<usize>> = vec![vec![]; n];
    for &(u, v, _, k) in &dag {
        let mut chain = vec![u];
        let mut prev = u;
        let span = layer[v].saturating_sub(layer[u]);
        let mid_l = layer[u] + span / 2;
        let lw = edges[k].label_w;
        for l in layer[u] + 1..layer[v] {
            let id = vs.len();
            let cl = if nodes[u].cluster == nodes[v].cluster { nodes[u].cluster } else { None };
            // 多跨层带标签的边在中点层预留适度宽度，避免并行长边的标签互相挤压
            let dw = if l == mid_l && lw > 0. { (lw * 0.65).clamp(18., 96.) } else { 14. };
            vs.push(V { w: dw, h: 8., cluster: cl, layer: l });
            vadj_up.push(vec![]);
            vadj_down.push(vec![]);
            vadj_down[prev].push(id);
            vadj_up[id].push(prev);
            chain.push(id);
            prev = id;
        }
        vadj_down[prev].push(v);
        vadj_up[v].push(prev);
        chain.push(v);
        chains.insert(k, chain);
    }
    let nlayers = vs.iter().map(|v| v.layer).max().unwrap_or(0) + 1;
    let mut layers: Vec<Vec<usize>> = vec![vec![]; nlayers];
    // 初始顺序：从上层往下深度优先遍历，按连线书写顺序依次放入各层（同 dagre initOrder），
    // 长边的虚拟节点紧跟它所在的分支，保持用户书写顺序的直觉（先写的分支在左）
    let mut starts: Vec<usize> = (0..n).collect();
    starts.sort_by_key(|&v| vs[v].layer);
    let mut seen = vec![false; vs.len()];
    for s in starts {
        let mut stack = vec![s];
        while let Some(v) = stack.pop() {
            if seen[v] {
                continue;
            }
            seen[v] = true;
            layers[vs[v].layer].push(v);
            stack.extend(vadj_down[v].iter().rev().filter(|&&w| !seen[w]));
        }
    }

    // ── 4. 重心法 + 相邻交换（transpose）减少交叉（同一子图的节点保持相邻）──
    let mut order_pos = vec![0f32; vs.len()];
    let refresh = |layers: &Vec<Vec<usize>>, order_pos: &mut Vec<f32>| {
        for l in layers {
            for (i, &v) in l.iter().enumerate() {
                order_pos[v] = i as f32;
            }
        }
    };
    refresh(&layers, &mut order_pos);
    let mut is_back_dummy = vec![false; vs.len()];
    for (&k, ch) in &chains {
        if reversed[k] {
            for &d in &ch[1..ch.len().saturating_sub(1)] {
                is_back_dummy[d] = true;
            }
        }
    }
    let clusters: Vec<Option<usize>> = vs.iter().map(|v| v.cluster).collect();
    transpose(&mut layers, &vadj_up, &vadj_down, &clusters, &is_back_dummy, vs.len());
    refresh(&layers, &mut order_pos);
    let mut best = (crossings(&layers, &vadj_down, vs.len()), layers.clone());
    for iter in 0..24 {
        let down = iter % 2 == 0;
        let range: Vec<usize> = if down { (1..nlayers).collect() } else { (0..nlayers.saturating_sub(1)).rev().collect() };
        for li in range {
            let mut keyed: Vec<(f32, f32, usize)> = layers[li]
                .iter()
                .map(|&v| {
                    let nb = if down { &vadj_up[v] } else { &vadj_down[v] };
                    let bc = if nb.is_empty() { order_pos[v] } else { nb.iter().map(|&u| order_pos[u]).sum::<f32>() / nb.len() as f32 };
                    (bc, 0., v)
                })
                .collect();
            // 子图分组键：组内平均重心
            let mut group: HashMap<usize, (f32, usize)> = HashMap::new();
            for &(bc, _, v) in &keyed {
                if let Some(c) = vs[v].cluster {
                    let e = group.entry(c).or_insert((0., 0));
                    e.0 += bc;
                    e.1 += 1;
                }
            }
            for k in keyed.iter_mut() {
                k.1 = match vs[k.2].cluster {
                    Some(c) => {
                        let (s, cnt) = group[&c];
                        s / cnt as f32
                    }
                    None => k.0,
                };
            }
            keyed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap().then(a.0.partial_cmp(&b.0).unwrap()));
            layers[li] = keyed.into_iter().map(|k| k.2).collect();
            for (i, &v) in layers[li].iter().enumerate() {
                order_pos[v] = i as f32;
            }
        }
        transpose(&mut layers, &vadj_up, &vadj_down, &clusters, &is_back_dummy, vs.len());
        refresh(&layers, &mut order_pos);
        let c = crossings(&layers, &vadj_down, vs.len());
        if c < best.0 {
            best = (c, layers.clone());
            if c == 0 {
                break;
            }
        }
    }
    let mut layers = best.1;
    // 最终再做一次同交叉数外推，把回边走廊稳稳留在最左/最右外缘
    transpose(&mut layers, &vadj_up, &vadj_down, &clusters, &is_back_dummy, vs.len());

    // ── 5. 坐标分配：Brandes-Köpf + 长边拉直 + 父子居中对齐 ──
    let gap = |a: usize, b: usize| -> f32 {
        // 虚拟节点之间保留清晰走廊；不同子图之间留更大空隙给子图边框
        let half = |v: usize| if v >= n { 10. } else { sp.node / 2. };
        let extra = if vs[a].cluster != vs[b].cluster { 24. } else { 0. };
        (vs[a].w + vs[b].w) / 2. + half(a) + half(b) + extra
    };
    let widths: Vec<f32> = vs.iter().map(|v| v.w).collect();
    let mut x = brandes_kopf(&layers, &vadj_up, &vadj_down, &widths, n, &gap);
    straighten_and_center(&mut x, &layers, &chains, &reversed, &vadj_up, &vadj_down, n, &gap);
    // 层的纵坐标与主轴跨度
    let mut ly = vec![0f32; nlayers];
    let mut lh = vec![0f32; nlayers];
    let mut acc = 0.;
    for (li, l) in layers.iter().enumerate() {
        let h = l.iter().filter(|&&v| v < n).map(|&v| vs[v].h).fold(0., f32::max).max(8.);
        lh[li] = h;
        ly[li] = acc + h / 2.;
        acc += h + sp.rank;
    }
    let minx = (0..vs.len()).map(|v| x[v] - vs[v].w / 2.).fold(f32::INFINITY, f32::min);
    let total_y = acc - sp.rank;
    let total_x = (0..vs.len()).map(|v| x[v] + vs[v].w / 2.).fold(f32::NEG_INFINITY, f32::max) - minx;
    let layer_spans: Vec<(f32, f32)> = (0..nlayers)
        .map(|li| {
            let (a, b) = (ly[li] - lh[li] / 2., ly[li] + lh[li] / 2.);
            match dir {
                Dir::TB | Dir::LR => (a, b),
                Dir::BT | Dir::RL => (total_y - b, total_y - a),
            }
        })
        .collect();
    let place = |v: usize| -> Pt {
        let (px, py) = (x[v] - minx, ly[vs[v].layer]);
        match dir {
            Dir::TB => pt(px, py),
            Dir::BT => pt(px, total_y - py),
            Dir::LR => pt(py, px),
            Dir::RL => pt(total_y - py, px),
        }
    };
    let _ = total_x;
    let pos: Vec<Pt> = (0..n).map(place).collect();
    let mut out_edges = vec![];
    for (k, e) in edges.iter().enumerate() {
        if e.from == e.to {
            // 自环：从节点右侧绕一个小圈
            let c = pos[e.from];
            let (w, h) = (nodes[e.from].w / 2., nodes[e.from].h / 2.);
            out_edges.push(vec![pt(c.x + w * 0.4, c.y - h), pt(c.x + w + 24., c.y - h - 12.), pt(c.x + w + 24., c.y + h + 12.), pt(c.x + w * 0.4, c.y + h)]);
            continue;
        }
        let chain = chains.get(&k).cloned().unwrap_or_default();
        let mut pts: Vec<Pt> = chain.iter().map(|&v| place(v)).collect();
        if reversed[k] {
            pts.reverse();
        }
        out_edges.push(pts);
    }
    Layout { pos, edges: out_edges, node_layer: layer, layer_spans }
}

/// 分层：网络单纯形（移植自 dagre `rank/network-simplex.js`，Gansner et al. "A Technique for Drawing
/// Directed Graphs"），各连通分量分别求解，层号从 0 开始。
/// 之后入边数等于出边数的节点下沉到紧挨下游的一层（分支汇合前的步骤排在同一行，长边留在分支一侧）。
/// `dag` 为 (起点, 终点, 最小跨层数, 原边序号)。
fn rank(n: usize, dag: &[(usize, usize, usize, usize)]) -> Vec<usize> {
    // 合并重边：权重相加、最小跨层取大
    let mut idx: HashMap<(usize, usize), usize> = HashMap::new();
    let mut es: Vec<(usize, usize, i32, i32)> = vec![]; // (u, v, minlen, weight)
    for &(u, v, ml, _) in dag {
        match idx.get(&(u, v)) {
            Some(&k) => {
                es[k].2 = es[k].2.max(ml as i32);
                es[k].3 += 1;
            }
            None => {
                idx.insert((u, v), es.len());
                es.push((u, v, ml as i32, 1));
            }
        }
    }
    let mut inc: Vec<Vec<usize>> = vec![vec![]; n];
    for (k, e) in es.iter().enumerate() {
        inc[e.0].push(k);
        inc[e.1].push(k);
    }
    // 拓扑序
    let mut indeg = vec![0usize; n];
    for e in &es {
        indeg[e.1] += 1;
    }
    let mut topo: Vec<usize> = (0..n).filter(|&v| indeg[v] == 0).collect();
    let mut i = 0;
    while i < topo.len() {
        let v = topo[i];
        i += 1;
        for &k in &inc[v] {
            if es[k].0 == v {
                indeg[es[k].1] -= 1;
                if indeg[es[k].1] == 0 {
                    topo.push(es[k].1);
                }
            }
        }
    }
    // 初始分层：最长路径（汇点为 0，往上为负）
    let mut r = vec![0i32; n];
    for &v in topo.iter().rev() {
        r[v] = inc[v].iter().filter(|&&k| es[k].0 == v).map(|&k| r[es[k].1] - es[k].2).min().unwrap_or(0);
    }
    let slack = |r: &[i32], k: usize| r[es[k].1] - r[es[k].0] - es[k].2;

    // 连通分量
    let mut comp = vec![usize::MAX; n];
    let mut comps: Vec<Vec<usize>> = vec![];
    for s in 0..n {
        if comp[s] != usize::MAX {
            continue;
        }
        let c = comps.len();
        let mut list = vec![s];
        comp[s] = c;
        let mut j = 0;
        while j < list.len() {
            let v = list[j];
            j += 1;
            for &k in &inc[v] {
                let w = if es[k].0 == v { es[k].1 } else { es[k].0 };
                if comp[w] == usize::MAX {
                    comp[w] = c;
                    list.push(w);
                }
            }
        }
        comps.push(list);
    }

    let mut in_tree = vec![false; n];
    let mut tadj: Vec<Vec<usize>> = vec![vec![]; n];
    let (mut low, mut lim, mut parent) = (vec![0usize; n], vec![0usize; n], vec![usize::MAX; n]);
    let mut cut = vec![0i32; n]; // 子节点 → 它与父节点之间树边的切割值
    for cn in &comps {
        let ce: Vec<usize> = (0..es.len()).filter(|&k| comp[es[k].0] == comp[cn[0]]).collect();
        if ce.is_empty() {
            continue;
        }
        // 可行紧致生成树
        let root = cn[0];
        in_tree[root] = true;
        let mut tnodes = vec![root];
        loop {
            let mut stack = tnodes.clone();
            while let Some(v) = stack.pop() {
                for &k in &inc[v] {
                    let w = if es[k].0 == v { es[k].1 } else { es[k].0 };
                    if !in_tree[w] && slack(&r, k) == 0 {
                        in_tree[w] = true;
                        tnodes.push(w);
                        tadj[v].push(w);
                        tadj[w].push(v);
                        stack.push(w);
                    }
                }
            }
            if tnodes.len() >= cn.len() {
                break;
            }
            let Some(&k) = ce.iter().filter(|&&k| in_tree[es[k].0] != in_tree[es[k].1]).min_by_key(|&&k| slack(&r, k)) else { break };
            let d = if in_tree[es[k].0] { slack(&r, k) } else { -slack(&r, k) };
            for &t in &tnodes {
                r[t] += d;
            }
        }

        // 后序编号 low / lim：x 在以 v 为根的子树中 ⇔ low[v] ≤ lim[x] ≤ lim[v]
        fn low_lim(v: usize, p: usize, next: &mut usize, tadj: &[Vec<usize>], low: &mut [usize], lim: &mut [usize], parent: &mut [usize]) {
            let l = *next;
            for &w in &tadj[v] {
                if w != p {
                    low_lim(w, v, next, tadj, low, lim, parent);
                }
            }
            low[v] = l;
            lim[v] = *next;
            *next += 1;
            parent[v] = p;
        }
        let tree_edge = |u: usize, v: usize| idx.get(&(u, v)).map(|&k| (k, true)).or_else(|| idx.get(&(v, u)).map(|&k| (k, false)));
        let init = |low: &mut Vec<usize>, lim: &mut Vec<usize>, parent: &mut Vec<usize>, cut: &mut Vec<i32>, tadj: &Vec<Vec<usize>>| {
            let mut next = 1;
            low_lim(root, usize::MAX, &mut next, tadj, low, lim, parent);
            let mut post: Vec<usize> = cn.clone();
            post.sort_by_key(|&v| lim[v]);
            for &c in &post {
                let p = parent[c];
                if p == usize::MAX {
                    continue;
                }
                let Some((k, child_is_tail)) = tree_edge(c, p) else { continue };
                let mut cv = es[k].3;
                for &k2 in &inc[c] {
                    let is_out = es[k2].0 == c;
                    let other = if is_out { es[k2].1 } else { es[k2].0 };
                    if other == p {
                        continue;
                    }
                    let to_head = is_out == child_is_tail;
                    cv += if to_head { es[k2].3 } else { -es[k2].3 };
                    if parent[other] == c {
                        cv += if to_head { -cut[other] } else { cut[other] };
                    }
                }
                cut[c] = cv;
            }
        };
        init(&mut low, &mut lim, &mut parent, &mut cut, &tadj);
        // 反复把切割值为负的树边换成松弛最小的非树边
        for _ in 0..(ce.len() * 4 + 50) {
            let Some(&c) = cn.iter().find(|&&c| parent[c] != usize::MAX && cut[c] < 0) else { break };
            let p = parent[c];
            let Some((k, _)) = tree_edge(c, p) else { break };
            let (v, w) = (es[k].0, es[k].1);
            let (tail, flip) = if lim[v] > lim[w] { (w, true) } else { (v, false) };
            let desc = |x: usize| low[tail] <= lim[x] && lim[x] <= lim[tail];
            let Some(&f) = ce.iter().filter(|&&k| flip == desc(es[k].0) && flip != desc(es[k].1)).min_by_key(|&&k| slack(&r, k)) else { break };
            tadj[c].retain(|&x| x != p);
            tadj[p].retain(|&x| x != c);
            tadj[es[f].0].push(es[f].1);
            tadj[es[f].1].push(es[f].0);
            init(&mut low, &mut lim, &mut parent, &mut cut, &tadj);
            // 沿树从根往下重新推算层号
            let mut pre: Vec<usize> = cn.clone();
            pre.sort_by_key(|&v| std::cmp::Reverse(lim[v]));
            pre.sort_by_key(|&v| depth(v, &parent));
            for &v in &pre {
                let p = parent[v];
                if p == usize::MAX {
                    continue;
                }
                if let Some((k, v_is_tail)) = tree_edge(v, p) {
                    r[v] = if v_is_tail { r[p] - es[k].2 } else { r[p] + es[k].2 };
                }
            }
        }
        let m = cn.iter().map(|&v| r[v]).min().unwrap_or(0);
        for &v in cn {
            r[v] -= m;
        }
    }
    // 入边数 = 出边数的节点：下沉到紧挨下游的一层
    for &v in topo.iter().rev() {
        let (ins, outs) = inc[v].iter().fold((0, 0), |(i, o), &k| if es[k].0 == v { (i, o + es[k].3) } else { (i + es[k].3, o) });
        if ins > 0 && ins == outs {
            let max = inc[v].iter().filter(|&&k| es[k].0 == v).map(|&k| r[es[k].1] - es[k].2).min().unwrap();
            r[v] = r[v].max(max);
        }
    }
    r.into_iter().map(|x| x.max(0) as usize).collect()
}

/// 树中节点的深度（根为 0）。
fn depth(mut v: usize, parent: &[usize]) -> usize {
    let mut d = 0;
    while parent[v] != usize::MAX {
        v = parent[v];
        d += 1;
    }
    d
}

/// 相邻节点局部交换（transpose）：交换同层同子图的相邻两点以严格减少交叉；
/// 交叉数相同时，把回边虚拟节点推向所在一侧的外缘，避免回边切入分支之间。
fn transpose(layers: &mut [Vec<usize>], up: &[Vec<usize>], down: &[Vec<usize>], clusters: &[Option<usize>], is_back_dummy: &[bool], nv: usize) {
    let mut pos = vec![0usize; nv];
    for l in layers.iter() {
        for (i, &v) in l.iter().enumerate() {
            pos[v] = i;
        }
    }
    let pair_cross = |a: usize, b: usize, adj: &[Vec<usize>], pos: &[usize]| -> usize {
        let mut c = 0;
        for &na in &adj[a] {
            let pa = pos[na];
            for &nb in &adj[b] {
                if pa > pos[nb] {
                    c += 1;
                }
            }
        }
        c
    };
    for _ in 0..8 {
        let mut improved = false;
        for l in layers.iter_mut() {
            let len = l.len();
            if len < 2 {
                continue;
            }
            let mid = len / 2;
            for i in 0..len - 1 {
                let (a, b) = (l[i], l[i + 1]);
                if clusters[a] != clusters[b] {
                    continue;
                }
                let before = pair_cross(a, b, up, &pos) + pair_cross(a, b, down, &pos);
                let after = pair_cross(b, a, up, &pos) + pair_cross(b, a, down, &pos);
                let push_back_outer =
                    after == before && ((i < mid && !is_back_dummy[a] && is_back_dummy[b]) || (i + 1 >= mid && is_back_dummy[a] && !is_back_dummy[b]));
                if after < before || push_back_outer {
                    l.swap(i, i + 1);
                    pos[a] = i + 1;
                    pos[b] = i;
                    improved = true;
                }
            }
        }
        if !improved {
            break;
        }
    }
}

/// 后处理：1) 把跨多层长边的虚拟节点链尽量拉成笔直竖线；2) 在不破坏间距的前提下把实节点居中对齐到相邻节点。
#[allow(clippy::too_many_arguments)]
fn straighten_and_center(
    x: &mut [f32],
    layers: &[Vec<usize>],
    chains: &HashMap<usize, Vec<usize>>,
    reversed: &[bool],
    up: &[Vec<usize>],
    down: &[Vec<usize>],
    n: usize,
    gap: &dyn Fn(usize, usize) -> f32,
) {
    let nv = x.len();
    let mut loc = vec![(0usize, 0usize); nv];
    for (li, l) in layers.iter().enumerate() {
        for (i, &v) in l.iter().enumerate() {
            loc[v] = (li, i);
        }
    }
    let bounds = |v: usize, x: &[f32]| -> (f32, f32) {
        let (li, i) = loc[v];
        let l = &layers[li];
        let lo = if i > 0 { x[l[i - 1]] + gap(l[i - 1], v) } else { f32::NEG_INFINITY };
        let hi = if i + 1 < l.len() { x[l[i + 1]] - gap(v, l[i + 1]) } else { f32::INFINITY };
        (lo, hi)
    };
    let mut chain_keys: Vec<usize> = chains.keys().copied().collect();
    chain_keys.sort_unstable();
    for _ in 0..12 {
        // 1. 拉直跨层虚拟节点链
        for &k in &chain_keys {
            let ch = &chains[&k];
            if ch.len() < 3 {
                continue;
            }
            let dummies = &ch[1..ch.len() - 1];
            let mut lo_max = f32::NEG_INFINITY;
            let mut hi_min = f32::INFINITY;
            for &d in dummies {
                let (lo, hi) = bounds(d, x);
                lo_max = lo_max.max(lo);
                hi_min = hi_min.min(hi);
            }
            if lo_max <= hi_min {
                let target = if reversed[k] {
                    // 回边保持在它所在的外侧通道，避免向中间切入穿过节点
                    dummies.iter().map(|&d| x[d]).sum::<f32>() / dummies.len() as f32
                } else {
                    (x[ch[0]] + x[*ch.last().unwrap()]) * 0.5
                };
                let tx = target.clamp(lo_max, hi_min);
                for &d in dummies {
                    x[d] = tx;
                }
            } else {
                // 无法全直时做受限拉普拉斯平滑，消除锯齿抖动
                for _ in 0..4 {
                    for j in 1..ch.len() - 1 {
                        let d = ch[j];
                        let (lo, hi) = bounds(d, x);
                        if lo <= hi {
                            x[d] = ((x[ch[j - 1]] + x[ch[j + 1]]) * 0.5).clamp(lo, hi);
                        }
                    }
                }
            }
        }
        // 2. 在走廊允许范围内把实节点向直接邻居重心平滑居中（先下后上，确保单链起终点严格对齐）
        for pass in 0..2 {
            let iter: Box<dyn Iterator<Item = usize>> = if pass == 0 { Box::new(0..n) } else { Box::new((0..n).rev()) };
            for v in iter {
                let (lo, hi) = bounds(v, x);
                if lo > hi {
                    continue;
                }
                let up_avg = (!up[v].is_empty()).then(|| up[v].iter().map(|&u| x[u]).sum::<f32>() / up[v].len() as f32);
                let dn_avg = (!down[v].is_empty()).then(|| down[v].iter().map(|&w| x[w]).sum::<f32>() / down[v].len() as f32);
                let (target, alpha) = match (up_avg, dn_avg) {
                    (Some(a), Some(b)) => ((a + b) * 0.5, 0.7),
                    (Some(a), None) => (a, if up[v].len() == 1 { 1.0 } else { 0.7 }),
                    (None, Some(b)) => (b, if down[v].len() == 1 { 1.0 } else { 0.7 }),
                    (None, None) => continue,
                };
                let next = (x[v] + (target - x[v]) * alpha).clamp(lo, hi);
                x[v] = next;
            }
        }
    }
}

/// 相邻两层之间的连线交叉数。
fn crossings(layers: &[Vec<usize>], down: &[Vec<usize>], nv: usize) -> usize {
    let mut pos = vec![0usize; nv];
    for l in layers {
        for (i, &v) in l.iter().enumerate() {
            pos[v] = i;
        }
    }
    let mut total = 0;
    for l in layers {
        let mut es: Vec<(usize, usize)> = l.iter().flat_map(|&u| down[u].iter().map(move |&w| (u, w))).map(|(u, w)| (pos[u], pos[w])).collect();
        es.sort();
        for i in 0..es.len() {
            for j in i + 1..es.len() {
                if es[i].0 < es[j].0 && es[i].1 > es[j].1 {
                    total += 1;
                }
            }
        }
    }
    total
}

/// Brandes-Köpf 横坐标分配（移植自 dagre `position/bk.js`）：上下 × 左右四种对齐各算一遍，
/// 取每个节点四个坐标的中间两个的平均值。`n` 之后的顶点是虚拟节点，`gap(a, b)` 为相邻两点中心的最小距离。
fn brandes_kopf(layers: &[Vec<usize>], up: &[Vec<usize>], down: &[Vec<usize>], w: &[f32], n: usize, gap: &dyn Fn(usize, usize) -> f32) -> Vec<f32> {
    let nv = w.len();
    let dummy = |v: usize| v >= n;
    let key = |a: usize, b: usize| (a.min(b), a.max(b));
    let mut order = vec![0usize; nv];
    for l in layers {
        for (i, &v) in l.iter().enumerate() {
            order[v] = i;
        }
    }
    // 一类冲突：普通边与「虚拟—虚拟」内段交叉时，放弃对齐普通边，保证长边笔直
    let mut conflicts: std::collections::HashSet<(usize, usize)> = Default::default();
    for li in 1..layers.len() {
        let (prev, layer) = (&layers[li - 1], &layers[li]);
        let Some(&last) = layer.last() else { continue };
        let (mut k0, mut scan) = (0usize, 0usize);
        for (i, &v) in layer.iter().enumerate() {
            let inner = if dummy(v) { up[v].iter().copied().find(|&u| dummy(u)) } else { None };
            let k1 = inner.map(|u| order[u]).unwrap_or(prev.len());
            if inner.is_some() || v == last {
                for &s in &layer[scan..=i] {
                    for &u in &up[s] {
                        let p = order[u];
                        if (p < k0 || k1 < p) && !(dummy(u) && dummy(s)) {
                            conflicts.insert(key(u, s));
                        }
                    }
                }
                scan = i + 1;
                k0 = k1;
            }
        }
    }

    let mut xss: Vec<Vec<f32>> = vec![];
    for top in [true, false] {
        for right in [false, true] {
            let mut lay: Vec<Vec<usize>> = if top { layers.to_vec() } else { layers.iter().rev().cloned().collect() };
            if right {
                for l in lay.iter_mut() {
                    l.reverse();
                }
            }
            let nb = if top { up } else { down };
            // 纵向对齐：每个节点尽量与中位数邻居连成一个块
            let mut root: Vec<usize> = (0..nv).collect();
            let mut align = root.clone();
            let mut pos = vec![0usize; nv];
            for l in &lay {
                for (i, &v) in l.iter().enumerate() {
                    pos[v] = i;
                }
            }
            for l in &lay {
                let mut prev: isize = -1;
                for &v in l {
                    let mut ws = nb[v].clone();
                    if ws.is_empty() {
                        continue;
                    }
                    ws.sort_by_key(|&u| pos[u]);
                    let mp = (ws.len() - 1) as f32 / 2.;
                    for i in mp.floor() as usize..=mp.ceil() as usize {
                        let u = ws[i];
                        if align[v] == v && prev < pos[u] as isize && !conflicts.contains(&key(v, u)) {
                            align[u] = v;
                            root[v] = root[u];
                            align[v] = root[v];
                            prev = pos[u] as isize;
                        }
                    }
                }
            }
            // 横向压缩：块之间按层内相邻关系建约束图，先尽量靠左，再把多余空隙往右收
            let mut bedge: HashMap<(usize, usize), f32> = HashMap::new();
            for l in &lay {
                for p in l.windows(2) {
                    let e = bedge.entry((root[p[0]], root[p[1]])).or_insert(0.);
                    *e = e.max(gap(p[0], p[1]));
                }
            }
            let mut succ: Vec<Vec<(usize, f32)>> = vec![vec![]; nv];
            let mut pred: Vec<Vec<(usize, f32)>> = vec![vec![]; nv];
            let mut indeg = vec![0usize; nv];
            for (&(a, b), &d) in &bedge {
                succ[a].push((b, d));
                pred[b].push((a, d));
                indeg[b] += 1;
            }
            let roots: Vec<usize> = (0..nv).filter(|&v| root[v] == v).collect();
            let mut topo: Vec<usize> = roots.iter().copied().filter(|&v| indeg[v] == 0).collect();
            let mut i = 0;
            while i < topo.len() {
                let v = topo[i];
                i += 1;
                for &(b, _) in &succ[v] {
                    indeg[b] -= 1;
                    if indeg[b] == 0 {
                        topo.push(b);
                    }
                }
            }
            let mut xs = vec![0f32; nv];
            for &b in &topo {
                xs[b] = pred[b].iter().map(|&(a, d)| xs[a] + d).fold(0., f32::max);
            }
            for &b in topo.iter().rev() {
                let min = succ[b].iter().map(|&(c, d)| xs[c] - d).fold(f32::INFINITY, f32::min);
                if min.is_finite() {
                    xs[b] = xs[b].max(min);
                }
            }
            let mut x: Vec<f32> = (0..nv).map(|v| xs[root[v]]).collect();
            if right {
                for v in x.iter_mut() {
                    *v = -*v;
                }
            }
            xss.push(x);
        }
    }
    // 以总宽度最小的一种为基准对齐另外三种：靠左的对齐左边界，靠右的对齐右边界（含节点半宽）
    let extent = |xs: &[f32]| -> (f32, f32) {
        let lo = (0..nv).map(|v| xs[v] - w[v] / 2.).fold(f32::INFINITY, f32::min);
        let hi = (0..nv).map(|v| xs[v] + w[v] / 2.).fold(f32::NEG_INFINITY, f32::max);
        (lo, hi)
    };
    let best = (0..4)
        .min_by(|&a, &b| {
            let (la, ha) = extent(&xss[a]);
            let (lb, hb) = extent(&xss[b]);
            (ha - la).partial_cmp(&(hb - lb)).unwrap()
        })
        .unwrap();
    let (bmin, bmax) = extent(&xss[best]);
    for (k, xs) in xss.iter_mut().enumerate() {
        if k == best {
            continue;
        }
        let (lo, hi) = extent(xs);
        let d = if k % 2 == 0 { bmin - lo } else { bmax - hi };
        for v in xs.iter_mut() {
            *v += d;
        }
    }
    let mut x: Vec<f32> = (0..nv)
        .map(|v| {
            let mut c = [xss[0][v], xss[1][v], xss[2][v], xss[3][v]];
            c.sort_by(|a, b| a.partial_cmp(b).unwrap());
            (c[1] + c[2]) / 2.
        })
        .collect();
    // 取平均后偶尔会挤得太近：按层从左到右补足间距
    for l in layers {
        for i in 1..l.len() {
            let min = x[l[i - 1]] + gap(l[i - 1], l[i]);
            if x[l[i]] < min {
                x[l[i]] = min;
            }
        }
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layered_and_acyclic() {
        let nodes = vec![LNode { w: 80., h: 40., cluster: None }; 4];
        let edges = vec![
            LEdge { from: 0, to: 1, minlen: 1, label_w: 0. },
            LEdge { from: 1, to: 2, minlen: 1, label_w: 0. },
            LEdge { from: 2, to: 0, minlen: 1, label_w: 0. }, // 环
            LEdge { from: 0, to: 3, minlen: 1, label_w: 0. },
        ];
        let l = layout(&nodes, &edges, Dir::TB, &Spacing::default());
        assert!(l.pos[1].y > l.pos[0].y);
        assert!(l.pos[2].y > l.pos[1].y);
        // 节点不重叠
        for i in 0..4 {
            for j in i + 1..4 {
                let (a, b) = (l.pos[i], l.pos[j]);
                assert!((a.x - b.x).abs() >= 80. || (a.y - b.y).abs() >= 40., "{i} {j} overlap");
            }
        }
        assert_eq!(l.edges.len(), 4);
        // 回边的折线方向与原始方向一致：终点在节点 0
        assert!(l.edges[2].last().unwrap().dist(l.pos[0]) < 0.1);
    }

    #[test]
    fn hundred_nodes_fast() {
        let nodes = vec![LNode { w: 90., h: 40., cluster: None }; 100];
        let edges: Vec<LEdge> = (1..100).map(|i| LEdge { from: (i - 1) / 3, to: i, minlen: 1, label_w: 0. }).collect();
        let t = std::time::Instant::now();
        let _ = layout(&nodes, &edges, Dir::LR, &Spacing::default());
        assert!(t.elapsed().as_millis() < 50, "{:?}", t.elapsed());
    }
}
