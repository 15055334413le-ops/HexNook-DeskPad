//! DeskPad 引擎：所有「找数据、管数据、执行动作」的代码，不创建任何窗口。
//!
//! 开发规范：docs/engine-architecture.md。引擎不做窗口/消息循环/渲染/全局热键，
//! 必须可无头验证（bin/probe.rs）。
pub mod log;
pub mod model;
pub mod search;
pub mod single;
pub mod update;
// 以下为私有模块
mod apps;
mod autostart;
mod dirs;
mod docthumb;
mod filekind;
mod icons;
mod keys;
mod launch;
mod recent;
mod store;
mod win;

pub use model::*;

/// PDF 缩略图子进程入口（app main 识别 `--thumb-pdf` 后调用）。
pub fn thumb_pdf_worker(args: &[String]) -> i32 {
    docthumb::pdf_worker_main(args, icons::THUMB_MAX)
}

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Storage::FileSystem::{
    FILE_NOTIFY_CHANGE, FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME,
    FILE_NOTIFY_CHANGE_LAST_WRITE,
};
use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, WaitForMultipleObjects, INFINITE,
    THREAD_MODE_BACKGROUND_BEGIN,
};
use windows::Win32::UI::Shell::{
    FOLDERID_CommonPrograms, FOLDERID_Desktop, FOLDERID_Programs, FOLDERID_PublicDesktop,
    FOLDERID_Recent,
};

/// 待处理扫描范围位标志（AtomicU8，engine 文档 4.8.3）。
pub(crate) const SCAN_APPS: u8 = 1;
pub(crate) const SCAN_FILES: u8 = 2;
/// refresh() 发起的应用重扫：距上次应用扫描不足 APPS_SOFT_MIN 时跳过（开始菜单/桌面的变化已由目录监听即时覆盖，
/// 这里只为发现 UWP 新装应用）。每次打开启动台都全量枚举 AppsFolder 会白白多占 3~4MB 内存和 CPU。
pub(crate) const SCAN_APPS_SOFT: u8 = 4;
const APPS_SOFT_MIN: std::time::Duration = std::time::Duration::from_secs(300);

pub(crate) struct Inner {
    snap: RwLock<Snapshot>,
    settings: Mutex<Settings>,
    usage: Mutex<search::Usage>,
    on_change: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    refresh_event: win::OwnedHandle,  // 自动复位事件
    shutdown_event: win::OwnedHandle, // 手动复位事件
    worker: Mutex<Option<JoinHandle<()>>>,
    data_dir: PathBuf,
    pending_scan: AtomicU8,
    first_apps: Mutex<bool>,
    first_apps_cv: Condvar,
}

/// 锁毒化处理：锁内数据始终是合法状态，毒化后直接取回数据继续（unwrap 的替代）。
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

