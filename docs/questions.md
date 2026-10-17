# questions.md

## 2026-10-05 · 第 5 节（依赖） · GetLocalTime 所需 feature 不在清单中

问题：4.9 要求日志时间用 `GetLocalTime` 取本地时间，但已对照 windows 0.58.0 源码确认：`GetLocalTime` / `SYSTEMTIME` 位于 `Win32_System_SystemInformation`，而第 5 节「原样照抄」的 feature 清单没有这一项，照抄清单无法编译。

建议方案：在 crates/engine/Cargo.toml 的 windows features 里追加 `Win32_System_SystemInformation` 与 `Win32_System_Time`（仅此两项，其余保持原清单；后者用于 probe recent 按 4.10 要求打印「mtime(本地时间)」，需要 FileTimeToSystemTime / SystemTimeToTzSpecificLocalTime）。UI 板块如需本地时间格式化同样需要追加。

## 2026-10-05 · 第 6 节 E2 验收 · 本机不存在「计算器」应用

问题：E2 验收要求 `probe apps` 包含「计算器」（UWP）。但本机（已用 `Get-StartApps` 独立核实）开始菜单里没有任何「计算器/Calculator」条目，只有「计算机管理」。扫描与过滤逻辑本身正常（照片/终端/Edge/设置等 UWP 与 Win32 项均在，共 165 项）。

建议方案：视为环境差异跳过该项；其余验收（count ≥ 30、含设置与 Microsoft Edge、无卸载项、两次运行输出一致）全部满足。若用户需要，可从 Store 安装计算器后重跑验收。


## 2026-10-05（UI agent · 第二批）

2. 2026-10-05 | UI 文档 6.2 | `RegisterWindowMessageW("HexNook.DeskPad.ShowPad")` 跨进程取值不一致：应用进程注册得到 0xC3E4，PowerShell 测试进程对同一字符串得到 0xC35A（均为 0xC000+ 区段、posted=True 但应用收不到）。原因未查明 | 二次启动唤起改用固定值 `WM_APP+3`（app.rs 的 WM_APP_SHOWPAD），只发给本应用唯一的 DeskPadMain 窗口，无冲突面。若坚持注册消息，建议改为命名事件 + PostMessage(WM_NULL) 方案
3. 2026-10-05 | UI 文档 §8 | D2D 结构体在 windows 0.58 改名（D2D1_RECT_F→D2D_RECT_F 等）之外，`CreateSolidColorBrush` 被 `Foundation_Numerics` feature 门控，清单缺该项 | 已补（同前一条记录第 2 条）
4. 2026-10-05 | UI 文档 §0.4 | 并行开发注意：本文件与引擎文件被两个 agent 交替写入，apps.rs/log.rs 等曾出现临时编译错误阻塞对方构建。建议：谁破坏谁修复，或约定提交前先 `cargo build -p <自己的crate>`

## 2026-10-05（UI agent · 第三批）

5. 2026-10-05 | UI 文档 6.7 | render 坐标 bug 待修（有完整证据）：无头渲染帧（docs/u2-frame-current.png）显示 ①右侧文件/文件夹卡只有文字无白色面板填充 ②搜索框位置偏移约 +130 DIP 且宽度异常 ③窗口左上角有一块错位的白色圆角碎片 ④布局日志数值全部正确（content 1120x720、apps_w 784、files (1093,149)），说明 layout.rs 正确、draw_pad_inner 的坐标换算有 bug | 下一轮优先修 draw_pad_inner 的 win_rect/offset 数学；修好后用 `DeskPad.exe --render-test out.bmp` 回归
6. 2026-10-05 | UI 文档 §0.4 | 尚有 ~19 个 warning，clippy -D warnings 未达标 | 下轮统一清理
7. 2026-10-05 | 协作 | 引擎 agent 已实现 E1-E4（probe apps/recent/search/icons 全通），本 agent 的 E0 桩已被真实实现覆盖，接口契约无变更 | 无需动作

