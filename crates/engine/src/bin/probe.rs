//! 无头验证工具（engine 文档 4.10）。命令随里程碑补齐；出错时退出码 1 并打到 stderr。
use std::path::PathBuf;
use std::process;

use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{SHGetKnownFolderPath, FOLDERID_LocalAppData, KNOWN_FOLDER_FLAG};

use deskpad_engine::{AppSource, EngineHandle};

struct ComSta {
    ok: bool,
}

impl ComSta {
    fn new() -> Self {
        // SAFETY: main 线程一次性初始化；ok（含 S_FALSE）时 Drop 配对 CoUninitialize。
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        Self { ok: hr.is_ok() }
    }
}

impl Drop for ComSta {
    fn drop(&mut self) {
        if self.ok {
            // SAFETY: 与 new() 中成功的 CoInitializeEx 一一配对。
            unsafe {
                CoUninitialize();
            }
        }
    }
}

fn main() {
    let _com = ComSta::new();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(msg) = dispatch(&args) {
        eprintln!("probe: {msg}");
        process::exit(1);
    }
}

fn dispatch(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("settings") => cmd_settings(),
        Some("apps") => cmd_apps(&args[1..]),
        Some("icons") => cmd_icons(),
        Some("recent") => cmd_recent(),
        Some("dirs") => cmd_dirs(),
        Some("search") => cmd_search(args.get(1).map(String::as_str).unwrap_or("")),
        Some("pick") => cmd_pick(args.get(1), args.get(2)),
        Some("launch") => cmd_launch(args.get(1)),
        Some("open") => cmd_open(args.get(1)),
        Some("watch") => cmd_watch(args.get(1)),
        Some("refresh-loop") => cmd_refresh_loop(args.get(1)),
        Some(other) => Err(format!("未知命令: {other}")),
        None => Err("用法: probe <settings|apps|icons|recent|dirs|search|pick|launch|open|watch|refresh-loop> [参数]".into()),
    }
}