pub(crate) fn read_lock<T>(m: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    match m.read() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

pub(crate) fn write_lock<T>(m: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    match m.write() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

/// 保证：Clone + Send + Sync。内部为 Arc<Inner>（4.8）。
#[derive(Clone)]
pub struct EngineHandle {
    inner: Arc<Inner>,
}

impl EngineHandle {
    /// 不要求调用线程已初始化 COM。流程见 4.8.1。
    /// 有 index.json 缓存时 ≤ 50ms 返回；首次运行最多阻塞 3 秒，等待应用列表就绪。
    pub fn init() -> Result<EngineHandle, EngineError> {
        // 1. 创建数据目录，初始化日志。
        let data_dir = store::data_dir()?;
        std::fs::create_dir_all(&data_dir)
            .map_err(|e| EngineError::Io(format!("create_dir_all {:?}: {e}", data_dir)))?;
        std::fs::create_dir_all(data_dir.join("icons"))
            .map_err(|e| EngineError::Io(format!("create icons dir: {e}")))?;
        log::init(&data_dir);
        log::info(&format!(
            "init v{} exe={}",
            env!("CARGO_PKG_VERSION"),
            win::exe_path()
        ));

        // 2. 加载 config.json → 钳制 → autostart 真实状态覆盖。
        let mut settings = store::load_settings(&data_dir);
        autostart::sync_on_init(&mut settings);

        // 3. 加载 usage.json。
        let usage = store::load_usage(&data_dir);

        // 4. 加载 index.json：存在 → generation=1、ready=true；不存在 → 空快照。
        let cached_apps = store::load_index(&data_dir);
        let had_index = cached_apps.is_some();
        let snap = match cached_apps {
            Some(apps) => Snapshot {
                generation: 1,
                ready: true,
                apps: Arc::new(apps),
                recent: Arc::new(Vec::new()),
                dirs: Arc::new(Vec::new()),
            },
            None => Snapshot::default(),
        };

        // 5. 启动扫描线程（4.8.2）。
        let inner = Arc::new(Inner {
            snap: RwLock::new(snap),
            settings: Mutex::new(settings),
            usage: Mutex::new(usage),
            on_change: Mutex::new(None),
            refresh_event: win::OwnedHandle::event(false)?,
            shutdown_event: win::OwnedHandle::event(true)?,
            worker: Mutex::new(None),
            data_dir,
            pending_scan: AtomicU8::new(SCAN_APPS | SCAN_FILES),
            first_apps: Mutex::new(false),
            first_apps_cv: Condvar::new(),
        });
        let worker_inner = Arc::clone(&inner);
        let worker = std::thread::Builder::new()
            .name("deskpad-scan".into())
            .spawn(move || worker_main(worker_inner))
            .map_err(|e| EngineError::Io(format!("spawn scan thread: {e}")))?;
        *lock(&inner.worker) = Some(worker);

        // 6. index.json 不存在时最多等 3 秒，直到第一次 APPS 阶段完成发布；超时照常返回。
        if !had_index {
            let deadline = std::time::Duration::from_secs(3);
            let start = std::time::Instant::now();
            let mut first = lock(&inner.first_apps);
            while !*first {
                let remain = deadline.saturating_sub(start.elapsed());
                if remain.is_zero() {
                    break;
                }
                let (g, to) = match inner.first_apps_cv.wait_timeout(first, remain) {
                    Ok(r) => r,
                    Err(p) => p.into_inner(),
                };
                first = g;
                if to.timed_out() {
                    break;
                }
            }
        }
        Ok(EngineHandle { inner })
    }

    /// 永不阻塞（只克隆几个 Arc）。
    pub fn snapshot(&self) -> Snapshot {
        read_lock(&self.inner.snap).clone()
    }

    /// 每次发布新快照后，在**引擎扫描线程**上调用 f。f 必须很快返回，UI 在里面只做 PostMessageW。
    /// 重复调用会替换旧的回调。
    pub fn set_on_change(&self, f: Box<dyn Fn() + Send + Sync + 'static>) {
        *lock(&self.inner.on_change) = Some(f);
    }

    /// 请求后台重扫（APPS + FILES 两类，见 4.8.3），非阻塞，自带去抖。UI 每次打开启动台都调用一次。
    pub fn refresh(&self) {
        self.inner.pending_scan.fetch_or(SCAN_APPS_SOFT | SCAN_FILES, Ordering::SeqCst);
        win::set_event(self.inner.refresh_event.get());
    }

    /// 返回 snap.apps 的下标，按相关度降序。内部调用 search::search(&snap.apps, &usage, query)。
    pub fn search(&self, snap: &Snapshot, query: &str) -> Vec<usize> {
        let usage = lock(&self.inner.usage);
        search::search(&snap.apps, &usage, query)
    }

    /// 调用线程必须已经 CoInitializeEx(STA)（UI 主线程和 probe 的 main 都满足）。
    /// from_query：用户是通过哪个查询启动的；没有查询时传 ""。成功后更新 usage.json。
    pub fn launch_app(&self, id: AppId, from_query: &str) -> Result<(), EngineError> {
        let entry = {
            let snap = read_lock(&self.inner.snap);
            snap.apps.iter().find(|a| a.id == id).cloned()
        };
        let Some(entry) = entry else { return Err(EngineError::UnknownApp(id)) };
        let file = match &entry.launch {
            LaunchTarget::AppsFolder(p) => format!("shell:AppsFolder\\{p}"),
            LaunchTarget::Shortcut(path) => path.to_string_lossy().into_owned(),
        };
        launch::execute(&file, &entry.name)?;
        let usage = {
            let mut u = lock(&self.inner.usage);
            *u.launch_counts.entry(id).or_insert(0) += 1;
            let q = search::normalize_query(from_query);
            if !q.is_empty() {
                u.query_picks.insert(q, id);
            }
            u.clone()
        };
        if let Err(e) = store::save_usage(&self.inner.data_dir, &usage) {
            log::warn(&format!("save usage.json failed: {e:?}"));
        }
        Ok(())
    }

    /// 打开文件或文件夹。调用线程要求同 launch_app。
    pub fn open_path(&self, path: &std::path::Path) -> Result<(), EngineError> {
        if !path.exists() {
            return Err(EngineError::PathNotFound(path.to_path_buf()));
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        launch::execute(&path.to_string_lossy(), &name)
    }

    pub fn settings(&self) -> Settings {
        lock(&self.inner.settings).clone()
    }

    /// 钳制字段 → 原子写 config.json → autostart 变化时写注册表 →
    /// max_recent 变化时触发 FILES 重扫。
    pub fn save_settings(&self, s: &Settings) -> Result<(), EngineError> {
        let mut s = s.clone();
        s.max_recent = s.max_recent.clamp(1, 20);
        let (old_autostart, old_max) = {
            let g = lock(&self.inner.settings);
            (g.autostart, g.max_recent)
        };
        store::save_settings(&self.inner.data_dir, &s)?;
        *lock(&self.inner.settings) = s.clone();
        if s.autostart != old_autostart {
            autostart::set(s.autostart)?;
        }
        if s.max_recent != old_max {
            self.inner.pending_scan.fetch_or(SCAN_FILES, Ordering::SeqCst);
            win::set_event(self.inner.refresh_event.get());
        }
        Ok(())
    }

    /// 通知后台线程退出并 join，最多等 1 秒。UI 在退出前调用。
    pub fn shutdown(&self) {
        win::set_event(self.inner.shutdown_event.get());
        let worker = lock(&self.inner.worker).take();
        if let Some(h) = worker {
            let start = std::time::Instant::now();
            while !h.is_finished() && start.elapsed() < std::time::Duration::from_secs(1) {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            if h.is_finished() {
                let _ = h.join();
            }
            // 超过 1 秒仍未退出：drop JoinHandle（detach），满足「最多等 1 秒」。
        }
    }
}

/// 发布 = 获取写锁 → generation += 1 → 替换字段 → 释放锁 → 调用 on_change（4.8.2）。
fn publish_apps(inner: &Inner, apps: Vec<AppEntry>, ready: bool) {
    {
        let mut snap = write_lock(&inner.snap);
        snap.generation += 1;
        snap.apps = Arc::new(apps);
        snap.ready = ready;
    }
    fire_on_change(inner);
}

/// 调用回调前必须先释放所有锁（包括 on_change 锁本身）。
pub(crate) fn fire_on_change(inner: &Inner) {
    let cb = lock(&inner.on_change).take();
    if let Some(f) = cb {
        f();
        *lock(&inner.on_change) = Some(f);
    }
}

/// 图标批次收尾的最终发布：generation += 1 → 释放锁 → 回调。
pub(crate) fn publish_current(inner: &Inner) {
    {
        write_lock(&inner.snap).generation += 1;
    }
    fire_on_change(inner);
}

/// 首次 APPS 发布后唤醒 init 的 3 秒等待（4.8.1 第 6 步）。
fn notify_first_apps(inner: &Inner) {
    *lock(&inner.first_apps) = true;
    inner.first_apps_cv.notify_all();
}

/// 一个被监听的目录：句柄 + 触发的扫描范围（engine 文档 4.8.3 表）。
struct Watch {
    handle: win::ChangeHandle,
    scope: u8,
}

fn watch_list() -> Vec<Watch> {
    let names = FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE;
    let table: [(&windows::core::GUID, bool, FILE_NOTIFY_CHANGE, u8); 5] = [
        (&FOLDERID_Programs, true, names, SCAN_APPS),
        (&FOLDERID_CommonPrograms, true, names, SCAN_APPS),
        (&FOLDERID_Desktop, false, names, SCAN_APPS | SCAN_FILES),
        (&FOLDERID_PublicDesktop, false, names, SCAN_APPS),
        (&FOLDERID_Recent, false, FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE, SCAN_FILES),
    ];
    let mut out = Vec::new();
    for (guid, subtree, filter, scope) in table {
        let Some(dir) = win::known_folder(guid) else { continue };
        match win::ChangeHandle::new(&dir.to_string_lossy(), subtree, filter) {
            Ok(handle) => out.push(Watch { handle, scope }),
            Err(e) => log::warn(&format!("watch {dir:?} failed: {e:?}")),
        }
    }
    out
}

/// 扫描线程主体（4.8.2）。只有这一个后台线程。
fn worker_main(inner: Arc<Inner>) {
    let _com = win::ComInit::sta();
    // 降低 CPU 和磁盘 IO 优先级，扫描时不和前台程序抢资源。
    // SAFETY: GetCurrentThread 返回伪句柄，仅本线程使用；失败只影响调度优先级。
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
    let mut failures: HashMap<AppId, u32> = HashMap::new();
    let mut stale: Vec<AppId> = Vec::new();
    let watches = watch_list();
    // 句柄顺序：0 = shutdown，1 = refresh，2.. = 目录监听
    let mut handles: Vec<HANDLE> = vec![inner.shutdown_event.get(), inner.refresh_event.get()];
    handles.extend(watches.iter().map(|w| w.handle.get()));

    let mut pending = inner.pending_scan.swap(0, Ordering::SeqCst) | SCAN_APPS | SCAN_FILES;
    let min_gap = std::time::Duration::from_secs(3);
    let mut last_apps = std::time::Instant::now();
    loop {
        if pending & SCAN_APPS_SOFT != 0 && last_apps.elapsed() >= APPS_SOFT_MIN {
            pending |= SCAN_APPS;
        }
        if pending & SCAN_APPS != 0 {
            apps_phase(&inner, &mut stale);
            last_apps = std::time::Instant::now();
        }
        if pending & SCAN_FILES != 0 {
            files_phase(&inner);
        }
        if !icons_fill_phase(&inner, &mut failures, &stale) {
            break; // 提取图标途中收到 shutdown
        }
        stale.clear();
        let scan_end = std::time::Instant::now();
        log::info(&format!("scan done private_ws={:.1}MB", win::private_ws_mb()));

        // 无超时等待第一个信号
        pending = match wait_signal(&inner, &handles, &watches, INFINITE) {
            Signal::Shutdown => break,
            Signal::Scope(s) => s,
            Signal::Timeout => 0,
        };
        // 去抖：连续 1000ms 没有新信号才开始扫描
        loop {
            match wait_signal(&inner, &handles, &watches, 1000) {
                Signal::Shutdown => {
                    log::info("scan thread exit");
                    return;
                }
                Signal::Scope(s) => pending |= s,
                Signal::Timeout => break,
            }
        }
        // 距上次扫描结束不足 3 秒 → 补睡到满 3 秒（期间仍响应 shutdown）
        let since = scan_end.elapsed();
        if since < min_gap {
            let ms = (min_gap - since).as_millis() as u32;
            if win::wait_one(inner.shutdown_event.get(), ms) {
                break;
            }
        }
        pending |= inner.pending_scan.swap(0, Ordering::SeqCst);
    }
    log::info("scan thread exit");
}

enum Signal {
    Shutdown,
    Scope(u8),
    Timeout,
}

fn wait_signal(inner: &Inner, handles: &[HANDLE], watches: &[Watch], timeout_ms: u32) -> Signal {
    // SAFETY: 所有句柄由 Inner / watches 持有且在等待期间存活；等待不转移所有权。
    let w = unsafe { WaitForMultipleObjects(handles, false, timeout_ms) };
    if w == WAIT_TIMEOUT {
        return Signal::Timeout;
    }
    let idx = w.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
    match idx {
        0 => Signal::Shutdown,
        1 => Signal::Scope(inner.pending_scan.swap(0, Ordering::SeqCst)),
        i if i - 2 < watches.len() => {
            let wch = &watches[i - 2];
            wch.handle.rearm();
            Signal::Scope(wch.scope)
        }
        _ => {
            // WAIT_FAILED 等异常：避免空转，睡一下当作超时
            std::thread::sleep(std::time::Duration::from_millis(200));
            Signal::Timeout
        }
    }
}

/// FILES 阶段：最近文件 + 常用文件夹 → 发布。
fn files_phase(inner: &Inner) {
    let max = lock(&inner.settings).max_recent;
    let mut recent = recent::scan(max);
    let dirs = dirs::scan();
    // 内容缩略图：只给「看内容才认得出」的类型取（缓存按路径+修改时间）
    let mut pdf_jobs = Vec::new();
    for (i, f) in recent.iter_mut().enumerate() {
        if matches!(f.kind, FileKind::Image | FileKind::Video | FileKind::Presentation | FileKind::Pdf) {
            match icons::thumbnail(&inner.data_dir, &f.path, f.mtime) {
                icons::Thumb::Ready(p) => f.thumb_png = Some(p),
                icons::Thumb::NeedPdf(out) => pdf_jobs.push((i, out)),
                icons::Thumb::Missing => {}
            }
        }
    }
    // 系统没有 PDF 缩略图处理器时：一次性子进程用 Windows.Data.Pdf 渲染首页（每轮最多 8 个）
    pdf_jobs.truncate(8);
    if !pdf_jobs.is_empty() {
        let jobs: Vec<_> = pdf_jobs.iter().map(|(i, o)| (recent[*i].path.clone(), o.clone())).collect();
        docthumb::render_pdfs(&jobs);
        for (i, out) in pdf_jobs {
            if out.exists() {
                recent[i].thumb_png = Some(out);
            } else {
                icons::mark_no_thumb(&inner.data_dir, &recent[i].path, recent[i].mtime);
            }
        }
    }
    icons::prune_thumbnails(&inner.data_dir);
    {
        let mut snap = write_lock(&inner.snap);
        snap.generation += 1;
        snap.recent = Arc::new(recent);
        snap.dirs = Arc::new(dirs);
    }
    fire_on_change(inner);
}

/// APPS 阶段：全量扫描 → 缓存图标解析 → 发布（ready=true）→ 写 index.json。
fn apps_phase(inner: &Inner, stale: &mut Vec<AppId>) {
    let mut apps = apps::scan();
    *stale = icons::resolve(&mut apps, &inner.data_dir);
    publish_apps(inner, apps.clone(), true);
    if let Err(e) = store::save_index(&inner.data_dir, &apps) {
        log::warn(&format!("save index.json failed: {e:?}"));
    }
    notify_first_apps(inner);
}

/// 图标补全阶段：缺失 + 过期逐个提取，每 40 个发布一次，全部完成后再发布一次并写 index.json。
/// 返回 false 表示提取途中收到 shutdown。
fn icons_fill_phase(inner: &Inner, failures: &mut HashMap<AppId, u32>, stale: &[AppId]) -> bool {
    let targets: Vec<(AppId, LaunchTarget)> = {
        let snap = read_lock(&inner.snap);
        snap.apps
            .iter()
            .filter(|a| a.icon_png.is_none() || stale.contains(&a.id))
            .map(|a| (a.id, a.launch.clone()))
            .collect()
    };
    if targets.is_empty() {
        return true;
    }
    let n = icons::fill_missing(inner, &targets, stale, failures);
    if n > 0 {
        publish_current(inner);
        let apps = read_lock(&inner.snap).apps.clone();
        if let Err(e) = store::save_index(&inner.data_dir, apps.as_slice()) {
            log::warn(&format!("save index.json failed: {e:?}"));
        }
    }
    !win::wait_one(inner.shutdown_event.get(), 0)
}

#[cfg(test)]
mod t {
    use super::model::*; use super::search::*;
    fn app(id: u64, name: &str, initials: &str, alias: &str) -> AppEntry {
        AppEntry { id, name: name.into(), launch: LaunchTarget::AppsFolder(name.into()), icon_png: None,
          source: AppSource::StartMenu, keys: SearchKeys { name_lower: name.to_lowercase(), initials: initials.into(), alias: alias.into(), sort_key: name.to_lowercase() } }
    }
    #[test] fn settings_roundtrip() {
        let s = Settings { max_recent: 12, ..Default::default() };
        let j = serde_json::to_string_pretty(&s).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&j).unwrap(), s);
        // 缺字段、多出未知字段的旧配置也必须能读，且已有字段保留
        let old: Settings = serde_json::from_str(r#"{"autostart":true,"unknown_future":1}"#).unwrap();
        assert!(old.autostart && old.max_recent == 8 && old.hotkey == HotkeyPreset::AltSpace);
    }
    #[test] fn ranking() {
        let apps = vec![
            app(1, "Adobe Photoshop 2025", "ap2", "photoshop"),
            app(2, "Windows PowerShell", "wps", "powershell"),
            app(3, "微信", "wx", "weixin"),
            app(4, "企业微信", "qywx", "wxwork"),
            app(5, "Visual Studio Code", "vsc", "code"),
            app(6, "WPS Office", "wo", "wps"),
        ];
        let mut u = Usage::default();
        let names = |v: Vec<usize>| v.iter().map(|&i| apps[i].name.clone()).collect::<Vec<_>>();
        println!("ps  {:?}", names(search(&apps, &u, "ps")));
        println!("wx  {:?}", names(search(&apps, &u, "wx")));
        println!("vsc {:?}", names(search(&apps, &u, "vsc")));
        println!("code {:?}", names(search(&apps, &u, "code")));
        println!("photo {:?}", names(search(&apps, &u, "photo")));
        println!("微 {:?}", names(search(&apps, &u, "微")));
        assert_eq!(search(&apps, &u, "wx")[0], 2);
        u.query_picks.insert("ps".into(), 1);
        assert_eq!(apps[search(&apps, &u, "ps")[0]].id, 1);
        assert_eq!(apps[search(&apps, &u, " P S ")[0]].id, 1);
        assert_eq!(word_starts("PowerShell"), vec![0,5]);
        assert_eq!(word_starts("Photoshop2025"), vec![0,9]);
    }
}