8. 2026-10-05 | UI 文档 6.7 | 像素级测量（1752x1152 渲染帧）：白色面板出现在 x=1..1174px（=0..783 DIP），而 apps_card 窗口相对位置应为 24..808 DIP（=36..1212px）——即卡片绘制整体左移约 24 DIP（=MARGIN），且文件/文件夹卡（应为 1200..1680px）完全无白色填充。搜索框蓝色边框在 y=40..76 DIP 行无像素。 suspicion：win_rect 的 +MARGIN 在实际绘制中未生效（或被后续改动覆盖），加上文件/目录卡的填充/描边调用可能整体缺失 | 下轮：在 draw_pad_inner 的三卡循环与搜索框绘制处打断点/加日志核对实际传入 rect；修完后 --render-test 回归

9. 2026-10-05 | UI 文档 6.7 | 标定实验结论：`FillRectangle` 在 DIP(500,300) 画的红块精确落在 px(750,450)-(899,599)（=DIP×1.5、原点(0,0)）——**RT 坐标映射完全正确**。裁剪图（docs/u2-frame-current.png 同帧）显示真正的问题：① 搜索框画在 y≈0（应在 y=40 DIP）且 x 偏右；② 应用卡白色面板画在 (116..783 DIP)（应在 24..808）；③ 文件/文件夹卡只有文字、无白色面板；④ 左上角有 SELECT_BG 色圆角碎片（疑似选中格/气泡画错位置）。所有元素偏移方向和幅度不一致 → 不是全局变换，是 draw_pad_inner 各段的 rect 换算各自有错 | 下轮：逐段核对 win_rect 与各段局部 RectF 的换算（注意 search/grid/cell 的 x/y 来自 l.* 减 content.x 后 +MARGIN，但卡片本体走 win_rect——两者应对齐）；有 --render-test 工具秒级回归

10. 2026-10-05 | UI 文档 6.2/6.5 | 【严重 bug 已修复】用户真机被锁死：全屏遮罩永久残留吃掉所有鼠标输入。根因链：SetForegroundWindow 被前台锁拒绝 → pad 收到 WA_INACTIVE → hide() 的 `if !pad_visible { return }` 守卫在显示序列竞态下跳过收尾 → 遮罩残留且其点击关闭路径同样被守卫挡住 → 死锁。修复三处：① pad_visible 在动窗口前置位 ② hide() 去掉守卫改无条件收尾 ③ SetForegroundWindow 前加 AttachThreadInput（微软文档方案，需 +Win32_System_Threading feature）。已验证 show/失活/再唤起全链路日志正确
11. 2026-10-05 | UI 文档 6.2 | 二次启动语义修正：WM_APP_SHOWPAD 由 show 改为 toggle（已打开时再双击 = 关闭），符合 5-#17 的开/关直觉

10. 【补充 2026-10-06】锁死 bug 的真正根因是 #10 之外的第二个缺陷：hide() 入口 KillTimer(ANIM) 后调用 start_close()，而关闭动画依赖该定时器推进到 finalize_hide → 永不隐藏。两处都要修：hide 只停光标定时器；start_close 自行 SetTimer。验证方法：向 pad 窗口 PostMessage(WM_ACTIVATE, 0)（合成失活），600ms 后 IsWindowVisible(pad) 与 (scrim) 必须都为 False

## 2026-10-11（Kiki · 审查 + 修复）

