# DeskPad UI 板块开发规范（deskpad-app）· v2

> 配套文档：`engine-architecture.md`（引擎板块）。**接口契约以引擎文档第 3 节为唯一依据**，本文只写用法，不复制类型定义。
> UI 板块只负责窗口、消息循环、热键、托盘、渲染和交互，所有数据都通过 `deskpad_engine::EngineHandle` 获取。
> **视觉权威：tofour Design System**（用户提供，位于 `C:\Users\15055\Desktop\HexHook_design_system.docx`）。第 4 节是它在启动台上的落地映射，两者冲突时以用户文档为准。
> **产品原则：功能上和 Windows 完美协调，用起来像系统自带的一样（第 5 节）；视觉上保持 tofour 风格，不模仿 Windows 外观；任何协调手段都不能以内存为代价（第 9 节）。**

## 0. 给执行 agent 的规则（先读）

1. **本文是唯一依据**。文中写死的数值、文案、顺序一律照做，不要"优化"。确实做不到时，停下来在 `docs/questions.md` 里追加一条（日期 + 章节号 + 问题 + 建议方案），然后跳过该项继续。
2. 视觉只看第 4 节和 tofour 文档。**禁止为了"像 Windows"而改视觉**：不加 Mica、Acrylic 或系统强调色，不用 DWM 圆角替代平滑圆角。
3. MUST / 必须是硬性要求，v0.2 字样的内容**不做**。
4. 编码约束：
   - `unwrap()` / `expect()` 只允许出现在测试里；
   - 每个 `unsafe` 块上方写一行 `// SAFETY:`；
   - Win32/COM 资源统一走 `src/win.rs` 的 RAII 包装（HDC、HBITMAP、HICON、HMENU 都要有 Drop）；
   - 每个里程碑完成时，`cargo build --release` 和 `cargo clippy -p deskpad-app -- -D warnings` 都必须通过。
5. **红线**：禁止 `SetWindowsHookEx`（包括低级键盘/鼠标钩子）、DLL 注入、进程伪装、轮询 `GetAsyncKeyState`。全局输入只允许通过 `RegisterHotKey` 获取。

## 1. 职责边界

- 做：
  - 进程引导：DPI 感知、STA COM、单实例检查及唤起已有实例；
  - 隐藏主窗口与消息循环；
  - 全局热键；
  - 托盘；
  - 启动台（遮罩窗口 + 内容窗口）、桌面角标按钮、首次引导气泡、设置窗口；
  - 键盘、鼠标、输入法交互；
  - 动画；
  - 与系统事件的协调（第 5 节）。
- 不做：文件枚举、图标提取、设置读写、注册表访问（全部归引擎）。UI 只允许使用 D2D、DWrite、WIC 做渲染和 PNG 解码。

## 2. 工程布局

> **项目根目录：`D:\MyWork\DeskPad\`**。根 Cargo.toml 和"谁建什么"的规则见引擎文档第 2 节；本 agent 先开工时，必须先按引擎文档第 6 节建好「E0 契约桩」。

```
D:\MyWork\DeskPad\crates\app\
├─ Cargo.toml          # 包名 deskpad-app，bin 名 DeskPad（第 8 节）
└─ src\
   ├─ main.rs          # 引导（6.1）
   ├─ app.rs           # 全局状态 App（单例，只在 UI 线程访问）+ 消息常量
   ├─ winmain.rs       # 隐藏主窗口：消息路由（6.2）
   ├─ hotkey.rs        # RegisterHotKey 封装（6.3）
   ├─ tray.rs          # 托盘（6.4）
   ├─ pad.rs           # 启动台：遮罩窗口 + 内容窗口的生命周期、显示与隐藏（6.5）
   ├─ layout.rs        # 布局计算 + 命中测试（6.6，纯函数，有单元测试）
   ├─ render.rs        # D2D 绘制（6.7）
   ├─ surface.rs       # 分层窗口离屏表面：DIB + DCRenderTarget + UpdateLayeredWindow（6.7）
   ├─ icons.rs         # PNG → ID2D1Bitmap（WIC），只解码可见项（6.7）
   ├─ input.rs         # 键鼠与输入法状态机（6.8）
   ├─ anim.rs          # 开关动画（6.9）
   ├─ corner.rs        # 桌面角标按钮 + 首次引导气泡（6.10）
   ├─ settings_ui.rs   # 设置窗口（6.11）
   ├─ glyph.rs         # 产品图形（角标、托盘图标运行时生成）（6.12）
   ├─ fmt.rs           # 相对时间格式化、错误文案（6.13，纯函数，有单元测试）
   ├─ theme.rs         # tofour Token（4.1 原样照抄）
   ├─ smooth.rs        # 连续曲率圆角 + 软阴影（4.3）
   └─ win.rs           # RAII 包装与小工具
```

**线程模型**：UI 只有主线程一个线程，不开任何后台线程。引擎通知通过 `set_on_change` 回调里的 `PostMessageW` 送回主线程，v1 里那个专门转发 receiver 的线程已删除。

## 3. 使用引擎

```rust
use deskpad_engine::{self as engine, EngineHandle, EngineError, Snapshot, Settings, HotkeyPreset, LaunchTarget};

