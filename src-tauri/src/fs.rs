// File transfer (server / controlled side).
//
// Mirrors the relevant subset of hbb_common::fs so this QuickSupport client is
// wire-compatible with the official RustDesk controller:
//   * ReadDir / AllFiles   -> directory browsing the controller drives.
//   * FileAction::Send     -> controller wants to download; we READ + send blocks.
//   * FileAction::Receive  -> controller wants to upload;   we WRITE incoming blocks.
//
// Block compression uses zstd (same codec/level as upstream). Path-traversal is
// rejected on every entry name so a controller can never escape the base dir.
use crate::proto_gen::message::{
    file_transfer_send_confirm_request as ftsc, FileAction, FileDirectory, FileEntry, FileResponse,
    FileTransferBlock, FileTransferDigest, FileTransferDone, FileTransferError,
    FileTransferSendConfirmRequest, FileType, Message,
};
use anyhow::{anyhow, bail, Result};
use log::{info, warn};
use protobuf::EnumOrUnknown;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

const COMPRESS_LEVEL: i32 = 3;
const READ_BUF_SIZE: usize = 128 * 1024;

// ---------------------------------------------------------------------------
// compression
// ---------------------------------------------------------------------------

pub fn compress(data: &[u8]) -> Vec<u8> {
    match zstd::encode_all(data, COMPRESS_LEVEL) {
        Ok(v) => v,
        Err(e) => {
            warn!("compress failed: {e}");
            Vec::new()
        }
    }
}

pub fn decompress(data: &[u8]) -> Vec<u8> {
    zstd::decode_all(data).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// path helpers / validation
// ---------------------------------------------------------------------------

#[inline]
pub fn get_string(p: &Path) -> String {
    p.to_str().unwrap_or("").to_owned()
}

#[inline]
pub fn get_path(s: &str) -> PathBuf {
    Path::new(s).to_path_buf()
}

pub fn validate_file_name_no_traversal(name: &str) -> Result<()> {
    if name.bytes().any(|b| b == 0) {
        bail!("file name contains null bytes");
    }
    let has_traversal = name
        .split(|c: char| c == '/' || (cfg!(windows) && c == '\\'))
        .filter(|s| !s.is_empty())
        .any(|s| s == "..");
    if has_traversal {
        bail!("path traversal detected in file name");
    }
    #[cfg(windows)]
    {
        if name.len() >= 2 {
            let b = name.as_bytes();
            if b[0].is_ascii_alphabetic() && b[1] == b':' {
                bail!("absolute path detected in file name");
            }
        }
        if name.starts_with('/') || name.starts_with('\\') {
            bail!("absolute path detected in file name");
        }
    }
    #[cfg(not(windows))]
    if name.starts_with('/') {
        bail!("absolute path detected in file name");
    }
    Ok(())
}

fn join_validated(base: &Path, name: &str) -> Result<PathBuf> {
    validate_file_name_no_traversal(name)?;
    Ok(if name.is_empty() {
        base.to_path_buf()
    } else {
        base.join(name)
    })
}

// ---------------------------------------------------------------------------
// directory listing
// ---------------------------------------------------------------------------

pub fn read_dir(path: &Path, include_hidden: bool) -> Result<FileDirectory> {
    let mut dir = FileDirectory {
        path: get_string(path),
        ..Default::default()
    };
    #[cfg(windows)]
    if path.to_str() == Some("/") || path.to_str() == Some("\\") {
        // Report drive letters for the virtual root, like upstream.
        let drives = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
        for i in 0..32u32 {
            if drives & (1 << i) != 0 {
                let letter = char::from_u32(b'A' as u32 + i).unwrap_or('A');
                dir.entries.push(FileEntry {
                    name: format!("{letter}:"),
                    entry_type: EnumOrUnknown::new(FileType::DirDrive),
                    ..Default::default()
                });
            }
        }
        return Ok(dir);
    }

    for entry in std::fs::read_dir(path)?.flatten() {
        let p = entry.path();
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_owned();
        if name.is_empty() {
            continue;
        }
        let meta = match std::fs::symlink_metadata(&p) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mut is_hidden = false;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x2 != 0 {
                is_hidden = true;
            }
        }
        #[cfg(not(windows))]
        if name.as_bytes()[0] == b'.' {
            is_hidden = true;
        }
        if is_hidden && !include_hidden {
            continue;
        }
        let (entry_type, size) = if p.is_dir() {
            if meta.file_type().is_symlink() {
                (FileType::DirLink, 0)
            } else {
                (FileType::Dir, 0)
            }
        } else if meta.file_type().is_symlink() {
            (FileType::FileLink, 0)
        } else {
            (FileType::File, meta.len())
        };
        let modified_time = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        dir.entries.push(FileEntry {
            name,
            entry_type: EnumOrUnknown::new(entry_type),
            is_hidden,
            size,
            modified_time,
            ..Default::default()
        });
    }
    Ok(dir)
}