12. 2026-10-11 | UI 6.5 | **背景幕从来没显示过**：`create_windows` 建了画刷却没传给 `register_class`（传的是 None），遮罩窗口是空的。已修：画刷作为类背景。u2-live.png 里「没有深色遮罩」就是这个
13. 2026-10-11 | UI 6.7 | 文本全部改为单行 + 按字符截断补「…」（之前文件名折两行、色块 DOCX 断成 DOC/X、截断没有省略号）。做法：在 `make_format` 上统一设 NO_WRAP + ellipsis trimming，`draw_text_trimmed` 不再覆盖成无省略号
14. 2026-10-11 | UI 6.6 | 文件类型色块宽 32 放不下四个字母（DOCX/HTML），改为 38，文件名起点 58→64（相对卡片）。**偏离规范，请用户确认**
15. 2026-10-11 | UI 第 9 节 | 内存口径澄清：规范指标是「内存(专用工作集)」= PrivateWorkingSetSize。之前 checklist 里的 32.8MB 是提交量（PrivateUsage），不是同一个数。日志现在两个都记：`mem[...] commit=… private_ws=…`
16. 2026-10-11 | UI 第 9 节 | 内存优化（实测，2560×1600 / 150%）：
    - 渲染改为「D2D → WIC 位图 + GDI 互操作推屏」，去掉 DIB+DC 渲染目标的双份缓冲（surface.rs，旧实现备份在 tools/bak/surface_dc.rs）
    - 启动台用**独立的 D2D 工厂 + ISOLATED DWrite 工厂**，隐藏时连工厂一起释放（软件渲染的暂存缓冲挂在工厂上，只放渲染目标不够）；隐藏后 HeapCompact + CoFreeUnusedLibrariesEx
    - 引擎 `refresh()` 不再每次打开都全量枚举 AppsFolder：应用重扫至少间隔 5 分钟（开始菜单/桌面的变化仍由目录监听即时触发），见 engine lib.rs `SCAN_APPS_SOFT`
    - 结果：空闲 4.2MB → 首次扫描后 6.1MB；**显示时 19.9MB（增量 ≈ 14MB）；隐藏后 11.5～12.6MB（目标 <15 ✅）**
    - 增量目标 <12MB 在 1080p/150% 下估算约 12～13MB，仍略超：其中 8.5MB 是面板像素本身（1752×1152×4），属硬成本。继续压的方向：按卡片拆成三个更小的分层窗口、或把阴影边距从 24 DIP 收到 16
17. 2026-10-11 | 工具 | 新增无头截图 `DeskPad.exe --render-test out.bmp [--query q]`（不建可见窗口、不占单实例锁），`tools/bmp2png.py` 转 PNG；自检脚本 tools/watch.bat（只跑 tools 下固定的 build.bat / probe.bat）

## 2026-10-12（Kiki · 图标改用系统图标）