let _guard = engine::single::acquire()?;          // 6.1：Err(AlreadyRunning) 时唤起已有实例后退出
let eng = EngineHandle::init()?;                  // 有缓存时立即返回
let hwnd_raw = main_hwnd.0 as isize;              // HWND 不是 Send，以整数形式捕获进闭包
eng.set_on_change(Box::new(move || unsafe {
    // SAFETY: PostMessageW 可以跨线程调用；hwnd 在引擎 shutdown 之前一直有效
    let _ = PostMessageW(HWND(hwnd_raw as _), WM_APP_SNAPSHOT, WPARAM(0), LPARAM(0));
}));
let snap: Snapshot = eng.snapshot();
let hits: Vec<usize> = eng.search(&snap, &query); // snap.apps 的下标
eng.launch_app(id, &query)?;                       // 从网格点击启动时 query 传 ""
eng.open_path(&path)?;
eng.refresh();                                     // 每次显示启动台时调用
let s = eng.settings(); eng.save_settings(&s)?;
eng.shutdown();                                    // 退出前调用
engine::log::info("...");
```

## 4. 设计系统 tofour（权威视觉规范）

> 来源：用户的 `HexHook_design_system.docx`（tofour · Retro Modern UI · 轻圆轻立体 · 早期 iOS / 2010s Web UI）。
> 本节是它在启动台上的落地映射。**禁令：毛玻璃、Neon/发光、大面积渐变、胶囊圆角(9999px)、厚重阴影。**

### 4.1 设计 Token（theme.rs，原样照抄）

```rust
// 色彩（ARGB 存储序，0xAARRGGBB）
pub const PRIMARY: u32 = 0xFF3878D8;        // 主色：只用于 Active / Selected / Focus
pub const PRIMARY_HOVER: u32 = 0xFF4A89E5;
pub const PRIMARY_PRESSED: u32 = 0xFF2E68C2;
pub const BACKGROUND: u32 = 0xFFF5F6F7;     // 页面背景
pub const SURFACE: u32 = 0xFFFAFAFA;        // 卡片 / 面板
pub const INPUT: u32 = 0xFFFFFFFF;          // 输入框
pub const TEXT_PRIMARY: u32 = 0xFF222222;
pub const TEXT_SECONDARY: u32 = 0xFF888888;
pub const TEXT_TERTIARY: u32 = 0xFFA3A3A3;
pub const BORDER: u32 = 0xFFE5E5E5;         // 1px 默认边框
pub const BORDER_HOVER: u32 = 0xFFD9D9D9;
pub const ICON: u32 = 0xFF65758B;
pub const SELECT_BG: u32 = 0xFFEAF2FF;      // 选中 / 浅蓝背景
pub const ROW_HOVER: u32 = 0xFFF7F7F7;      // 文件行 hover
pub const DANGER: u32 = 0xFFD93025;         // 错误提示文字；tofour 文档定义了 Danger 色时以文档为准
pub const SCRIM: u32 = 0x4D0F121A;          // 背景幕 rgba(15,18,26,0.30)
// 圆角（4 的倍数递进；连续曲率，见 4.3）
pub const RADIUS_CARD: f32 = 10.0;          // 启动台三大面板
pub const RADIUS_INPUT: f32 = 6.0;          // 搜索框
pub const RADIUS_ITEM: f32 = 6.0;           // 应用格、文件行 hover 块、文件夹格（同心：10 − 4）
pub const RADIUS_MODAL: f32 = 12.0;         // 设置窗口
pub const RADIUS_TOAST: f32 = 8.0;          // 引导气泡
pub const RADIUS_BADGE: f32 = 4.0;          // 文件类型色块
// 阴影 (blur, dy, alpha)
pub const SHADOW_L1: (f32, f32, f32) = (4.0, 1.0, 0.06);    // 角标按钮
pub const SHADOW_L2: (f32, f32, f32) = (6.0, 2.0, 0.08);    // Hover / Toast
pub const SHADOW_L3: (f32, f32, f32) = (12.0, 4.0, 0.12);   // 面板 / Modal
pub const SHADOW_INPUT: (f32, f32, f32) = (2.0, 1.0, 0.04); // 搜索框内阴影 inset
pub const SHADOW_MARGIN: f32 = 24.0;        // 带阴影窗口四周预留的透明边距（≥ blur*1.5 + dy）
// 字体
pub const FONT_FAMILY: &str = "Microsoft YaHei UI";
pub const FS_TITLE: f32 = 16.0;    // 面板标题，600
pub const FS_BODY: f32 = 14.0;     // 文件名、搜索框、按钮，400
pub const FS_CAPTION: f32 = 12.0;  // 应用名、文件夹名、计数，400
pub const FS_SMALL: f32 = 11.0;    // 时间、类型色块文字，400（类型色块用 600）
// 间距（8px 网格）
pub const PAD_CARD: f32 = 16.0;    // 卡片内边距
pub const GAP: f32 = 12.0;         // 组件间距、应用格间距
pub const GAP_CARD: f32 = 16.0;    // 三块面板之间
```

### 4.2 启动台组件落地

| 组件 | tofour 规范 |
|---|---|
| 背景幕 | **纯半透明深色遮罩 `SCRIM`，不加模糊**（毛玻璃被禁） |
| 三块面板 | `SURFACE` 填充 + `1px BORDER` 边框 + `RADIUS_CARD` 连续曲率 + L3 阴影 |
| 搜索框 | `INPUT` + `1px BORDER` + 内阴影 `SHADOW_INPUT` + `RADIUS_INPUT`；**聚焦时边框变 `PRIMARY`**（启动台显示期间搜索框始终处于聚焦态）；placeholder「搜索应用」用 `TEXT_TERTIARY` |
| 应用格 | 默认无底无边。**Hover**：`INPUT` 白底 `RADIUS_ITEM` + L2 阴影，图标上移 2px。**键盘选中**：`1px PRIMARY` 边框 + `SELECT_BG` 底。两种状态同时成立时，用选中样式并保留上移 |
| 文件行 | 文件名 `TEXT_PRIMARY`、时间 `TEXT_TERTIARY`（`FS_SMALL`）；Hover 时 `ROW_HOVER` 圆角行；类型色块用内容色（6.7 表） |
| 文件夹格 | `INPUT` 白底 + `1px BORDER` + `RADIUS_ITEM`；Hover 时 `SELECT_BG` 背景、名称变 `PRIMARY` |
| 面板标题 | `FS_TITLE` / 600 / `TEXT_PRIMARY`；计数等辅助信息 `FS_CAPTION` / `TEXT_SECONDARY` |
| 蓝色使用 | **只用于交互状态**（选中 / Focus / Active）和产品图形，不做大面积装饰 |

### 4.3 连续曲率圆角（smooth.rs）

- 所有圆角都用连续曲率（ξ = 0.6），半径数值仍取 4/6/8/10/12px；嵌套同心时，内层圆角 = 外层圆角 − 内边距。
- **直接移植现成实现**：`D:\MyWork\HexNookIME\server\smooth_corner.cpp/.h`（C++/Direct2D，包含 `fillSmoothRect / drawSmoothRect / drawSoftShadow`；阴影是高斯九宫格拉伸，没有分层台阶）。
- Rust 化方式：几何部分（三次贝塞尔控制点公式）改写为 `ID2D1Factory::CreatePathGeometry` + `ID2D1GeometrySink`（AddBezier / AddLine）。`drawSoftShadow` 的九宫格**按 (半径, blur) 预渲染一次角贴图并缓存**，每帧只做拉伸贴图，不实时模糊。这些都是已有的自研代码，直接参考移植，不引入第三方库。
- 空间不足时的退化规则照抄原实现：R 不超过短边的一半，必要时 ξ 降为 (短边/2)÷R − 1。
- 生成的路径几何按 (w, h, r) 缓存在 `HashMap` 里，启动台隐藏时清空。

### 4.4 渲染技术路线（v0.1 定案）

- **Direct2D 软件渲染**：`ID2D1Factory`（SINGLE_THREADED）创建 `ID2D1DCRenderTarget`，渲染目标属性为 `type = D2D1_RENDER_TARGET_TYPE_SOFTWARE`、`pixelFormat = (DXGI_FORMAT_B8G8R8A8_UNORM, D2D1_ALPHA_MODE_PREMULTIPLIED)`、`dpi = 显示器 DPI`（于是所有绘制直接使用 DIP 逻辑单位）。
  - **必须用 SOFTWARE**：硬件渲染会把 D3D 和显卡驱动加载进进程，常驻内存会多出 10～30 MB，违反第 9 节。
- 分层窗口呈现（surface.rs）：
  1. `CreateDIBSection`（32bpp top-down）；
  2. 选入内存 DC，`BindDC`；
  3. `BeginDraw` / `EndDraw`；
  4. `UpdateLayeredWindow(ULW_ALPHA, BLENDFUNCTION{AC_SRC_OVER, 0, alpha, AC_SRC_ALPHA})`。
- 只在数据或交互变化时重绘，空闲时不跑任何定时器（光标闪烁定时器只在启动台显示期间存在）。
- `EndDraw` 返回 `D2DERR_RECREATE_TARGET` 时，重建 RT 和所有设备相关位图，再重绘一次。

## 5. 与 Windows 协调（行为层，像系统自带功能一样）

> 目标：用户感觉不到这是一个第三方程序——不抢焦点、不挡游戏、不留垃圾、系统状态变化时自己跟上。下表每一行都是验收项（第 10 节 U6）。

| # | 场景 | 必须的行为 | 实现要点 |
|---|---|---|---|
| 1 | 呼出 | 在**鼠标所在显示器**上打开，120ms 内可交互，输入焦点直接在搜索框，可以立刻打字 | 6.5 |
| 2 | 关闭后焦点 | 按 Esc 或再按热键关闭时，**焦点还给呼出前的窗口**（和开始菜单一样） | 显示前记下 `GetForegroundWindow()`；关闭时如果它仍 `IsWindow` 就 `SetForegroundWindow` 回去。因启动应用、打开文件或点击别处而关闭时不还焦点 |
| 3 | 启动应用 | 新程序出现在最前面，启动台立刻消失，没有动画残影 | 先 `launch_app`，再**立即**隐藏两个窗口（不做淡出），这样前台权限会自然交给新进程 |
| 4 | 游戏与全屏 | 前台是全屏游戏、全屏演示或全屏视频时，**热键不呼出**，以免把游戏切出去 | WM_HOTKEY 到达时先调 `SHQueryUserNotificationState`，结果为 `QUNS_RUNNING_D3D_FULL_SCREEN`、`QUNS_PRESENTATION_MODE` 或 `QUNS_BUSY` 时直接忽略。托盘和角标点击不受影响 |
| 5 | 系统键 | 启动台打开时按 Win、Alt+Tab、Win+D 或点击任务栏，启动台自然收起 | 内容窗口收到 `WM_ACTIVATE(WA_INACTIVE)` 就隐藏（激活的是自己的设置窗口时除外） |
| 6 | 锁屏、睡眠 | 锁屏时启动台立即收起；唤醒后数据自动刷新 | `WTSRegisterSessionNotification(main, NOTIFY_FOR_THIS_SESSION)`，`WTS_SESSION_LOCK` 时隐藏；`WM_POWERBROADCAST` 收到 `PBT_APMRESUMEAUTOMATIC` 时调用 `eng.refresh()` |
| 7 | 关机、注销 | 不拖慢关机，不弹任何对话框 | `WM_QUERYENDSESSION` 返回 TRUE；`WM_ENDSESSION(wParam=TRUE)` 时删除托盘图标并调用 `eng.shutdown()`，然后直接返回 |
| 8 | 资源管理器重启 | 托盘图标自动回来 | 处理 `RegisterWindowMessageW("TaskbarCreated")`，收到后重新 NIM_ADD。**所以主窗口不能是 message-only 窗口**（message-only 窗口收不到广播） |
| 9 | 多显示器、DPI | 每个显示器都清晰；拔插显示器后角标不会跑到看不见的地方 | 进程第一行就调用 PerMonitorV2；`WM_DPICHANGED` 时按建议矩形调整；`WM_DISPLAYCHANGE` 以及 `WM_SETTINGCHANGE(SPI_SETWORKAREA)` 时把角标钳回可见工作区 |
| 10 | 系统偏好 | 跟随「动画效果」开关、光标闪烁速度、拖动阈值、滚轮行数 | `SPI_GETCLIENTAREAANIMATION` 为 FALSE 时所有动画直接跳到终态；光标闪烁用 `GetCaretBlinkTime()`；拖动判定用 `SM_CXDRAG/SM_CYDRAG`（按 DPI 取值）；滚轮每 120 单位滚一行网格。`WM_SETTINGCHANGE` 时重新读取这些设置 |
| 11 | 输入法 | 中文输入法直接在搜索框打字，候选窗出现在光标下方；Ctrl+V 可以粘贴 | 6.8 |
| 12 | 热键冲突 | 热键被占用时不抢、不报错弹窗，只发一条系统通知 | 6.3 |
| 13 | 通知 | 只通过托盘气泡发系统通知（自动遵守专注助手 / 勿扰），全程最多两种：热键冲突、启动失败（启动台已关闭时） | `NIF_INFO`，dwInfoFlags = `NIIF_INFO`，不带声音（`NIIF_NOSOUND`） |
| 14 | 权限 | 不以管理员身份运行，避免它启动的程序也带上管理员权限 | 启动时 `IsUserAnAdmin()` 为真 → 弹 MessageBox「请不要以管理员身份运行 DeskPad」后退出 |
| 15 | 开机自启 | 开机时静默驻留，不弹任何东西（包括首次引导气泡） | 命令行带 `--autostart` 时跳过引导气泡，延迟到用户第一次打开启动台之后再显示 |
| 16 | 干净 | 不在 `%LocalAppData%\DeskPad` 和 HKCU Run 键之外留下任何东西；退出后没有残留进程 | 托盘「退出」→ 销毁所有窗口 → `eng.shutdown()` → `PostQuitMessage(0)` |
| 17 | 二次启动 | 再次双击 exe = 打开已在运行的启动台，而不是报错 | 6.1 第 4 步 |

## 6. 模块明细

### 6.1 main.rs — 引导（顺序不可调换）

```rust
#![windows_subsystem = "windows"]
```

1. `SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)`，**必须在创建任何窗口之前调用**（v1 把它放在第 6 步是错的）。
2. `IsUserAnAdmin()` 为真 → MessageBox 提示后退出（5-#14）。
3. `CoInitializeEx(COINIT_APARTMENTTHREADED)`，用 `ComInit` 守卫保持到 main 结束。
4. `engine::single::acquire()`：
   - 返回 `Err(AlreadyRunning)` 时：`AllowSetForegroundWindow(ASFW_ANY)`；然后 `FindWindowW("DeskPadMain", None)`，找到就 `PostMessageW(WM_APP_SHOWPAD)`；最后退出码 0 退出。
   - `WM_APP_SHOWPAD = RegisterWindowMessageW("HexNook.DeskPad.ShowPad")`。
5. `EngineHandle::init()`，失败时 MessageBox「DeskPad 启动失败，详情见日志」后退出。
6. 创建 D2D factory、DWrite factory、WIC factory（WIC 用 `CoCreateInstance(CLSID_WICImagingFactory)`），三者在进程生命周期内只创建一次。
7. 注册窗口类，创建隐藏主窗口 `DeskPadMain`。它是**普通的顶层 WS_POPUP 窗口，从不显示，也不是 message-only 窗口**（原因见 5-#8）。
8. `eng.set_on_change(...)`（见第 3 节）。
9. `tray::add()` → `hotkey::register()` → `corner::create()` → `WTSRegisterSessionNotification`。
10. 没有 `--autostart`，且 `settings.first_run_done == false` → 显示引导气泡。
11. `GetMessageW` 循环。退出后依次 `eng.shutdown()`，`_guard` 被 drop。

### 6.2 winmain.rs — 消息路由（主窗口 WndProc）

| 消息 | 处理 |
|---|---|
| `WM_HOTKEY` (id=1) | 先按 5-#4 检查全屏状态，然后 `pad::toggle(ShowReason::Hotkey)` |
| `WM_APP_TRAY` (WM_APP+1) | 见 6.4 |
| `WM_APP_SNAPSHOT` (WM_APP+2) | `pad::on_snapshot()`：重新取 snapshot；启动台可见时重新搜索当前 query，按 id 保持选中项，然后重绘 |
| `WM_APP_SHOWPAD`（注册消息） | `pad::show(ShowReason::SecondInstance)` |
| `TaskbarCreated`（注册消息） | `tray::add()` |
| `WM_WTSSESSION_CHANGE` | `WTS_SESSION_LOCK` → `pad::hide(HideReason::System)` |
| `WM_POWERBROADCAST` | `PBT_APMRESUMEAUTOMATIC` → `eng.refresh()` |
| `WM_SETTINGCHANGE` | 重新读取 5-#10 的系统偏好；`SPI_SETWORKAREA` → `corner::reclamp()` |
| `WM_DISPLAYCHANGE` | `corner::reclamp()`；启动台可见时直接隐藏 |
| `WM_QUERYENDSESSION` / `WM_ENDSESSION` | 见 5-#7 |

### 6.3 hotkey.rs

- `register(preset)`：
  1. `let (m, vk) = preset.mods_vk()`；
  2. `RegisterHotKey(main, 1, HOT_KEY_MODIFIERS(m | MOD_NOREPEAT), vk)`。
- 注册失败时：发托盘通知，标题「DeskPad」，正文「{preset.label()} 已被其他程序占用。可以点击桌面左下角的图标，或在设置中换一个快捷键。」程序照常运行。
- `change(old, new)`：先 `UnregisterHotKey`，再注册 new；失败就重新注册 old，并返回 Err 给设置窗口，由设置窗口在行内显示错误。
- 设置窗口里快捷键选项下方的说明文字：「Alt + Space 会替代窗口左上角菜单的系统快捷键」。

### 6.4 tray.rs

- `Shell_NotifyIconW(NIM_ADD)` 的参数：
  - `uID = 1`；
  - `uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP`；
  - `uCallbackMessage = WM_APP_TRAY`；
  - `szTip = "DeskPad"`；
  - hIcon 由 `glyph::tray_icon(GetSystemMetricsForDpi(SM_CXSMICON, dpi))` 生成。
- 添加成功后紧接着 `NIM_SETVERSION`（`NOTIFYICON_VERSION_4`）。
- **不使用 guidItem**：GUID 和 exe 路径绑定，exe 一旦被移动就会注册失败。
- VERSION_4 下 `LOWORD(lParam)` 是事件类型：
  - `WM_LBUTTONUP` / `NIN_SELECT` / `NIN_KEYSELECT` → `pad::toggle(ShowReason::Tray)`；
  - `WM_CONTEXTMENU` → 弹出菜单。
- 菜单用系统原生菜单：`CreatePopupMenu`，项目依次为【打开启动台 / 设置… / 开机自启（勾选态取自 settings().autostart）/ 分隔线 / 退出】。
  - 弹出前 `SetForegroundWindow(main)`；
  - 用 `TrackPopupMenuEx(TPM_RIGHTBUTTON | TPM_RETURNCMD)`，坐标取 `GET_X/Y_LPARAM(wParam)`；
  - 弹出后 `PostMessageW(main, WM_NULL)`（这是系统菜单正常消失必需的已知写法）。
- 「开机自启」点击后：`s.autostart = !s.autostart; eng.save_settings(&s)`。

### 6.5 pad.rs — 启动台（两个窗口）

**为什么拆成两个窗口**：背景幕要铺满整个工作区。如果用 UpdateLayeredWindow 推满屏 DIB，4K 屏幕上光这一块就是 33 MB。拆开以后，背景幕由系统用纯色加整窗透明度合成，进程内**零**位图；只有面板区域需要 DIB。

| 窗口 | 类名 | 样式 | 内容 |
|---|---|---|---|
| 背景幕 | `DeskPadScrim` | `WS_POPUP`；`WS_EX_LAYERED \| WS_EX_TOOLWINDOW \| WS_EX_TOPMOST \| WS_EX_NOACTIVATE` | 类背景刷 `CreateSolidBrush(RGB(15,18,26))`；透明度由 `SetLayeredWindowAttributes(LWA_ALPHA, 77)` 设置（77 = 0.30×255）；铺满鼠标所在显示器的 **rcWork** |
| 内容 | `DeskPadPad` | `WS_POPUP`；`WS_EX_LAYERED \| WS_EX_TOOLWINDOW \| WS_EX_TOPMOST`；**owner = 背景幕窗口**（从而始终位于背景幕之上） | 三块面板，用 UpdateLayeredWindow 推送；窗口矩形 = 面板外接矩形向四周扩 `SHADOW_MARGIN` |

- 背景幕窗口处理 `WM_MOUSEACTIVATE` 时返回 `MA_NOACTIVATE`；收到 `WM_LBUTTONDOWN` 或 `WM_RBUTTONDOWN` → `hide(HideReason::ClickOutside)`。
- 内容窗口中 alpha=0 的像素会让鼠标点击穿透到背景幕，背景幕负责关闭，**不需要额外的命中测试**。

**show(reason)：**

1. 已经可见时直接返回（ShowReason::Hotkey 和 Tray 走 toggle，toggle 遇到可见状态会调用 hide）。
2. 记录 `prev_fg = GetForegroundWindow()`。
3. `GetCursorPos` → `MonitorFromPoint(MONITOR_DEFAULTTONEAREST)` → 用 `GetMonitorInfoW` 取 rcWork，用 `GetDpiForMonitor(MDT_EFFECTIVE_DPI)` 取 dpi。
4. 取 `snap = eng.snapshot()`，然后 `eng.refresh()`；清空 query，`selected = Some(0)`，`focus = Apps`，`scroll = 0`。
5. 调 `layout::compute(rcWork 换算成 DIP, dpi, &snap, &settings)`；`surface.ensure(物理尺寸)`；`render` 第一帧；`UpdateLayeredWindow(alpha = 0)`。
6. 背景幕 `SetWindowPos(HWND_TOPMOST, rcWork, SWP_NOACTIVATE | SWP_SHOWWINDOW)`，alpha 0；内容窗口 `ShowWindow(SW_SHOW)`，然后 `SetForegroundWindow(内容窗口)`。
7. 启动光标闪烁定时器，调用 `anim::open()`（6.9）。

**hide(reason)：**

1. 停掉所有定时器。
2. 按 reason 处理：
   - `Launch`：立即 `ShowWindow(SW_HIDE)` 两个窗口；
   - 其他：系统动画开启时做 80ms 淡出，然后隐藏。
3. reason 是 `Escape` 或 `Hotkey`，并且 `IsWindow(prev_fg)` → `SetForegroundWindow(prev_fg)`。
4. **释放内存**：DIB section、内存 DC、DCRenderTarget、全部图标 `ID2D1Bitmap`、圆角几何缓存。只保留三个 factory 和 DWrite 文本格式。

### 6.6 layout.rs — 布局（单位：DIP；纯函数，必须有单元测试）

输入 `work: RectF`（工作区，DIP）、`max_recent`、`dirs_len`；输出所有矩形。

```
content_w = min(work.w − 96, 1120)      content_h = min(work.h − 96, 720)
content   = 在 work 中水平垂直居中（最小支持的工作区为 1024×640 DIP）
right_w   = if content_w < 1000 { 280 } else { 320 }
apps_card = (content.x, content.y, content_w − GAP_CARD − right_w, content_h)
files_h   = min(64 + max_recent*36, content_h − GAP_CARD − 168)
files_card= (apps_card.right + GAP_CARD, content.y, right_w, files_h)
dirs_card = (files_card.x, files_card.bottom + GAP_CARD, right_w, content_h − files_h − GAP_CARD)
```

- **应用卡**：
  - 搜索框 `(16, 16, w−32, 36)`；
  - 网格区域从 y=68 开始，高 `h−68−16`；
  - 6 列，列间距与行间距都是 `GAP`；`cell_w = (w − 32 − 5*12) / 6`，`cell_h = 104`；
  - 格内：图标 62×62，水平居中，距格顶 10；名称框 `(4, 80, cell_w−8, 16)`，`FS_CAPTION`，居中，单行，超长时省略号（`DWRITE_TRIMMING_GRANULARITY_CHARACTER` + 省略号符号）；
  - 可见行数 = `floor((grid_h + 12) / 116)`；内容超出时右侧 4px 内画滚动条：轨道不画，滑块 `BORDER_HOVER`，圆角 2。
- **文件卡**：
  - 标题「最近文件」位于 `(16, 16)`，高 24；
  - 行从 y=48 开始，行高 36，可见行数 `floor((files_h − 64) / 36)`；
  - 行矩形 `(8, y, w−16, 36)`；类型色块 `(16, 8, 32, 20)`；文件名从 x=58 到 `w−16−64`；时间右对齐，宽 64。
- **文件夹卡**：
  - 标题「常用文件夹」；
  - 网格 3 列，间距 8，`cell_h = 72`；
  - 格内：文件夹图形 24×20，居中，距格顶 12；名称 `FS_CAPTION`，y=38；计数「{n} 项」`FS_SMALL`，y=54；
  - 放不下的行直接不显示（v0.1 不做滚动）。
- `hit_test(pt) -> Hit::{ App(usize 可见下标) | File(usize) | Dir(usize) | Search | Card | None }`。`Card` 表示点在面板空白处（不关闭）；`None` 不会出现，因为透明区域的点击已经穿透给背景幕。
- **单元测试**：1920×1040 和 1024×640 两种工作区下，面板之间不重叠、全部位于工作区内、cell_w > 62。

### 6.7 render.rs / surface.rs / icons.rs

每帧的绘制顺序（内容窗口，原点在面板外接矩形向外扩 `SHADOW_MARGIN` 的位置）：

1. `Clear(透明)`。
2. 三块面板：`drawSoftShadow(L3)` → `fillSmoothRect(SURFACE)` → `drawSmoothRect(BORDER, 1px)`。
3. 搜索框：白底、内阴影、聚焦边框；文字 `FS_BODY`；光标为 1px×18 的 `TEXT_PRIMARY` 竖线；query 为空时显示 placeholder。
4. 应用网格：按 4.2 画 hover / 选中状态 → 图标 → 名称。图标还没加载好或 icon_png 为 None 时，画一个 `BORDER` 色的 62px 平滑圆角方块占位。
5. 文件行：hover 背景 → 类型色块（扩展名转大写，最多 4 个字符，`FS_SMALL`/600/白色，居中）→ 文件名 → 时间。
6. 文件夹格：白底、边框 → 文件夹图形（`ICON` 色，用 D2D 路径画：上方小标签 + 圆角矩形）→ 名称 → 计数。
7. 状态文字（应用卡网格区域居中，`FS_BODY`/`TEXT_SECONDARY`）：
   - `!snap.ready && apps 为空` →「正在读取应用…」
   - 搜索无结果 →「没有找到“{q}”」
   - 最近文件为空（在文件卡内显示）→「最近没有打开过文件」
8. 错误条：启动失败时，在搜索框下方 4px 处显示 `DANGER` 色 `FS_CAPTION` 文字，3 秒后消失（文案见 6.13）。

**类型色块颜色**（内容色，不属于交互色）：

| Document | Spreadsheet | Presentation | Pdf | Image | Video | Audio | Archive | Installer | Code | Other |
|---|---|---|---|---|---|---|---|---|---|---|
| #2B579A | #217346 | #D24726 | #D93025 | #8E5BD8 | #E8710A | #D81B60 | #8D6E63 | #00897B | #455A64 | #9E9E9E |

**icons.rs**：

- 缓存结构：`HashMap<AppId, (PathBuf, ID2D1Bitmap)>`。
- **只为当前可见的格子解码**，滚动时按需补齐；启动台隐藏时整个清空（6.5）。
- 解码流程：`IWICImagingFactory::CreateDecoderFromFilename(GENERIC_READ, WICDecodeMetadataCacheOnLoad)` → `GetFrame(0)` → `CreateFormatConverter().Initialize(GUID_WICPixelFormat32bppPBGRA)` → `rt.CreateBitmapFromWicBitmap`。
- 收到新快照时，只淘汰 id 已经消失或 icon_png 路径变了的条目。

**surface.rs**：

- `ensure(w, h)`：尺寸不变时复用。否则重建 DIB（`CreateDIBSection`，32bpp，biHeight 取负）和内存 DC，并 `BindDC` 整个矩形。
- `present(hwnd, pos, alpha)` 调用 `UpdateLayeredWindow(hwnd, None, Some(&pos), Some(&size), memdc, Some(&POINT{0,0}), COLORREF(0), Some(&blend), ULW_ALPHA)`。
- `set_alpha_and_pos(hwnd, pos, alpha)`：hdcSrc 传 None，**只改透明度和位置，不重新上传位图**（动画全部用它，6.9）。

### 6.8 input.rs — 键鼠与输入法状态机

状态：`query: String`、`focus: Apps | Files | Dirs`、`selected: Option<usize>`（当前焦点区内的下标）、`hover: Option<Hit>`、`scroll_row: usize`、`hits: Vec<usize>`。

| 输入 | 行为 |
|---|---|
| `WM_CHAR` 可打印字符 | `query.push(c)` → `hits = eng.search(&snap, &query)` → `focus = Apps; selected = Some(0); scroll_row = 0` |
| `VK_BACK` | 删除一个字符（按字素簇删除：如果末尾是低代理项，就连同前面的高代理项一起删）；Ctrl+Backspace 清空 query |
| `Ctrl+V` | `OpenClipboard` → 取 `CF_UNICODETEXT` 的第一行，去掉首尾空白，最多 64 个字符，追加到 query |
| `VK_ESCAPE` | query 非空 → 清空 query；query 为空 → `hide(Escape)` |
| 方向键 | 在当前焦点区里移动选中项：应用区按 6 列网格移动，可以跨行，超出可见区域时自动滚动；文件区和文件夹区同理 |
| `VK_TAB` / `Shift+Tab` | 焦点在 Apps → Files → Dirs 之间循环，selected 置 0 |
| `Home` / `End` / `PageUp` / `PageDown` | 跳到首项、末项，或滚动一屏 |
| `VK_RETURN` | 对当前选中项执行动作；没有选中项但 hits 非空时，执行 hits[0] |
| `WM_MOUSEMOVE` | 更新 hover，只有变化时才重绘；第一次移动时调用 `TrackMouseEvent(TME_LEAVE)` |
| `WM_MOUSELEAVE` | 清除 hover |
| `WM_MOUSEWHEEL` | 每 120 单位滚动一行（应用区） |
| `WM_LBUTTONUP` | **抬起时触发，不是按下时**：App 项 → 启动；File / Dir 项 → open_path；`Search` / `Card` → 不做任何事 |

- **执行动作**：
  - 应用：`eng.launch_app(id, &query)`；
  - 文件或文件夹：`eng.open_path(&path)`；
  - 成功 → `hide(Launch)`；失败 → 显示错误条，启动台保持打开。
- **输入法**：
  - 内容窗口不调用 `ImmAssociateContextEx` 禁用输入法，保持默认上下文；
  - 每次重绘后，以及处理 `WM_IME_STARTCOMPOSITION` 时，调用 `ImmGetContext` → `ImmSetCompositionWindow(CFS_POINT, 光标位置)` 和 `ImmSetCandidateWindow(CFS_CANDIDATEPOS, 光标下沿)` → `ImmReleaseContext`。坐标是窗口客户区内的物理像素。
  - 上屏的文字由 DefWindowProc 转成 `WM_CHAR` 送达（窗口类必须用 W 版本注册）。

### 6.9 anim.rs

- 所有动画都由 `SetTimer(内容窗口, ID_ANIM, 15)` 推进，**只调用 `set_alpha_and_pos`，不重绘**。
- **打开**：120ms，ease-out-cubic；内容窗口 alpha 0→255，y 从 +8 DIP 移到 0；背景幕 alpha 0→77。
- **关闭**（非 Launch）：80ms，alpha 降到 0，然后隐藏。
- `SPI_GETCLIENTAREAANIMATION == FALSE` 时直接跳到终态（5-#10）。
- 动画结束后 `KillTimer`。

### 6.10 corner.rs — 启动按钮（内嵌任务栏左端，唯一常驻元素）

- 窗口：`WS_POPUP` + `WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`。命中区域 28×28 DIP，窗口即命中区域，**不再留 `SHADOW_MARGIN` 透明边距**（按钮没有背景和阴影，见"绘制"）。
- **位置固定，不支持拖动**（questions.md #33）：只贴任务栏左端一处——`FindWindowW("Shell_TrayWnd")` 取主任务栏矩形，`x = 任务栏左边缘 + 8 DIP`、`y = 任务栏内垂直居中`（按按钮所在显示器 DPI 换算物理像素）。任务栏不存在（explorer 未启动 / 被 shell 替换）时退回桌面左下角，距左 10 DIP、距下 10 DIP。
  - 位置在 `WM_DISPLAYCHANGE` / `WM_SETTINGCHANGE(SPI_SETWORKAREA)` / `WM_DPICHANGED` 时重算，始终贴任务栏；没有拖动、没有网格吸附、不写位置设置。
- **层级**：`WM_WINDOWPOSCHANGING` 强制 `hwndInsertAfter = HWND_TOPMOST` —— 与任务栏同属 topmost 段并绘制在任务栏之上。
- **禁止 `SetParent` 到任务栏**：跨进程子窗口能创建成功、`PrintWindow` 也能取到正确画面，但任务栏自己的绘制流程会把它盖住，用户完全看不见（questions.md #31 实测）。只做"定位 + 层级"，窗口始终是本进程的顶层窗口，explorer 重启不会连带销毁。
- **绘制**：**无背景、无边框、无阴影**，只画居中的产品图形（6.12，18 DIP，默认 `PRIMARY`，Hover 变 `PRIMARY_HOVER`）。任务栏本身有底色，加白色卡片会像贴上去的一块，破坏"像系统自带"的目标（questions.md #33）。
  - 命中区先铺一层 `HIT_SHIELD`（`theme.rs`，`0x08000000`，3% 黑，肉眼不可见）：分层窗口按像素 alpha 判定命中，不铺这一层时四块图形之间 alpha=0 的缝隙会把点击漏给任务栏，点图标正中央反而没反应。**新增颜色必须同时加进 `RenderCtx` 的 palette 列表**，否则 `pal.get()` 找不到画刷、整个 `draw_corner` 失败、按钮直接不显示（questions.md #33）。
- **交互**：
  - 左键单击 → `pad::toggle(ShowReason::Corner)`（只区分按下/抬起，没有拖动判定）；
  - 右键 → 系统原生菜单【打开启动台 / 设置… / 退出】。

### 6.10.1 与任务栏的关系（为什么是覆盖窗口）

Windows 没有给第三方应用"往任务栏里加按钮"的公开接口：老的 DeskBand / 任务栏工具栏（`Shell_TrayWnd` + `TB_` 按钮）在 Windows 7 起被移除；Windows 11 的任务栏是 XAML + Composition 的私有实现，没有扩展点。官方可用的只有通知区域图标（`Shell_NotifyIcon`，且 Win11 默认折叠进溢出区），那不是任务栏按钮。因此这里采用"贴准位置 + 正确层级"的覆盖窗口，等价于视觉内嵌；注入 hook 或改 explorer 属于红线，禁止。

- **首次引导气泡**：
  - 独立的分层窗口，放在按钮右侧 8 DIP 处（右侧空间不够就放在上方）；
  - 样式：tofour Toast，`INPUT` 白底 + 1px `BORDER` + `RADIUS_TOAST` + L2 阴影；文字 `FS_BODY`；
  - 文案：「点击这里，或按 {hotkey.label()} 打开启动台」；
  - **消失条件**：点击气泡本身、点击角标、启动台被打开、或 8 秒后，任一满足即消失；
  - 消失后执行 `first_run_done = true; save_settings`。
### 6.11 settings_ui.rs — 设置窗口

- 窗口：分层窗口，tofour Modal 样式（白底 + `RADIUS_MODAL` + L3 阴影），主体 420×320 DIP，居中显示在鼠标所在显示器；`WS_EX_TOOLWINDOW`，不加 TOPMOST。
- 同一时间只存在一个设置窗口，重复打开时激活已有的那个。打开设置窗口时，启动台先隐藏。
- **布局**（距主体左边 24 DIP）：
  1. Header：标题「设置」（`FS_TITLE`/600），右上角 × 按钮，下方 1px `BORDER` 分隔线。按住 Header 可以拖动窗口。
  2. 快捷键：3 个单选项（`HotkeyPreset::ALL`，显示 `label()`），下方是说明文字（6.3），以及冲突时的行内错误。
  3. 开机自启：勾选框 →「开机时自动启动」。
  4. 首次引导：次要按钮 [重新显示引导]。
  5. Footer：上方 1px 分隔线，右对齐 [取消]（次要按钮）和 [保存]（主按钮）。
- **控件样式**：
  - 主按钮：`PRIMARY` 底、白字、高 32、`RADIUS_INPUT`，hover 用 `PRIMARY_HOVER`，按下用 `PRIMARY_PRESSED`；
  - 次要按钮：白底 + 1px `BORDER`，文字 `TEXT_PRIMARY`；
  - 勾选框和单选框：16px，选中时填充 `PRIMARY`；
  - 键盘焦点环：1px `PRIMARY`，与控件间隔 2px。
- **键盘**：Tab / Shift+Tab 切换控件，Space 切换勾选，Enter = 保存，Esc = 取消。
- **保存**：
  1. 热键有变化 → `hotkey::change`，失败时停在窗口上并显示错误；
  2. `eng.save_settings`；
  3. 点过重新显示引导 → `first_run_done = false`，弹出气泡；
  4. 关闭窗口。

### 6.12 glyph.rs — 产品图形

- 图形：2×2 的四个圆角小方块，每块边长为整体的 7/16，间隔 2/16，圆角为边长的 2/7，颜色 `PRIMARY`。
- 角标中显示为 16 DIP。
- 托盘图标在运行时生成：用 D2D 画到 32bpp DIB 上，再 `CreateIconIndirect`（颜色位图加全 0 的 mask）得到 HICON，尺寸取 `SM_CXSMICON`（按 DPI）。**不内嵌 .ico 资源**，所以 v0.1 没有 build.rs。

### 6.13 fmt.rs（纯函数，必须有单元测试）

- **相对时间** `rel_time(mtime, now)`：
  - 小于 60 秒 →「刚刚」
  - 小于 60 分钟 →「N 分钟前」
  - 同一天 →「今天 HH:MM」
  - 昨天 →「昨天 HH:MM」
  - 小于 7 天 →「N 天前」
  - 同一年 →「M月D日」
  - 其他 →「YYYY/M/D」
- **错误文案** `error_text(&EngineError)`：

| EngineError | 文案 |
|---|---|
| `LaunchFailed{name, ..}` | 「无法启动 {name}」 |
| `PathNotFound(p)` | 「找不到 {文件名}，它可能已被移动或删除」 |
| `UnknownApp(_)` | 「这个应用已不存在」 |
| 其他 | 「出错了，详情见日志」 |

## 7. 退出流程

托盘或角标菜单点「退出」→ 销毁设置窗口、气泡、角标、启动台两个窗口 → `Shell_NotifyIconW(NIM_DELETE)` → `UnregisterHotKey` → `WTSUnRegisterSessionNotification` → `eng.shutdown()` → `PostQuitMessage(0)`。

## 8. 依赖（crates/app/Cargo.toml，原样照抄）

```toml
[package]
name = "deskpad-app"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "DeskPad"
path = "src/main.rs"

