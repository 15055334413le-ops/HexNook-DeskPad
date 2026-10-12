//! 系统缩略图拿不到时的内置兜底（engine 规范 4.x 缩略图）：
//! - PPTX / PPSX / POTX：读 Office Open XML 包里 PowerPoint / WPS 保存时写入的 `docProps/thumbnail.jpeg`（首张幻灯片封面），
//!   只按中央目录定位并读这一项，不把整个文件读进内存。
//! - PDF：用 Windows 自带的 `Windows.Data.Pdf` 渲染第一页。为了不让 WinRT / PDF 渲染器的 DLL 和堆常驻 DeskPad 主进程，
//!   这一步放在一次性子进程 `DeskPad.exe --thumb-pdf <in> <out> ...` 里做，渲染完立刻退出。
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// PDF 子进程整体超时：超过就杀掉，当次没有缩略图。
const PDF_WORKER_TIMEOUT: Duration = Duration::from_secs(12);
/// 中央目录 / 单项的大小上限，防止畸形文件让我们分配大块内存。
const ZIP_CD_MAX: u64 = 4 * 1024 * 1024;
const ZIP_ENTRY_MAX: u64 = 4 * 1024 * 1024;

fn u16le(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}
fn u32le(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// 从 zip 包里读出名为 `name` 的一项（仅支持 stored / deflate，不支持 zip64，Office 的缩略图都满足）。
pub(crate) fn zip_entry(path: &Path, name: &str) -> Option<Vec<u8>> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    // EOCD 在文件末尾 22 + 注释（≤64K）字节内
    let tail = len.min(22 + 65_535);
    f.seek(SeekFrom::Start(len - tail)).ok()?;
    let mut t = vec![0u8; tail as usize];
    f.read_exact(&mut t).ok()?;
    let eocd = (0..t.len().saturating_sub(21)).rev().find(|&i| u32le(&t, i) == Some(0x0605_4b50))?;
    let cd_size = u32le(&t, eocd + 12)? as u64;
    let cd_off = u32le(&t, eocd + 16)? as u64;
    if cd_size > ZIP_CD_MAX || cd_off + cd_size > len {
        return None;
    }
    f.seek(SeekFrom::Start(cd_off)).ok()?;
    let mut cd = vec![0u8; cd_size as usize];
    f.read_exact(&mut cd).ok()?;
    let mut i = 0usize;
    while u32le(&cd, i) == Some(0x0201_4b50) {
        let method = u16le(&cd, i + 10)?;
        let csize = u32le(&cd, i + 20)? as u64;
        let usize_ = u32le(&cd, i + 24)? as u64;
        let nlen = u16le(&cd, i + 28)? as usize;
        let xlen = u16le(&cd, i + 30)? as usize;
        let clen = u16le(&cd, i + 32)? as usize;
        let loff = u32le(&cd, i + 42)? as u64;
        let ename = cd.get(i + 46..i + 46 + nlen)?;
        if ename.eq_ignore_ascii_case(name.as_bytes()) {
            if csize > ZIP_ENTRY_MAX || usize_ > ZIP_ENTRY_MAX {
                return None;
            }
            let mut lh = [0u8; 30];
            f.seek(SeekFrom::Start(loff)).ok()?;
            f.read_exact(&mut lh).ok()?;
            if u32le(&lh, 0) != Some(0x0403_4b50) {
                return None;
            }
            let skip = 30 + u16le(&lh, 26)? as u64 + u16le(&lh, 28)? as u64;
            f.seek(SeekFrom::Start(loff + skip)).ok()?;
            let mut data = vec![0u8; csize as usize];
            f.read_exact(&mut data).ok()?;
            return match method {
                0 => Some(data),
                8 => miniz_oxide::inflate::decompress_to_vec_with_limit(&data, usize_ as usize).ok(),
                _ => None,
            };
        }
        i += 46 + nlen + xlen + clen;
    }
    None
}

/// PPTX 等的内嵌封面（JPEG / PNG 原始字节），没有则 None。
pub(crate) fn ooxml_thumbnail(path: &Path) -> Option<Vec<u8>> {
    zip_entry(path, "docProps/thumbnail.jpeg").or_else(|| zip_entry(path, "docProps/thumbnail.png"))
}