fn read_dir_recursive(path: &Path, prefix: &Path, include_hidden: bool) -> Result<Vec<FileEntry>> {
    let mut out = Vec::new();
    if path.is_dir() {
        let fd = read_dir(path, include_hidden)?;
        for entry in fd.entries.iter() {
            match entry.entry_type.enum_value() {
                Ok(FileType::File) => {
                    let mut e = entry.clone();
                    e.name = get_string(&prefix.join(&e.name));
                    out.push(e);
                }
                Ok(FileType::Dir) => {
                    if let Ok(mut sub) = read_dir_recursive(
                        &path.join(&entry.name),
                        &prefix.join(&entry.name),
                        include_hidden,
                    ) {
                        out.append(&mut sub);
                    }
                }
                _ => {}
            }
        }
        Ok(out)
    } else if path.is_file() {
        let (size, modified_time) = std::fs::metadata(path)
            .ok()
            .map(|m| {
                let mt = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                (m.len(), mt)
            })
            .unwrap_or((0, 0));
        out.push(FileEntry {
            entry_type: EnumOrUnknown::new(FileType::File),
            size,
            modified_time,
            ..Default::default()
        });
        Ok(out)
    } else {
        bail!("Not exists");
    }
}

pub fn get_recursive_files(path: &str, include_hidden: bool) -> Result<Vec<FileEntry>> {
    read_dir_recursive(&get_path(path), &get_path(""), include_hidden)
}

// ---------------------------------------------------------------------------
// filesystem mutations (remove / create / rename) — controller-driven, like
// hbb_common::fs. Paths are controller-supplied absolute paths; we validate
// against null bytes / emptiness and, for the new name in rename, traversal.
// ---------------------------------------------------------------------------

fn validate_fs_path_argument(path: &str, arg_name: &str) -> Result<()> {
    if path.is_empty() {
        bail!("{arg_name} cannot be empty");
    }
    if path.bytes().any(|b| b == 0) {
        bail!("{arg_name} contains null bytes");
    }
    Ok(())
}

pub fn remove_file(path: &str) -> Result<()> {
    validate_fs_path_argument(path, "file path")?;
    std::fs::remove_file(get_path(path))?;
    Ok(())
}

pub fn create_dir(path: &str) -> Result<()> {
    validate_fs_path_argument(path, "directory path")?;
    std::fs::create_dir_all(get_path(path))?;
    Ok(())
}