18. 2026-10-12 | 引擎 3.1 契约 | **契约新增字段**：`FileEntry.icon_png`、`DirEntry.icon_png`（`Option<PathBuf>`，带 `#[serde(default)]`，旧数据可读）。引擎在 FILES 阶段用 IShellItemImageFactory（只取图标）提取系统图标：普通文件按扩展名共享 `icons/ext-<ext>.png`，exe/lnk/url/ico/msi 与文件夹按路径缓存 `icons/f-<hash>.png` / `icons/d-<hash>.png`，7 天内复用。需同步写回引擎文档第 3.1 节
19. 2026-10-12 | UI 6.6/6.7 | 用户反馈手绘文件夹/文件色块与界面风格违和。改为与应用网格一致：最近文件用 24×24 系统文件图标；常用文件夹去掉白底边框格子，改成和应用格同一套状态（默认无底、悬停白底+L2 阴影上浮 2、选中主色边+浅蓝底），图标 36×36 系统文件夹图标（下载/图片/视频等自带系统徽记）。文件夹格高 72→80。取不到系统图标时才回退到 fileicons.rs 的矢量图形
20. 2026-10-12 | UI 6.6/6.7 | 用户连续否决「彩色色块」「系统图标」「彩色 squircle」三版，要求简洁高级感。定稿方向：右侧两张卡统一为**列表行**（行高 36）+ **单色线性符号**（20×20、线宽 1.5、圆头圆角、默认 ICON 灰，悬停/选中变主色），无底板无色块；常用文件夹从 3 列方格改为单列列表，名称左、计数右对齐（TEXT_TERTIARY）。layout.rs：dir_cols=1、dir_cell_h=36。系统图标提取（#18）暂留在引擎里但 UI 未使用，待用户确认后决定去留
21. 2026-10-12 | 引擎 3.1 契约 + UI 6.6 | **新功能：最近文件 / 常用文件夹可切换「图标 / 列表」视图**（用户需求）。契约 `Settings` 新增 `files_view`、`dirs_view: ViewMode { Grid(默认), List }`（serde default，旧配置可读）。卡片右上角 52×24 分段按钮（左图标、右列表），点击即保存并重排。图标视图：文件每行 3 个（带折角白纸 34×42 + L1 阴影 + 单色类型符号，格高 88），文件夹每行 4 个（浅灰蓝两层文件夹 46×36 + 高光 + 常用文件夹单色符号，格高 80）；格子状态与应用格一致。列表视图同 #20。键盘方向键按当前列数移动
22. 2026-10-12 | UI 6.7 + 引擎 icons.rs | 用户定稿：文件夹用 **Windows 系统文件夹图标，统一改成蓝色系**。引擎提取文件夹图标时做色相替换（有彩色像素的色相 → 214°，饱和度 ×0.85 封顶 0.78、明度 ×0.94；近白/灰的徽记与高光不动），缓存文件名前缀 `d2-` 以淘汰旧的黄色缓存。图标视图 48×48、列表视图 24×24；取不到时回退到矢量图形。recolor 有单元测试
23. 2026-10-12 | UI 6.7（**作废 #18、#22**）| 用户指出蓝色文件夹违反 tofour 规范（02：蓝色只用于交互；12：图标为线性、#65758B、不做装饰性 3D）。已读用户桌面上的 HexHook_design_system.docx 后重做：文件夹图标视图 = 冷灰两层（后板 #D7DEE7 / 前板 #EAEEF3 / 1px 描边 #C6CFDA）+ 前板顶部 1px 白高光 + L1 阴影，常用文件夹前板「压印」单色符号（#8D9AAC + 下方 1px 白色错位，early iOS letterpress），悬停/选中时符号变 #3878D8；形态参考 macOS 文件夹的「标签 + 后板 + 前板」结构，图形全部自绘。文件图标视图 = 白纸 + 折角 + 1px 边 + L1 阴影 + #65758B 线性符号。列表视图 = 20px 线性符号。系统图标提取与蓝色重着色代码、契约里的 FileEntry/DirEntry.icon_png 字段已删除
24. 2026-10-12 | 引擎 3.1 契约 + UI 6.7（替换 #23 的文件部分）| 用户要求文件图标「视频像视频、PPT 是封面、照片是照片、Word/Excel 各不相同，扁平化」。契约新增 `FileEntry.thumb_png: Option<PathBuf>`（serde default）：engine files 阶段对 Image/Video/Presentation/Pdf 调 IShellItemImageFactory::GetImage(160, SIIGBF_THUMBNAILONLY)，缓存 `icons/t-{fnv(path|mtime)}.png`（长边 ≤160），7 天未更新的 t-*.png 由 prune_thumbnails 删除；系统无缩略图处理器时为 None。UI：有缩略图 → 等比放入 68×45（网格）/24×24（列表），6px/4px 连续曲率圆角 + 1px #E5E5E5，视频叠白色圆形播放标记；无缩略图 → 扁平白纸 + 折角 + 1px 边（去掉阴影）+ 类型图形 + 底部扩展名标签，标签色取 tofour 14 节状态标签：Word 蓝、表格绿、演示橙、PDF 红，其余中性灰 #F0F2F5/#65758B。旧 draw_file_icon/kind_sym 已删
25. 2026-10-12 | 引擎 缩略图兜底 | 系统拿不到缩略图时内置兜底（新模块 engine/src/docthumb.rs）：PPTX/PPSX/POTX 读包内 docProps/thumbnail.jpeg（PowerPoint/WPS 保存时写入的首页封面；只按 zip 中央目录 seek 读这一项，单项/目录上限 4MB，仅 stored/deflate）；PDF 用系统自带 Windows.Data.Pdf 渲染首页，放在一次性子进程 `DeskPad.exe --thumb-pdf <in> <out> ...`（MTA，12s 超时即杀，每轮最多 8 个），WinRT/PDF 渲染器不进主进程。失败写 `icons/t-{key}.none` 标记，同一 mtime 不再重试，随 t-* 一起 7 天清理。依赖新增：image 的 jpeg feature、miniz_oxide 0.8；windows features 新增 Foundation/Data_Pdf/Storage/Storage_Streams。旧版二进制 .ppt 不支持，回退类型图标
26. 2026-10-12 | UI 6.6/6.11 | 用户找不到设置入口（托盘图标在 Win11 默认被收进溢出区）。启动台搜索框右侧新增 36×36 设置按钮（线性齿轮 20px，默认 ICON，悬停 ROW_HOVER 底 + PRIMARY），点击投递 WM_APP_SETTINGS(WM_APP+4) 给主窗口，在 with_app 之外调用 settings_ui::open（打开时启动台自动收起）。搜索框宽度相应减 44。托盘与角标右键菜单入口保留
27. 2026-10-12 | 发布 | 0.1.0 发布到公开仓库 HexNook-DeskPad-Releases：本会话无权创建 GitHub Release，改为文件直存 releases/<版本>/DeskPad-<版本>-win-x64.zip + NOTES.md，根目录 latest.json（version/url/sha256/size/notes）供 v0.2 在线更新读取
28. 2026-10-12 | 新 crate crates/setup | 用户要求：产品名 HN DeskPad，提供下载器（在线安装程序）HN-DeskPad-Setup.exe。读取发布仓库 latest.json（raw.githubusercontent，WinINet 跟随系统代理）→ 下载 zip → SHA-256 校验 → 退出正在运行的 DeskPad（先投 WM_APP_QUIT=WM_APP+5 正常退出，2s 后 TerminateProcess）→ 解压去掉顶层 DeskPad/ → 复制自身为 uninstall.exe → 开始菜单快捷方式（桌面可选）→ HKCU 卸载信息（HN DeskPad）。用户选的位置后面**自动加一层 HN DeskPad 文件夹**（已以它结尾不重复加），选到不可写位置提示换文件夹；全程 asInvoker（嵌入清单，防止文件名含 Setup 触发安装程序检测提权）。默认位置 %LOCALAPPDATA%\Programs，已安装时为原位置。界面 520×372，D2D 自绘，tofour Token。卸载：uninstall.exe --uninstall [--quiet]，删除 Run/StartupApproved 值、快捷方式、卸载信息、%LOCALAPPDATA%\DeskPad、安装目录。app 侧：新增 WM_APP_QUIT；托盘提示与错误框标题改为 HN DeskPad