/// probe icons：等图标补全（最多 60 秒），打印命中率并列出缺失项。
fn cmd_icons() -> Result<(), String> {
    use std::time::{Duration, Instant};
    let engine = EngineHandle::init().map_err(|e| e.to_string())?;
    let start = Instant::now();
    let mut last_gen = 0u64;
    let mut last_change = Instant::now();
    loop {
        let snap = engine.snapshot();
        let missing = snap.apps.iter().filter(|a| a.icon_png.is_none()).count();
        if snap.generation != last_gen {
            last_gen = snap.generation;
            last_change = Instant::now();
        }
        if missing == 0 || start.elapsed() > Duration::from_secs(60) {
            break;
        }
        // 快照 5 秒无变化 → 认为补全结束（剩余为失败重试上限项）。
        if start.elapsed() > Duration::from_secs(2) && last_change.elapsed() > Duration::from_secs(5) {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let snap = engine.snapshot();
    let total = snap.apps.len();
    let with = snap.apps.iter().filter(|a| a.icon_png.is_some()).count();
    let rate = if total == 0 { 100.0 } else { with as f64 * 100.0 / total as f64 };
    println!("icons: {with}/{total} hit_rate={rate:.1}%");
    for a in snap.apps.iter().filter(|a| a.icon_png.is_none()) {
        println!("missing: {:016x} | {}", a.id, a.name);
    }
    engine.shutdown();
    Ok(())
}

/// probe apps [--ids] [--all]：默认只列出启动台可见应用；--all 含隐藏和系统项。
fn cmd_apps(args: &[String]) -> Result<(), String> {
    let ids_only = args.iter().any(|a| a == "--ids");
    let all = args.iter().any(|a| a == "--all");
    let engine = EngineHandle::init().map_err(|e| e.to_string())?;
    let snap = engine.snapshot();
    let apps = if all { &snap.all_apps } else { &snap.apps };
    if ids_only {
        for a in apps.iter() {
            println!("{:016x}", a.id);
        }
    } else {
        println!("count={} ready={}", apps.len(), snap.ready);
        for a in apps.iter() {
            let src = match a.source {
                AppSource::StartMenu => "StartMenu",
                AppSource::Desktop => "Desktop",
                AppSource::ExtraScanDir => "ExtraScanDir",
                AppSource::Manual => "Manual",
            };
            let icon = if a.icon_png.is_some() { "有" } else { "无" };
            let mut tags = Vec::new();
            if a.is_system {
                tags.push("[system]");
            }
            if a.hidden {
                tags.push("[hidden]");
            }
            let tags = if tags.is_empty() {
                String::new()
            } else {
                format!(" {}", tags.join(" "))
            };
            println!(
                "{:016x} | {} | {}{} | {} | {} | 图标:{}",
                a.id, src, a.name, tags, a.keys.alias, a.keys.initials, icon
            );
        }
    }
    engine.shutdown();
    Ok(())
}

fn local_appdata() -> Result<PathBuf, String> {
    // SAFETY: pw 由系统分配，CoTaskMemFree 恰好释放一次。
    unsafe {
        let pw = SHGetKnownFolderPath(&FOLDERID_LocalAppData, KNOWN_FOLDER_FLAG(0), None)
            .map_err(|e| e.to_string())?;
        let s = pw.to_string().unwrap_or_default();
        CoTaskMemFree(Some(pw.as_ptr().cast()));
        Ok(PathBuf::from(s))
    }
}

/// 打印 config.json 当前内容与生效设置，autostart 用注册表真实状态（init 已覆盖）。
fn cmd_settings() -> Result<(), String> {
    let dir = local_appdata()?.join("DeskPad");
    match std::fs::read_to_string(dir.join("config.json")) {
        Ok(text) => println!("--- config.json ---\n{text}"),
        Err(_) => println!("--- config.json 不存在 ---"),
    }
    let engine = EngineHandle::init().map_err(|e| e.to_string())?;
    let s = engine.settings();
    println!("--- 生效设置（钳制后） ---");
    println!("{}", serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?);
    println!("autostart_real={}", s.autostart);
    engine.shutdown();
    Ok(())
}

/// 等第一次 FILES 阶段发布（recent/dirs 非空或 3 秒超时）。
fn wait_files(engine: &EngineHandle) -> deskpad_engine::Snapshot {
    let start = std::time::Instant::now();
    loop {
        let snap = engine.snapshot();
        if !snap.dirs.is_empty() || start.elapsed() > std::time::Duration::from_secs(3) {
            return snap;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn fmt_local(unix: i64) -> String {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let t = (unix + 11_644_473_600) * 10_000_000;
    let ft = FILETIME { dwLowDateTime: t as u32, dwHighDateTime: (t >> 32) as u32 };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    // SAFETY: 输入输出结构体都在栈上。
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc).is_err() || SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err() {
            return unix.to_string();
        }
    }
    format!("{:04}-{:02}-{:02} {:02}:{:02}", local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute)
}

/// probe recent：每行 mtime(本地时间) | kind | 路径。
fn cmd_recent() -> Result<(), String> {
    let engine = EngineHandle::init().map_err(|e| e.to_string())?;
    let snap = wait_files(&engine);
    println!("count={}", snap.recent.len());
    for f in snap.recent.iter() {
        println!("{} | {:?} | {}", fmt_local(f.mtime), f.kind, f.path.display());
    }
    engine.shutdown();
    Ok(())
}

/// probe dirs：每行 count | 名称 | 路径。
fn cmd_dirs() -> Result<(), String> {
    let engine = EngineHandle::init().map_err(|e| e.to_string())?;
    let snap = wait_files(&engine);
    for d in snap.dirs.iter() {
        println!("{} | {} | {}", d.count, d.name, d.path.display());
    }
    engine.shutdown();
    Ok(())
}

fn usage_path() -> Result<PathBuf, String> {
    Ok(local_appdata()?.join("DeskPad").join("usage.json"))
}

fn load_usage() -> deskpad_engine::search::Usage {
    usage_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// probe search <q>：前 10 条，分数 | 名称（分数含 pick 加成）。
fn cmd_search(q: &str) -> Result<(), String> {
    use deskpad_engine::search::{normalize_query, score};
    let engine = EngineHandle::init().map_err(|e| e.to_string())?;
    let snap = engine.snapshot();
    let usage = load_usage();
    let nq = normalize_query(q);
    let pick = usage.query_picks.get(&nq).copied();
    println!("query={q:?} normalized={nq:?}");
    for &i in engine.search(&snap, q).iter().take(10) {
        let a = &snap.apps[i];
        let mut sc = score(a, &nq);
        if pick == Some(a.id) {
            sc += 1000;
        }
        println!("{sc:>5} | {} | {:016x}", a.name, a.id);
    }
    engine.shutdown();
    Ok(())
}

fn parse_id(s: Option<&String>) -> Result<u64, String> {
    let s = s.ok_or("缺少 id（16 位十六进制，见 probe apps）")?;
    u64::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|e| format!("id 解析失败: {e}"))
}

/// probe pick <q> <id>：只写 usage.json，不启动。
fn cmd_pick(q: Option<&String>, id: Option<&String>) -> Result<(), String> {
    let q = q.ok_or("缺少查询串")?;
    let id = parse_id(id)?;
    let mut usage = load_usage();
    let nq = deskpad_engine::search::normalize_query(q);
    usage.query_picks.insert(nq.clone(), id);
    let path = usage_path()?;
    let text = serde_json::to_string_pretty(&usage).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    println!("picked {nq:?} -> {id:016x}");
    Ok(())
}

fn cmd_launch(id: Option<&String>) -> Result<(), String> {
    let id = parse_id(id)?;
    let engine = EngineHandle::init().map_err(|e| e.to_string())?;
    let r = engine.launch_app(id, "").map_err(|e| format!("{e:?}"));
    engine.shutdown();
    r?;
    println!("launched {id:016x}");
    Ok(())
}

fn cmd_open(path: Option<&String>) -> Result<(), String> {
    let path = path.ok_or("缺少路径")?;
    let engine = EngineHandle::init().map_err(|e| e.to_string())?;
    let r = engine.open_path(std::path::Path::new(path)).map_err(|e| format!("{e:?}"));
    engine.shutdown();
    r?;
    println!("opened {path}");
    Ok(())
}

/// probe watch [秒数=60]：订阅 on_change，每次触发打印 generation 和三个列表长度。
fn cmd_watch(secs: Option<&String>) -> Result<(), String> {
    let secs: u64 = secs.and_then(|s| s.parse().ok()).unwrap_or(60);
    let engine = EngineHandle::init().map_err(|e| e.to_string())?;
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let tx = std::sync::Mutex::new(tx);
    engine.set_on_change(Box::new(move || {
        if let Ok(t) = tx.lock() {
            let _ = t.send(());
        }
    }));
    let start = std::time::Instant::now();
    let total = std::time::Duration::from_secs(secs);
    while start.elapsed() < total {
        if rx.recv_timeout(std::time::Duration::from_millis(200)).is_ok() {
            let s = engine.snapshot();
            println!(
                "[{:>6.1}s] generation={} apps={} recent={} dirs={}",
                start.elapsed().as_secs_f32(), s.generation, s.apps.len(), s.recent.len(), s.dirs.len()
            );
        }
    }
    engine.shutdown();
    Ok(())
}

fn private_mb() -> f64 {
    use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut pmc = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    // SAFETY: 结构体大小已填写，输出到栈变量。
    let ok = unsafe {
        K32GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc as *mut _ as *mut _, pmc.cb)
    };
    if ok.as_bool() { pmc.PrivateUsage as f64 / 1_048_576.0 } else { -1.0 }
}

/// probe refresh-loop <n>：连续 refresh n 次，每次等快照更新；每 10 次打印一次 PrivateUsage(MB)。
fn cmd_refresh_loop(n: Option<&String>) -> Result<(), String> {
    let n: u32 = n.and_then(|s| s.parse().ok()).unwrap_or(20);
    let engine = EngineHandle::init().map_err(|e| e.to_string())?;
    for i in 1..=n {
        let g0 = engine.snapshot().generation;
        engine.refresh();
        let start = std::time::Instant::now();
        while engine.snapshot().generation == g0 && start.elapsed() < std::time::Duration::from_secs(15) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if i % 10 == 0 || i == 1 {
            println!("refresh {i:>4}: PrivateUsage={:.2} MB", private_mb());
        }
    }
    engine.shutdown();
    Ok(())
}