[dependencies]
deskpad-engine = { path = "../engine" }
windows = { version = "=0.58.0", features = [
  "Win32_Foundation",
  "Win32_Graphics_Direct2D",
  "Win32_Graphics_Direct2D_Common",
  "Win32_Graphics_DirectWrite",
  "Win32_Graphics_Dxgi_Common",
  "Win32_Graphics_Gdi",
  "Win32_Graphics_Imaging",
  "Win32_System_Com",
  "Win32_System_DataExchange",
  "Win32_System_LibraryLoader",
  "Win32_System_Memory",
  "Win32_System_Ole",
  "Win32_System_RemoteDesktop",
  "Win32_UI_HiDpi",
  "Win32_UI_Input_Ime",
  "Win32_UI_Input_KeyboardAndMouse",
  "Win32_UI_Shell",
  "Win32_UI_WindowsAndMessaging",
] }
```

> 以上 feature 都已对照 windows 0.58.0 源码核对过。v1 漏掉的几项：
> - `D2D1_PIXEL_FORMAT` 和 `CreateDCRenderTarget` 需要 `Win32_Graphics_Dxgi_Common`；
> - 输入法相关需要 `Win32_UI_Input_Ime`；
> - 剪贴板需要 `Win32_System_DataExchange` + `Win32_System_Memory`，`CF_UNICODETEXT` 在 `Win32_System_Ole`；
> - 锁屏通知需要 `Win32_System_RemoteDesktop`。
>
> UI **不依赖 image crate**（PNG 由 WIC 解码），也不依赖 anyhow 等其他 crate。版本锁定 `=0.58.0`，与引擎一致。v1 里「卡住超过一天允许降级 GDI+」的条款**作废**：软件 D2D 路线已经确定，有问题记进 questions.md。

## 9. 内存预算（与功能同等重要）

| 项 | 预算 | 手段 |
|---|---|---|
| 启动台隐藏时常驻（专用工作集） | **目标 < 15 MB，红线 < 20 MB** | 隐藏时释放 DIB、RT、图标位图、几何缓存（6.5）；软件 D2D，不加载显卡驱动（4.4）；UI 不开后台线程 |
| 启动台显示时的增量 | < 12 MB（1080p、150% 缩放） | 背景幕零位图（6.5）；DIB 只覆盖面板区域；图标只解码可见格（约 40 个 × 36 KB） |
| 空闲 CPU | 0（隐藏时没有任何定时器） | 只有事件驱动；引擎靠目录变更通知，不轮询 |
| exe 体积 | 目标 ≤ 3 MB，红线 < 5 MB | release 配置见引擎文档第 2 节 |

**禁止的做法**：
- 用 `EmptyWorkingSet` 或 `SetProcessWorkingSetSize` 人为压低工作集——这只是让数字好看，下次打开会变慢。
- 常驻缓存全部应用图标的位图。
- 使用 WebView、XAML Islands、WinUI 等任何重型 UI 框架。

## 10. 里程碑与验收

| 里程碑 | 内容 | 验收 |
|---|---|---|
| U1 | 引导、主窗口、托盘、热键、两窗口空启动台（只有背景幕和三个空面板）；基于 E0 桩 | Alt+Space 能开能关；不出现在任务栏和 Alt+Tab 中；托盘「退出」后没有残留进程；二次双击 exe 会打开已运行的启动台；5-#8（重启资源管理器后托盘图标恢复） |
| U2 | surface 与 render、smooth.rs、三块面板、应用网格接入真实数据、图标 | 视觉逐项对照第 4 节（对照原型截图 `preview-v2-launchpad-tofour.png`）；平滑圆角没有折痕；点击应用能启动；首帧 ≤ 120ms（日志里记录 show 到第一次 present 的耗时） |
| U3 | 搜索、键盘、输入法、粘贴 | 输入 "ps" 实时过滤；用中文输入法输入「微信」能搜到；候选窗贴着搜索框光标；Enter 启动；Esc 先清空、再关闭；方向键、Tab、Home/End 都正常 |
| U4 | 文件卡、文件夹卡、滚动、动画、DPI | 最近文件和常用文件夹数据正确；100%、150%、200% 缩放下都清晰；跨不同 DPI 的显示器呼出都正常；关闭系统动画后没有任何过渡 |
| U5 | 角标按钮、引导气泡、设置窗口 | 角标可以拖动，重启后位置保留；拔掉副屏后角标回到主屏；首次运行出现一次气泡，`--autostart` 时不出现；设置窗口的每一项都生效；热键改成 Ctrl+Alt+Space 后立即可用 |
| U6 | 第 5 节协调项全部过一遍 | 5-#1～#17 逐条手测，结果记录在 `docs/u6-checklist.md`（每条写：通过/不通过 + 测试方法）；全屏游戏中按 Alt+Space 不会切出游戏 |

性能红线：从热键到可交互 ≤ 120ms；隐藏状态下 CPU 为 0；内存见第 9 节。

## 11. 与引擎板块联调

1. U1 基于 E0 契约桩开发，U2 开始接真实引擎（两边在同一个 workspace，一次 `cargo build` 同时编译）。
2. 集成日检查单：
   - `cargo build --release` 通过，产物为 `target\release\DeskPad.exe`；
   - 完整走一遍：Alt+Space 呼出 → 输入 "ps" → 回车启动 Photoshop（第一次排序不对时，选中它启动一次，之后 "ps" 的首条就会是它）；
   - 视觉对照 tofour：面板 #FAFAFA、1px #E5E5E5 边框、10px 平滑圆角、蓝色只出现在交互状态，**没有毛玻璃、没有渐变、没有胶囊圆角**；
   - 托盘勾选「开机自启」后注册表 Run 键里出现 DeskPad，取消后消失；在任务管理器里禁用后，设置窗口显示为未勾选；
   - 桌面上除了左下角的按钮之外没有任何常驻元素；首次引导只出现一次（删除 config.json 后重启可以复现）；
   - 第 9 节内存与 CPU 指标全部达标；
   - **红线 grep 自查**：`rg "SetWindowsHookEx|WriteProcessMemory|ReadProcessMemory|CreateRemoteThread|GetAsyncKeyState" crates` 无结果。