## 2026-10-14（v0.2 收尾）

29. 2026-10-14 | UI 6.7 + 引擎 4.4 | 修复 checklist「已知视觉小问题」两条：① 文件夹图标视图：图标 y 10→8、名称 y 55→58（底 71→74），重叠消除（渲染帧 tools/shot-fix.png 验证）；② 常用文件夹按显示名去重：桌面分类目录与已知文件夹（文档/图片/视频等）重名时不再重复收录（渲染帧 tools/shot-fix2.png 验证，目录数 10→8）| 三项验收（build / test / clippy -D warnings）全过

30. 2026-10-14 | UI 6.10 | 用户要求角标按钮位置改为「内嵌 Windows 底部任务栏左端靠底」。实现：default_pos() 用 `FindWindowW("Shell_TrayWnd")` 拿任务栏矩形，按钮主体放任务栏左边缘内侧 6 DIP、底对齐 6 DIP（按按钮所在显示器 DPI 换算物理像素）；任务栏不存在时退回旧桌面左下角。层级联动：WM_WINDOWPOSCHANGING 按位置模式强制 insertAfter——默认位置插在任务栏 HWND 正上方（同属 topmost 段、不被普通窗口盖住，reposition() 在 WM_DISPLAYCHANGE / WM_SETTINGCHANGE 时自动重插，explorer 重启也能恢复）；拖走的浮动位置保持 HWND_BOTTOM。实测：corner rect=(-18,997)-(70,1085) 贴任务栏 (0,1019)-(1707,1067) 左端，GetWindow(GW_HWNDPREV) = 任务栏句柄（z 序验证通过）。启动台/托盘/气泡行为不变 | build / clippy -D warnings 通过

