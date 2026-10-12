//! SearchKeys 计算：拼音首字母、alias、sort_key、FNV-1a（engine 文档 4.1.3 / 4.1.4）。
use crate::model::SearchKeys;
use crate::search::{is_cjk, word_starts};

/// FNV-1a 64 位哈希（offset basis 0xcbf29ce484222325，prime 0x100000001b3）。
/// 禁止使用 DefaultHasher——它的算法不保证跨 Rust 版本稳定。
pub(crate) fn fnv1a64(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// GB2312 一级汉字按拼音排序，用区位码边界就能得到首字母。
const BOUNDS: [(u16, char); 23] = [
    (0xB0A1,'a'),(0xB0C5,'b'),(0xB2C1,'c'),(0xB4EE,'d'),(0xB6EA,'e'),(0xB7A2,'f'),
    (0xB8C1,'g'),(0xB9FE,'h'),(0xBBF7,'j'),(0xBFA6,'k'),(0xC0AC,'l'),(0xC2E8,'m'),
    (0xC4C3,'n'),(0xC5B6,'o'),(0xC5BE,'p'),(0xC6DA,'q'),(0xC8BB,'r'),(0xC8F6,'s'),
    (0xCBFA,'t'),(0xCDDA,'w'),(0xCEF4,'x'),(0xD1B9,'y'),(0xD4D1,'z'),
];
/// 一级字表之外、但常见于软件名的字。发现新的可以往这里追加（需在 questions.md 记一笔）。
const SUPPLEMENT: [(char, char); 13] = [
    ('哔','b'),('咔','k'),('咚','d'),('咪','m'),('喵','m'),('嗨','h'),('昕','x'),
    ('浏','l'),('穹','q'),('蜓','t'),('蜻','q'),('逍','x'),('呱','g'),
];

pub fn hanzi_initial(c: char) -> Option<char> {
    if let Some(&(_, l)) = SUPPLEMENT.iter().find(|(h, _)| *h == c) { return Some(l); }
    let mut w = [0u16; 2];
    let w = c.encode_utf16(&mut w);
    let mut buf = [0u8; 4];
    // SAFETY: 输入输出缓冲区都由本函数持有，长度正确。
    let n = unsafe {
        windows::Win32::Globalization::WideCharToMultiByte(
            936,
            0,
            w,
            Some(&mut buf),
            windows::core::PCSTR::null(),
            None,
        )
    };
    if n != 2 {
        return None;
    }
    let code = (buf[0] as u16) << 8 | buf[1] as u16;
    if !(0xB0A1..=0xD7F9).contains(&code) {
        return None; // 只覆盖一级汉字
    }
    BOUNDS.iter().rev().find(|(start, _)| code >= *start).map(|&(_, l)| l)
}

/// alias（exe 名）从 AppsFolder 解析名推导（4.1.4）：
/// 1. 含 `!`（UWP AUMID）：! 前部分 → 第一个 _ 前 → 最后一个 . 后；
/// 2. 含 `\` 或 `/`：最后一段文件名去扩展名；
/// 3. 其他：按 . 切分，丢掉纯数字段和 exe、microsoft，取剩下的最后一段。
///
/// 结果一律小写。
pub(crate) fn alias_from_parsing(parsing: &str) -> String {
    let raw: String = if let Some(pos) = parsing.find('!') {
        let head = &parsing[..pos];
        let head = head.split('_').next().unwrap_or(head);
        head.rsplit('.').next().unwrap_or(head).to_string()
    } else if parsing.contains('\\') || parsing.contains('/') {
        let last = parsing.rsplit(['\\', '/']).next().unwrap_or(parsing);
        file_stem_lower(last)
    } else {
        parsing
            .split('.')
            .rfind(|s| {
                !s.chars().all(|c| c.is_ascii_digit())
                    && !s.eq_ignore_ascii_case("exe")
                    && !s.eq_ignore_ascii_case("microsoft")
            })
            .unwrap_or("")
            .to_string()
    };
    raw.to_lowercase()
}

/// 来源 B 的 .lnk：目标 exe 文件名去扩展名。
pub(crate) fn alias_from_target(target: &str) -> String {
    if target.is_empty() {
        return String::new();
    }
    file_stem_lower(target)
}

fn file_stem_lower(s: &str) -> String {
    std::path::Path::new(s)
        .file_stem()
        .map(|x| x.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| s.to_lowercase())
}

/// initials：词首字符 → ASCII 字母/数字取小写，汉字取拼音首字母，其他跳过（4.1.4）。
pub(crate) fn initials(name: &str) -> String {
    let mut out = String::new();
    for &i in &word_starts(name) {
        let Some(c) = name.chars().nth(i) else { continue };
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if is_cjk(c) {
            if let Some(l) = hanzi_initial(c) {
                out.push(l);
            }
        }
    }
    out
}

/// sort_key：每个汉字替换为拼音首字母（取不到保留原字），其余字符转小写。
pub(crate) fn sort_key(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if is_cjk(c) {
            out.push(hanzi_initial(c).unwrap_or(c));
        } else {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
        }
    }
    out
}

pub(crate) fn compute(name: &str, alias: &str) -> SearchKeys {
    SearchKeys {
        name_lower: name.to_lowercase(),
        initials: initials(name),
        alias: alias.to_lowercase(),
        sort_key: sort_key(name),
    }
}

#[cfg(test)]
#[cfg(windows)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a64_empty() {
        assert_eq!(fnv1a64(b""), 0xcbf29ce484222325);
    }

    #[test]
    fn initials_basic() {
        assert_eq!(initials("微信"), "wx");
        assert_eq!(initials("网易云音乐"), "wyyyl");
        assert_eq!(initials("哔哩哔哩"), "blbl");
        assert_eq!(initials("腾讯会议"), "txhy");
        assert_eq!(initials("Visual Studio Code"), "vsc");
        assert_eq!(initials("PowerShell"), "ps");
        assert_eq!(initials("QQ音乐"), "qyl");
    }

    #[test]
    fn sort_key_mixed() {
        // 「微信」应排在 w 区
        assert_eq!(sort_key("微信"), "wx");
        assert_eq!(sort_key("Adobe Photoshop"), "adobe photoshop");
    }

    #[test]
    fn alias_rules() {
        assert_eq!(alias_from_parsing("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"), "windowscalculator");
        assert_eq!(alias_from_parsing("{6D809377-6F57-43CA-BC1F-99B095B86B59}\\Adobe\\Photoshop.exe"), "photoshop");
        assert_eq!(alias_from_parsing("Microsoft.Office.WINWORD.EXE.15"), "winword");
        assert_eq!(alias_from_target("C:\\Tools\\7zFM.exe"), "7zfm");
        assert_eq!(alias_from_target(""), "");
    }
}