pub fn rename_file(path: &str, new_name: &str) -> Result<()> {
    validate_fs_path_argument(path, "path")?;
    if new_name.is_empty() {
        bail!("new file name cannot be empty");
    }
    validate_file_name_no_traversal(new_name)?;
    let src = std::path::Path::new(path);
    if !src.exists() {
        bail!("{path:?} not exists");
    }
    let parent = src
        .parent()
        .ok_or_else(|| anyhow!("parent directory of {path:?} not found"))?;
    std::fs::rename(src, parent.join(new_name))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// transfer job
// ---------------------------------------------------------------------------

/// A single inbound (write) or outbound (read) file-transfer job.
pub struct TransferJob {
    pub id: i32,
    /// true: we read local files and send blocks to the peer (download);
    /// false: we receive blocks from the peer and write them locally (upload).
    pub is_read: bool,
    base: PathBuf,
    files: Vec<FileEntry>,
    pub file_num: i32,
    pub total_size: u64,
    pub finished_size: u64,
    pub transferred: u64,

    enable_overwrite_detection: bool,

    // read-side (we send) state
    read_stream: Option<File>,
    file_confirmed: bool,
    file_is_waiting: bool,
    done: bool,

    // write-side (we receive) state
    write_stream: Option<File>,

    /// A produced-but-unsent outbound message (re-buffered when the bounded
    /// writer channel was full). Flushed before producing anything new.
    pub(crate) pending: Option<Message>,
}

impl TransferJob {
    pub fn new_read(
        id: i32,
        remote: String,
        base: PathBuf,
        files: Vec<FileEntry>,
        enable_overwrite_detection: bool,
    ) -> Self {
        let total_size = files.iter().map(|f| f.size).sum();
        info!(
            "new read job {id}: {} file(s), {total_size} bytes, remote={remote}, base={}",
            files.len(),
            base.display()
        );
        Self {
            id,
            is_read: true,
            base,
            files,
            file_num: 0,
            total_size,
            enable_overwrite_detection,
            ..Self::default_state()
        }
    }

    pub fn new_write(
        id: i32,
        remote: String,
        base: PathBuf,
        files: Vec<FileEntry>,
        enable_overwrite_detection: bool,
    ) -> Result<Self> {
        // Validate every entry name up-front so we never write outside `base`.
        for f in &files {
            validate_file_name_no_traversal(&f.name)?;
        }
        let total_size = files.iter().map(|f| f.size).sum();
        info!(
            "new write job {id}: {} file(s), {total_size} bytes, remote={remote}, base={}",
            files.len(),
            base.display()
        );
        Ok(Self {
            id,
            is_read: false,
            base,
            files,
            file_num: 0,
            total_size,
            enable_overwrite_detection,
            ..Self::default_state()
        })
    }

    const fn default_state() -> Self {
        Self {
            id: 0,
            is_read: false,
            base: PathBuf::new(),
            files: Vec::new(),
            file_num: 0,
            total_size: 0,
            finished_size: 0,
            transferred: 0,
            enable_overwrite_detection: false,
            read_stream: None,
            file_confirmed: false,
            file_is_waiting: false,
            done: false,
            write_stream: None,
            pending: None,
        }
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Current file's display name (for UI feedback).
    pub fn current_name(&self) -> String {
        self.files
            .get(self.file_num as usize)
            .map(|f| f.name.clone())
            .unwrap_or_default()
    }

    // ----- read side (we send blocks to the peer) -----

    /// Resolve the absolute path of the file currently being read.
    fn read_target(&self, idx: usize) -> Option<PathBuf> {
        let entry = self.files.get(idx)?;
        // For single-file transfers upstream allows an empty name (the path is
        // carried by `base` itself).
        if self.files.len() == 1 && entry.name.is_empty() {
            Some(self.base.clone())
        } else {
            join_validated(&self.base, &entry.name).ok()
        }
    }

    async fn open_read_stream(&mut self) -> Result<bool> {
        let idx = self.file_num as usize;
        if idx >= self.files.len() {
            self.read_stream.take();
            return Ok(true); // job finished
        }
        if self.read_stream.is_none() {
            let path = self
                .read_target(idx)
                .ok_or_else(|| anyhow!("invalid file name in job {}", self.id))?;
            match File::open(&path).await {
                Ok(f) => {
                    self.read_stream = Some(f);
                    self.file_confirmed = false;
                    self.file_is_waiting = false;
                }
                Err(e) => {
                    // Skip unreadable files: advance and surface the error.
                    self.file_num += 1;
                    self.file_confirmed = false;
                    self.file_is_waiting = false;
                    return Err(e.into());
                }
            }
        }
        Ok(false)
    }

    async fn current_digest(&self) -> Result<(u64, u64)> {
        let f = self.read_stream.as_ref().ok_or_else(|| anyhow!("no stream"))?;
        let meta = f.metadata().await?;
        let mtime = meta
            .modified()?
            .duration_since(UNIX_EPOCH)?
            .as_secs();
        Ok((mtime, meta.len()))
    }

    /// Produce the next outbound message for a read job (digest / block / done),
    /// or `None` when waiting for the peer / nothing more to do.
    pub async fn next_read_outbound(&mut self) -> Result<Option<Message>> {
        // Flush any re-buffered block first so we never drop or reorder data.
        if let Some(m) = self.pending.take() {
            return Ok(Some(m));
        }
        if self.done {
            return Ok(None);
        }

        // Need a new file open?
        let finished = self.open_read_stream().await?;
        if finished {
            self.done = true;
            return Ok(Some(new_done_msg(self.id, self.file_num)));
        }

        // Overwrite-detection handshake for the current file.
        if self.enable_overwrite_detection
            && !self.file_confirmed
            && !self.file_is_waiting
        {
            if let Ok((mtime, size)) = self.current_digest().await {
                self.file_is_waiting = true;
                return Ok(Some(new_digest_msg(self.id, self.file_num, mtime, size)));
            }
        }
        if self.file_is_waiting {
            return Ok(None);
        }

        // Read one block.
        let idx = self.file_num as usize;
        let mut buf = vec![0u8; READ_BUF_SIZE];
        let mut total = 0usize;
        loop {
            let n = self
                .read_stream
                .as_mut()
                .ok_or_else(|| anyhow!("stream gone"))?
                .read(&mut buf[total..])
                .await?;
            if n == 0 {
                break;
            }
            total += n;
            if total == READ_BUF_SIZE {
                break;
            }
        }

        if total == 0 {
            // EOF of current file: advance and let the next tick drive progress.
            self.file_num += 1;
            self.read_stream = None;
            self.file_confirmed = false;
            self.file_is_waiting = false;
            return Ok(None);
        }

        self.finished_size += total as u64;
        let name = self
            .files
            .get(idx)
            .map(|f| f.name.as_str())
            .unwrap_or("");
        let (data, compressed) = if !is_compressed_file(name) {
            let c = compress(&buf[..total]);
            if c.len() < total {
                (c, true)
            } else {
                (buf[..total].to_vec(), false)
            }
        } else {
            (buf[..total].to_vec(), false)
        };
        self.transferred += data.len() as u64;

        Ok(Some(new_block_msg(self.id, idx as i32, data, compressed)))
    }

    /// React to a SendConfirm from the peer (read jobs only).
    pub async fn on_send_confirm(&mut self, r: &FileTransferSendConfirmRequest) {
        if self.file_num != r.file_num {
            info!("job {}: confirm file_num {} != current {}, ignoring", self.id, r.file_num, self.file_num);
            return;
        }
        match &r.union {
            Some(ftsc::Union::Skip(s)) => {
                if *s {
                    self.skip_current_file().await;
                } else {
                    self.file_confirmed = true;
                    self.file_is_waiting = false;
                }
            }
            Some(ftsc::Union::OffsetBlk(off)) => {
                self.file_confirmed = true;
                self.file_is_waiting = false;
                if *off > 0 {
                    self.seek_read(*off as u64).await;
                }
            }
            None => {}
        }
    }

    async fn skip_current_file(&mut self) {
        self.read_stream.take();
        self.file_num += 1;
        self.file_confirmed = false;
        self.file_is_waiting = false;
    }

    async fn seek_read(&mut self, offset: u64) {
        if let Some(f) = self.read_stream.as_mut() {
            if f.seek(std::io::SeekFrom::Start(offset)).await.is_ok() {
                self.transferred += offset;
                self.finished_size += offset;
            }
        }
    }

    // ----- write side (we receive blocks from the peer) -----

    fn write_target(&self, idx: usize) -> Result<PathBuf> {
        let entry = self
            .files
            .get(idx)
            .ok_or_else(|| anyhow!("file_num {idx} out of range"))?;
        let final_path = join_validated(&self.base, &entry.name)?;
        Ok(final_path)
    }

    /// Finalize the file currently being written: rename `.download` -> final
    /// and restore its modification time.
    async fn finalize_current(&mut self) {
        if let Some(f) = self.write_stream.take() {
            let _ = f.sync_all().await;
        }
        let idx = self.file_num as usize;
        if let Ok(path) = self.write_target(idx) {
            let dl = download_path(&path);
            if Path::new(&dl).exists() {
                if let Err(e) = std::fs::rename(&dl, &path) {
                    // Renaming a still-open file can race on Windows; fall back
                    // to copy+remove so the file is always saved under its final
                    // name before we acknowledge completion to the sender.
                    if let Err(e2) = std::fs::copy(&dl, &path).and_then(|_| std::fs::remove_file(&dl)) {
                        warn!("save {}: rename failed ({e}), copy also failed ({e2})", path.display());
                    }
                }
                if let Some(entry) = self.files.get(idx) {
                    let _ = filetime::set_file_mtime(
                        &path,
                        filetime::FileTime::from_unix_time(entry.modified_time as i64, 0),
                    );
                }
            }
            let _ = std::fs::remove_file(format!("{}.digest", path.display()));
        }
    }

    /// Write an incoming block for an upload (write) job.
    pub async fn write_block(&mut self, block: &FileTransferBlock) -> Result<()> {
        if block.id != self.id {
            bail!("block id mismatch");
        }
        let incoming_num = block.file_num;
        if incoming_num != self.file_num || self.write_stream.is_none() {
            // Switching to a new file: finalize the previous one first.
            if self.write_stream.is_some() {
                self.finalize_current().await;
            }
            self.file_num = incoming_num;
            let path = self.write_target(incoming_num as usize)?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            let dl = download_path(&path);
            // Truncate any stale partial.
            let _ = std::fs::remove_file(&dl);
            self.write_stream = Some(
                OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&dl)
                    .await?,
            );
        }

        let data = if block.compressed {
            decompress(&block.data)
        } else {
            block.data.to_vec()
        };
        // Match hbb_common: write whatever data arrived (an empty block is the
        // sender's EOF marker and is a no-op here). Finalization happens only on
        // a file_num change or on `Done`, never on empty data — otherwise a
        // failed zstd decompress (which yields empty) would prematurely close
        // the file and the following blocks would truncate it.
        self.write_stream
            .as_mut()
            .ok_or_else(|| anyhow!("no write stream"))?
            .write_all(&data)
            .await?;
        self.finished_size += data.len() as u64;
        self.transferred += block.data.len() as u64;
        Ok(())
    }

    /// Respond to a digest received from the peer (upload jobs). We always
    /// accept the file from the start; this overwrites any existing copy, which
    /// is the expected QuickSupport behavior.
    pub fn build_confirm_for_digest(&self, d: &FileTransferDigest) -> Message {
        new_send_confirm_msg(d.id, d.file_num, false, 0)
    }

    /// Mark the whole job as finished (peer sent Done for an upload).
    pub async fn finish(&mut self) {
        self.finalize_current().await;
        self.done = true;
    }

    pub fn cancel(&mut self) {
        self.done = true;
        self.read_stream.take();
        self.write_stream.take();
    }
}

#[inline]
fn download_path(final_path: &Path) -> PathBuf {
    let mut s = final_path.as_os_str().to_os_string();
    s.push(".download");
    s.into()
}

#[inline]
fn is_compressed_file(name: &str) -> bool {
    let exts = ["xz", "gz", "zip", "7z", "rar", "bz2", "tgz", "png", "jpg", "jpeg"];
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    exts.contains(&ext.as_str())
}

// ---------------------------------------------------------------------------
// message constructors
// ---------------------------------------------------------------------------

pub fn new_dir_msg(id: i32, path: String, entries: Vec<FileEntry>) -> Message {
    let mut resp = FileResponse::new();
    resp.set_dir(FileDirectory {
        id,
        path,
        entries,
        ..Default::default()
    });
    let mut m = Message::new();
    m.set_file_response(resp);
    m
}

pub fn new_error_msg<T: ToString>(id: i32, err: T, file_num: i32) -> Message {
    let mut resp = FileResponse::new();
    resp.set_error(FileTransferError {
        id,
        error: err.to_string(),
        file_num,
        ..Default::default()
    });
    let mut m = Message::new();
    m.set_file_response(resp);
    m
}

pub fn new_block_msg(id: i32, file_num: i32, data: Vec<u8>, compressed: bool) -> Message {
    let mut resp = FileResponse::new();
    resp.set_block(FileTransferBlock {
        id,
        file_num,
        data: bytes::Bytes::from(data),
        compressed,
        ..Default::default()
    });
    let mut m = Message::new();
    m.set_file_response(resp);
    m
}

pub fn new_done_msg(id: i32, file_num: i32) -> Message {
    let mut resp = FileResponse::new();
    resp.set_done(FileTransferDone {
        id,
        file_num,
        ..Default::default()
    });
    let mut m = Message::new();
    m.set_file_response(resp);
    m
}

pub fn new_digest_msg(id: i32, file_num: i32, last_modified: u64, file_size: u64) -> Message {
    let mut resp = FileResponse::new();
    resp.set_digest(FileTransferDigest {
        id,
        file_num,
        last_modified,
        file_size,
        ..Default::default()
    });
    let mut m = Message::new();
    m.set_file_response(resp);
    m
}

pub fn new_send_confirm_msg(id: i32, file_num: i32, skip: bool, offset: u32) -> Message {
    let mut r = FileTransferSendConfirmRequest::new();
    r.id = id;
    r.file_num = file_num;
    r.union = Some(if skip {
        ftsc::Union::Skip(true)
    } else {
        ftsc::Union::OffsetBlk(offset)
    });
    let mut a = FileAction::new();
    a.set_send_confirm(r);
    let mut m = Message::new();
    m.set_file_action(a);
    m
}

pub fn remove_job(id: i32, jobs: &mut Vec<TransferJob>) -> Option<TransferJob> {
    jobs.iter()
        .position(|j| j.id == id)
        .map(|i| jobs.remove(i))
}
