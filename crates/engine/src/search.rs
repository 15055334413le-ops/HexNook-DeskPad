//! 搜索排序（纯函数，无 Win32 依赖；拼音首字母由 keys.rs 预先算好放进 SearchKeys）。
use crate::model::{AppEntry, AppId};
use std::collections::HashMap;

/// 使用记录（usage.json），由引擎维护，search 只读。
#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Usage {
    /// 每个应用经 DeskPad 启动的次数。
    pub launch_counts: HashMap<AppId, u32>,
    /// 规范化查询串 → 用户上一次从该查询启动的应用。
    pub query_picks: HashMap<String, AppId>,
}

/// 查询规范化：trim、小写、去掉所有空白。
pub fn normalize_query(q: &str) -> String {
    q.trim().chars().filter(|c| !c.is_whitespace()).flat_map(|c| c.to_lowercase()).collect()
}

/// 词首位置（char 下标）：0、非字母数字之后、小写→大写的驼峰处、字母↔数字切换处、每个 CJK 字符。
pub fn word_starts(name: &str) -> Vec<usize> {
    let cs: Vec<char> = name.chars().collect();
    let mut v = Vec::new();
    for i in 0..cs.len() {
        let c = cs[i];
        if !c.is_alphanumeric() { continue; }
        let start = if i == 0 { true } else {
            let p = cs[i - 1];
            !p.is_alphanumeric()
                || (p.is_lowercase() && c.is_uppercase())
                || (p.is_ascii_digit() != c.is_ascii_digit())
                || is_cjk(c) || is_cjk(p)
        };
        if start { v.push(i); }
    }
    v
}

pub fn is_cjk(c: char) -> bool { ('\u{4E00}'..='\u{9FFF}').contains(&c) }

fn is_subsequence(q: &str, s: &str) -> Option<usize> {
    let mut it = s.chars().enumerate();
    let mut first = None;
    for qc in q.chars() {
        loop {
            match it.next() {
                Some((i, sc)) if sc == qc => { if first.is_none() { first = Some(i); } break; }
                Some(_) => continue,
                None => return None,
            }
        }
    }
    first
}

/// 单个应用对查询的得分；0 = 不匹配。q 必须已经过 normalize_query。
pub fn score(app: &AppEntry, q: &str) -> u32 {
    let k = &app.keys;
    let name_nospace: String = k.name_lower.chars().filter(|c| !c.is_whitespace()).collect();
    if k.name_lower == q || name_nospace == q { return 120; }
    if k.name_lower.starts_with(q) || name_nospace.starts_with(q) { return 100; }
    let lower: Vec<char> = k.name_lower.chars().collect();
    let starts = word_starts(&app.name);
    for &i in &starts {
        let tail: String = lower[i.min(lower.len())..].iter().collect();
        if tail.starts_with(q) { return 85; }
    }
    if k.initials.starts_with(q) { return 75; }
    if k.name_lower.contains(q) { return 60; }
    if !k.alias.is_empty() && k.alias.starts_with(q) { return 55; }
    if k.initials.contains(q) { return 45; }
    if !k.alias.is_empty() && k.alias.contains(q) { return 40; }
    if let Some(first) = is_subsequence(q, &k.name_lower) {
        return if starts.contains(&first) { 30 } else { 20 };
    }
    0
}

/// 返回 apps 的下标，按相关度降序。空查询返回 0..len（apps 已按 sort_key 排好）。
pub fn search(apps: &[AppEntry], usage: &Usage, raw_q: &str) -> Vec<usize> {
    let q = normalize_query(raw_q);
    if q.is_empty() { return (0..apps.len()).collect(); }
    let pick = usage.query_picks.get(&q).copied();
    let mut hits: Vec<(u32, u32, usize, usize)> = Vec::new(); // (score, count, name_len, idx)
    for (idx, a) in apps.iter().enumerate() {
        let mut s = score(a, &q);
        if s == 0 { continue; }
        if pick == Some(a.id) { s += 1000; }
        let cnt = usage.launch_counts.get(&a.id).copied().unwrap_or(0);
        hits.push((s, cnt, a.name.chars().count(), idx));
    }
    hits.sort_by(|x, y| y.0.cmp(&x.0).then(y.1.cmp(&x.1)).then(x.2.cmp(&y.2))
        .then(apps[x.3].keys.sort_key.cmp(&apps[y.3].keys.sort_key)));
    hits.into_iter().map(|h| h.3).collect()
}
