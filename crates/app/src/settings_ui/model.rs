//! 设置窗口的分区与行模型（UI 文档 6.11）。纯函数，不碰 Win32，可单测。
//!
//! 行高 48 DIP：左侧标题 + 一行说明（TEXT_SECONDARY），右侧控件。
use deskpad_engine::model::{HotkeyPreset, IconStyle, PadSize, Settings, SortMode, ViewMode, VisualStyle};

/// 窗口主体（DIP，不含阴影边距）。
pub const W: f32 = 680.0;
pub const H: f32 = 500.0;
pub const NAV_W: f32 = 160.0;
pub const HEADER_H: f32 = 48.0;
/// 内容区左右边距。
pub const CONTENT_X: f32 = NAV_W + 24.0;
pub const CONTENT_W: f32 = W - CONTENT_X - 24.0;
/// 行高与行间距。
pub const ROW_H: f32 = 48.0;
/// 分组标题高度。
pub const GROUP_H: f32 = 32.0;

/// 五个分区。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    General,
    Appearance,
    Apps,
    Update,
    About,
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::General,
        Section::Appearance,
        Section::Apps,
        Section::Update,
        Section::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::General => "通用",
            Section::Appearance => "外观",
            Section::Apps => "应用",
            Section::Update => "更新",
            Section::About => "关于",
        }
    }

    /// --settings 参数（中英文均可）。
    pub fn parse(s: &str) -> Option<Section> {
        Some(match s {
            "通用" | "general" => Section::General,
            "外观" | "appearance" => Section::Appearance,
            "应用" | "apps" => Section::Apps,
            "更新" | "update" => Section::Update,
            "关于" | "about" => Section::About,
            _ => return None,
        })
    }

    pub fn slug(self) -> &'static str {
        match self {
            Section::General => "general",
            Section::Appearance => "appearance",
            Section::Apps => "apps",
            Section::Update => "update",
            Section::About => "about",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&s| s == self).unwrap_or(0)
    }
}

/// 右侧控件种类。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Control {
    /// 分段按钮：labels 是选项（选中项下标在 Row 构建时求值）。
    Seg(&'static [&'static str]),
    Toggle,
    /// 次要按钮（一个动作）。
    Btn(&'static str),
    /// 步进器（最近文件数量 1..=20）。
    Stepper,
    /// 只有文字，无控件（版本号、状态、链接等）。
    Text,
}

/// 一行设置（或一个分组标题）。
#[derive(Clone, Copy, Debug)]
pub enum Row {
    /// 分组标题（TEXT_SECONDARY，12px）。
    Group(&'static str),
    /// 设置行：标题 + 说明 + 控件。id 用来求选中态与命中。
    Item {
        id: ItemId,
        title: &'static str,
        desc: &'static str,
        ctl: Control,
    },
}

/// 各设置行的稳定 id（命中测试、焦点、即时写设置用）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ItemId {
    Hotkey,
    Autostart,
    ReplayOnboard,
    VisualStyle,
    PadSize,
    IconStyle,
    FilesView,
    DirsView,
    RecentCount,
    ShowSystem,
    SortMode,
    AddScanDir,
    AddManualApp,
    CheckUpdate,
    AutoUpdate,
    Version,
    Releases,
    OpenLogs,
    // 列表类行在窗口侧动态生成（隐藏应用 / 扫描目录 / 手动应用），不进本表。
}

/// 一个分区的行（静态部分；列表行动态追加）。
pub fn rows(sec: Section) -> Vec<Row> {
    use ItemId::*;
    match sec {
        Section::General => vec![
            Row::Item {
                id: Hotkey,
                title: "快捷键",
                desc: "Alt + Space 会替代窗口左上角菜单快捷键",
                ctl: Control::Seg(&["Alt + Space", "Ctrl + Alt + Space", "Alt + `"]),
            },
            Row::Item {
                id: Autostart,
                title: "开机自启",
                desc: "登录 Windows 后自动驻留托盘",
                ctl: Control::Toggle,
            },
            Row::Group("其他"),
            Row::Item {
                id: ReplayOnboard,
                title: "首次引导",
                desc: "重新显示角标旁的引导气泡",
                ctl: Control::Btn("重新显示"),
            },
        ],
        Section::Appearance => vec![
            Row::Item {
                id: VisualStyle,
                title: "主题",
                desc: "启动台主体的视觉风格",
                ctl: Control::Seg(&["扁平", "毛玻璃"]),
            },
            Row::Item {
                id: PadSize,
                title: "启动台大小",
                desc: "窗口或占满整块屏幕",
                ctl: Control::Seg(&["窗口", "全屏"]),
            },
            Row::Item {
                id: IconStyle,
                title: "图标样式",
                desc: "应用图标保留原样或统一圆角",
                ctl: Control::Seg(&["原生", "统一圆角"]),
            },
            Row::Item {
                id: FilesView,
                title: "最近文件视图",
                desc: "最近文件的默认展示方式",
                ctl: Control::Seg(&["图标", "列表"]),
            },
            Row::Item {
                id: DirsView,
                title: "常用文件夹视图",
                desc: "常用文件夹的默认展示方式",
                ctl: Control::Seg(&["图标", "列表"]),
            },
            Row::Group("数量"),
            Row::Item {
                id: RecentCount,
                title: "最近文件数量",
                desc: "启动台「最近文件」里最多显示几条",
                ctl: Control::Stepper,
            },
        ],
        Section::Apps => vec![
            Row::Item {
                id: ShowSystem,
                title: "显示系统工具",
                desc: "在应用列表里显示管理 / 维护工具",
                ctl: Control::Toggle,
            },
            Row::Item {
                id: SortMode,
                title: "排序",
                desc: "应用列表的排列顺序",
                ctl: Control::Seg(&["名称", "使用频率"]),
            },
            // 隐藏应用 / 额外扫描目录 / 手动应用三组列表由窗口侧追加。
        ],
        Section::Update => vec![
            Row::Item {
                id: Version,
                title: "当前版本",
                desc: "本机安装的 HN DeskPad 版本",
                ctl: Control::Text,
            },
            Row::Item {
                id: CheckUpdate,
                title: "更新",
                desc: "状态显示在这里；检查或立即更新",
                ctl: Control::Btn("检查更新"),
            },
            Row::Item {
                id: AutoUpdate,
                title: "自动检查更新",
                desc: "打开启动台时检查（距上次 ≥24 小时）",
                ctl: Control::Toggle,
            },
        ],
        Section::About => vec![
            Row::Item {
                id: Version,
                title: "HN DeskPad",
                desc: "桌面启动台",
                ctl: Control::Text,
            },
            Row::Item {
                id: Releases,
                title: "发布页",
                desc: "在浏览器中打开发布仓库",
                ctl: Control::Btn("打开发布页"),
            },
            Row::Item {
                id: OpenLogs,
                title: "日志",
                desc: "打开 %LocalAppData%\\DeskPad 日志文件夹",
                ctl: Control::Btn("打开日志文件夹"),
            },
        ],
    }
}