31. 2026-10-16 | UI 6.10（**部分作废 #30 的层级做法**）| 「内嵌任务栏」曾按 `SetParent(corner, Shell_TrayWnd)` 实现：`CreateWindowExW` 直接建 `WS_CHILD` 会失败（实测返回 NULL、HRESULT=0），改成建 `WS_POPUP` 后再 `SetParent` 则返回 Ok，但**用户完全看不见按钮**——跨进程子窗口被任务栏自己的绘制流程盖住；`PrintWindow` 仍能取到正确画面，说明窗口和渲染都正常，只是合成层级在任务栏之下。**最终做法：不做 SetParent**，改为顶层 `WS_POPUP` + 屏幕物理像素定位 + `WM_WINDOWPOSCHANGING` 强制 `hwndInsertAfter = HWND_TOPMOST`（浮动位置仍 `HWND_BOTTOM`）。实测 body=(9,1531) 落在任务栏 (0,1528)-(2560,1600) 左端，点击按钮 `pad::show` 正常打开启动台；附带好处：窗口始终属于本进程，explorer 重启不会连带销毁
32. 2026-10-16 | 工具 | 截图口径修正：PowerShell 未声明 DPI 感知时 `CopyFromScreen` 只覆盖左上 1707×1067（虚拟化坐标），会漏掉底部任务栏，导致误判"任务栏不存在"。看任务栏/角标一律先 `SetProcessDPIAware()` 再按真实物理分辨率（本机 2560×1600）截图；`GetWindowRect` 的读数同样受调用进程 DPI 感知状态影响

33. 2026-10-16 | UI 6.10 / 6.11 + 引擎 3.1 契约 | 用户提出三点：① 按钮不该能被拖出任务栏（拖出去还回不来）；② 去掉白色背景；③ 为什么不用任务栏的公开接口。处理：**① 位置固定**——删除全部拖动逻辑（`dragging/grab/down` 状态、`drag_to`、`SM_CXDRAG` 判定、网格吸附），`WM_LBUTTONDOWN/UP` 只做"按下→抬起=切换启动台"；位置每次由 `WM_DISPLAYCHANGE / WM_SETTINGCHANGE / WM_DPICHANGED` 重算，永远贴任务栏左端。契约同步删除 `Settings.launcher_pos` 与 `grid_snap`（旧 config.json 里这两个字段被 serde 忽略，能正常读）。**② 去掉白底**——按钮改为只画 18 DIP 产品图形（默认 PRIMARY，hover PRIMARY_HOVER），无背景、无边框、无阴影；命中区域 28×28 DIP，窗口不再留 SHADOW_MARGIN 透明边距（88×88→42×42 物理像素）。**③ 任务栏没有公开接口**——DeskBand / 任务栏工具栏在 Windows 7 起被移除，Win11 任务栏是 XAML+Composition 私有实现；官方可用的只有通知区域图标（`Shell_NotifyIcon`，Win11 默认折叠进溢出区），所以只能"贴准位置 + 正确层级"的覆盖窗口；注入 hook 或改 explorer 违反红线。设置窗口同步删除「网格对齐」勾选与「启动按钮位置/重置到默认」，主体高度 400→320 DIP。实测：按钮 body=(12,1543)-(54,1585) 贴任务栏 (0,1528)-(2560,1600)，单击 `pad::show complete (visible)` 正常 | build / clippy -D warnings / 9 个引擎测试全过

34. 2026-10-16 | UI 6.10 | #33 收尾时踩到两个坑，记录备查：① **分层窗口按像素 alpha 命中**——按钮改成"只画图形"后，四块小方块之间 alpha=0 的缝隙会把点击漏给任务栏，点图标正中央（缝隙所在处）没反应，点方块上才有效（实测三点命中/一点漏过）。解决：绘制时先铺 `HIT_SHIELD`（theme.rs，`0x08000000`，3% 黑，肉眼看不出，实测该处与任务栏底色只差 1 个色阶），整块 28 DIP 都能点。② **新增颜色必须注册进 palette**——`RenderCtx` 的画刷是启动时按固定列表预建的，`pal.get()` 遇到未注册颜色会返回 Err；直接用一个新 alpha 值会连带 `draw_corner` 失败 → `corner present failed: 0x80004005` → 按钮整个不显示（日志里只有一行 `render: brush 0x0c000000 not in palette`）。教训：加颜色先加 palette。