/// 一次性子进程批量渲染 PDF 首页：jobs = (输入 PDF, 输出 PNG)。返回后检查输出文件是否存在即可。
pub(crate) fn render_pdfs(jobs: &[(PathBuf, PathBuf)]) {
    if jobs.is_empty() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let mut cmd = Command::new(exe);
    cmd.arg("--thumb-pdf");
    for (i, o) in jobs {
        cmd.arg(i).arg(o);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let Ok(mut child) = cmd.spawn() else { return };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if start.elapsed() < PDF_WORKER_TIMEOUT => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                crate::log::warn("thumb-pdf worker timeout, killed");
                return;
            }
        }
    }
}

/// 子进程入口：`--thumb-pdf <in> <out> [<in> <out> ...]`。在 MTA 里用 Windows.Data.Pdf 渲染第一页为 PNG，
/// 长边 = `max_side`，白底。由 app 的 main 在一切初始化之前调用，返回进程退出码。
pub fn pdf_worker_main(args: &[String], max_side: u32) -> i32 {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    // SAFETY: 本子进程主线程首次初始化 COM（MTA，允许阻塞等待 WinRT 异步）。
    if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_err() {
        return 2;
    }
    let mut ok = 0;
    for pair in args.chunks(2) {
        if let [i, o] = pair {
            if render_pdf_page(Path::new(i), Path::new(o), max_side).is_ok() {
                ok += 1;
            }
        }
    }
    if ok > 0 { 0 } else { 1 }
}

fn render_pdf_page(input: &Path, out: &Path, max_side: u32) -> windows::core::Result<()> {
    use windows::core::HSTRING;
    use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
    use windows::Storage::StorageFile;
    use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(input.as_os_str()))?.get()?;
    let doc = PdfDocument::LoadFromFileAsync(&file)?.get()?;
    let page = doc.GetPage(0)?;
    let sz = page.Size()?;
    let s = max_side as f32 / sz.Width.max(sz.Height).max(1.0);
    let opts = PdfPageRenderOptions::new()?;
    opts.SetDestinationWidth(((sz.Width * s).round() as u32).max(1))?;
    opts.SetDestinationHeight(((sz.Height * s).round() as u32).max(1))?;
    // 默认编码器就是 PNG，背景默认白色
    let stream = InMemoryRandomAccessStream::new()?;
    page.RenderWithOptionsToStreamAsync(&stream, &opts)?.get()?;
    let n = stream.Size()? as u32;
    let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
    reader.LoadAsync(n)?.get()?;
    let mut buf = vec![0u8; n as usize];
    reader.ReadBytes(&mut buf)?;
    std::fs::write(out, buf).map_err(|_| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 手工拼一个两项的 zip（一项 stored、一项 deflate），验证按中央目录定位读取。
    fn build_zip(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let (mut out, mut cd) = (Vec::new(), Vec::new());
        for (name, data, deflate) in entries {
            let comp = if *deflate { miniz_oxide::deflate::compress_to_vec(data, 6) } else { data.to_vec() };
            let off = out.len() as u32;
            let method: u16 = if *deflate { 8 } else { 0 };
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0]);
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0; 8]); // time, date, crc
            out.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&3u16.to_le_bytes()); // extra len
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&[0, 0, 0]);
            out.extend_from_slice(&comp);
            cd.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            cd.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            cd.extend_from_slice(&method.to_le_bytes());
            cd.extend_from_slice(&[0; 8]);
            cd.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            cd.extend_from_slice(&(data.len() as u32).to_le_bytes());
            cd.extend_from_slice(&(name.len() as u16).to_le_bytes());
            cd.extend_from_slice(&[0; 12]); // extra, comment, disk, int attr, ext attr(4)
            cd.extend_from_slice(&off.to_le_bytes());
            cd.extend_from_slice(name.as_bytes());
        }
        let cd_off = out.len() as u32;
        out.extend_from_slice(&cd);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(cd.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_off.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    #[test]
    fn reads_stored_and_deflated_entries() {
        let big = vec![b'x'; 5000];
        let z = build_zip(&[("ppt/slides/slide1.xml", b"<a/>", false), ("docProps/thumbnail.jpeg", &big, true)]);
        let p = std::env::temp_dir().join(format!("deskpad-zip-test-{}.pptx", std::process::id()));
        std::fs::write(&p, z).unwrap();
        assert_eq!(zip_entry(&p, "ppt/slides/slide1.xml").as_deref(), Some(&b"<a/>"[..]));
        assert_eq!(ooxml_thumbnail(&p), Some(big));
        assert_eq!(zip_entry(&p, "nope"), None);
        let _ = std::fs::remove_file(&p);
    }
}