// ---- 选中态求值（从 Settings 推出各分段控件的下标）----

pub fn hotkey_idx(s: &Settings) -> usize {
    HotkeyPreset::ALL.iter().position(|&p| p == s.hotkey).unwrap_or(0)
}
pub fn visual_style_idx(s: &Settings) -> usize {
    match s.visual_style {
        VisualStyle::Flat => 0,
        VisualStyle::Glass => 1,
    }
}
pub fn pad_size_idx(s: &Settings) -> usize {
    match s.pad_size {
        PadSize::Window => 0,
        PadSize::Fullscreen => 1,
    }
}
pub fn icon_style_idx(s: &Settings) -> usize {
    match s.icon_style {
        IconStyle::Native => 0,
        IconStyle::Unified => 1,
    }
}
pub fn sort_mode_idx(s: &Settings) -> usize {
    match s.sort_mode {
        SortMode::Name => 0,
        SortMode::Frequency => 1,
    }
}

pub fn view_mode_idx(m: ViewMode) -> usize {
    match m {
        ViewMode::Grid => 0,
        ViewMode::List => 1,
    }
}

pub fn idx_view_mode(i: usize) -> ViewMode {
    if i == 0 { ViewMode::Grid } else { ViewMode::List }
}

#[cfg(test)]
mod t {
    use super::*;

    #[test]
    fn section_parse() {
        assert_eq!(Section::parse("通用"), Some(Section::General));
        assert_eq!(Section::parse("apps"), Some(Section::Apps));
        assert_eq!(Section::parse("关于"), Some(Section::About));
        assert_eq!(Section::parse("xx"), None);
    }

    #[test]
    fn rows_have_controls() {
        for sec in Section::ALL {
            let rs = rows(sec);
            assert!(!rs.is_empty());
            for r in rs {
                if let Row::Item { ctl, .. } = r {
                    if let Control::Seg(labels) = ctl {
                        assert!(labels.len() >= 2);
                    }
                }
            }
        }
    }

    #[test]
    fn hotkey_row_explains_system_menu_conflict() {
        let desc = rows(Section::General)
            .into_iter()
            .find_map(|row| match row {
                Row::Item {
                    id: ItemId::Hotkey,
                    desc,
                    ..
                } => Some(desc),
                _ => None,
            })
            .expect("general section must contain the hotkey row");
        assert!(desc.contains("Alt + Space"));
        assert!(desc.contains("窗口左上角菜单"));
    }

    #[test]
    fn original_card_rows_are_present_in_all_sections() {
        fn item_ids(sec: Section) -> Vec<ItemId> {
            rows(sec)
                .into_iter()
                .filter_map(|row| match row {
                    Row::Item { id, .. } => Some(id),
                    Row::Group(_) => None,
                })
                .collect()
        }

        assert_eq!(
            Section::ALL.map(Section::label),
            ["通用", "外观", "应用", "更新", "关于"]
        );
        assert_eq!(
            item_ids(Section::General),
            [ItemId::Hotkey, ItemId::Autostart, ItemId::ReplayOnboard]
        );
        assert_eq!(
            item_ids(Section::Appearance),
            [
                ItemId::VisualStyle,
                ItemId::PadSize,
                ItemId::IconStyle,
                ItemId::FilesView,
                ItemId::DirsView,
                ItemId::RecentCount,
            ]
        );
        assert_eq!(
            item_ids(Section::Apps),
            [ItemId::ShowSystem, ItemId::SortMode]
        );
        assert_eq!(
            item_ids(Section::Update),
            [
                ItemId::Version,
                ItemId::CheckUpdate,
                ItemId::AutoUpdate,
            ]
        );
        assert_eq!(
            item_ids(Section::About),
            [ItemId::Version, ItemId::Releases, ItemId::OpenLogs]
        );
    }
}
