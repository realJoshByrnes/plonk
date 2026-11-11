//! Writable in-memory file system: create/open dispositions, directories, writes, truncate, rename,
//! delete (including delete-on-close), attribute queries.
use super::*;

const T0: u64 = 133_500_000_000_000_000;
const STATUS_COLLISION: u32 = 0xC000_0035;
const STATUS_NOT_A_DIR: u32 = 0xC000_0103;
const STATUS_FILE_IS_A_DIRECTORY: u32 = 0xC000_00BA;
const STATUS_PATH_NOT_FOUND: u32 = 0xC000_003A;

impl WineProc {
    pub(super) fn is_dir(&self, path: &str) -> bool {
        if path.is_empty() || path == "c:" || self.dirs.contains(path) { return true; }
        let pre = format!("{path}\\");
        self.files.keys().any(|k| k.starts_with(&pre)) || self.dirs.iter().any(|d| d.starts_with(&pre))
    }

    fn parent_exists(&self, path: &str) -> bool {
        match path.rsplit_once('\\') { Some((p, _)) => self.is_dir(p), None => true }
    }

    fn put_basic(&mut self, buf: u32, is_dir: bool, size: u32, network: bool) -> Res<()> {
        for i in 0..4 { self.w32(buf + i * 8, T0 as u32)?; self.w32(buf + i * 8 + 4, (T0 >> 32) as u32)?; }
        let attrs = if is_dir { 0x10 } else { 0x80 };
        if network {
            self.w32(buf + 32, size)?; self.w32(buf + 36, 0)?; // allocation
            self.w32(buf + 40, size)?; self.w32(buf + 44, 0)?; // end of file
            self.w32(buf + 48, attrs)?;
        } else {
            self.w32(buf + 32, attrs)?;
        }
        Ok(())
    }

    pub(super) fn vfs_call(&mut self, name: &str, a: &[u32]) -> Res<Option<u32>> {
        Ok(Some(match name {
            "NtCreateFile" | "NtOpenFile" => {
                let create = name == "NtCreateFile";
                let (hp, oa, iosb) = (a[0], a[2], a[3]);
                let (disp, opts) = if create { (a[7], a[8]) } else { (1, a[5]) };
                let path = self.oa_path(oa)?;
                self.log(format!("  (open {path} disp={disp} opts={opts:#x})"));
                let dir_exists = self.is_dir(&path);
                let file_exists = self.files.contains_key(&path);
                let mut info = 1u32; // FILE_OPENED
                if opts & 1 != 0 { // directory
                    if file_exists { return Ok(Some(STATUS_NOT_A_DIR)); }
                    match disp {
                        1 if !dir_exists => { if let Some(k) = self.remote_key(&path) { self.need_file = Some(k); return Ok(None); } return Ok(Some(STATUS_OBJECT_NAME_NOT_FOUND)); }
                        2 if dir_exists => return Ok(Some(STATUS_COLLISION)),
                        2 | 3 if !dir_exists => { if !self.parent_exists(&path) { return Ok(Some(STATUS_PATH_NOT_FOUND)); } self.dirs.insert(path.clone()); info = 2; }
                        _ => {}
                    }
                } else if dir_exists && !file_exists {
                    // opening a directory without FILE_DIRECTORY_FILE is allowed for queries
                } else {
                    match disp {
                        1 | 4 if !file_exists => {
                            if let Some(k) = self.remote_key(&path) { self.need_file = Some(k); return Ok(None); }
                            self.w32(iosb, STATUS_OBJECT_NAME_NOT_FOUND)?;
                            return Ok(Some(if self.parent_exists(&path) { STATUS_OBJECT_NAME_NOT_FOUND } else { STATUS_PATH_NOT_FOUND }));
                        }
                        2 if file_exists => return Ok(Some(STATUS_COLLISION)),
                        0 | 5 | 4 if file_exists => { self.files.insert(path.clone(), Vec::new()); info = if disp == 0 { 0 } else { 3 }; }
                        0 | 2 | 3 | 5 if !file_exists => {
                            if !self.parent_exists(&path) { return Ok(Some(STATUS_PATH_NOT_FOUND)); }
                            self.files.insert(path.clone(), Vec::new());
                            info = 2;
                        }
                        _ => {}
                    }
                }
                let is_dir = self.is_dir(&path) && !self.files.contains_key(&path);
                let h = self.new_handle(Obj::File { path, pos: 0, dir: is_dir });
                if opts & 0x1000 != 0 { self.del_on_close.insert(h); } // FILE_DELETE_ON_CLOSE
                self.w32(hp, h)?;
                self.w32(iosb, 0)?;
                self.w32(iosb + 4, info)?;
                0
            }
            "NtWriteFile" => {
                let (h, iosb, buf, len, poff) = (a[0], a[4], a[5], a[6], a[7]);
                let (path, pos) = match self.handles.get(&h) { Some(Obj::File { path, pos, dir: false }) => (path.clone(), *pos), Some(Obj::File { .. }) => return Ok(Some(STATUS_FILE_IS_A_DIRECTORY)), _ => return Ok(None) };
                let off = if poff != 0 { self.r32(poff)? } else { pos } as usize;
                let data = self.mem.read_bytes(buf, len).map_err(|_| "write buf")?.to_vec();
                let f = self.files.entry(path).or_default();
                if f.len() < off + data.len() { f.resize(off + data.len(), 0); }
                f[off..off + data.len()].copy_from_slice(&data);
                if let Some(Obj::File { pos, .. }) = self.handles.get_mut(&h) { *pos = (off + data.len()) as u32; }
                self.w32(iosb, 0)?; self.w32(iosb + 4, len)?;
                0
            }
            "NtSetInformationFile" => {
                // (handle, iosb, buf, len, class)
                let (h, iosb, buf, class) = (a[0], a[1], a[2], a[4]);
                let (path, _) = match self.handles.get(&h) { Some(Obj::File { path, pos, .. }) => (path.clone(), *pos), _ => return Ok(Some(STATUS_INVALID_HANDLE)) };
                match class {
                    4 | 22 | 12 => {} // basic info (times/attributes), pipe info: ignored
                    13 | 64 => { if self.mem.r8(buf).unwrap_or(0) & 1 != 0 { self.del_on_close.insert(h); } else { self.del_on_close.remove(&h); } }
                    14 => { let p = self.r32(buf)?; self.log(format!("  (seek {path} -> {p})")); if let Some(Obj::File { pos, .. }) = self.handles.get_mut(&h) { *pos = p; } }
                    19 | 20 => { // allocation / end-of-file: truncate or extend
                        let n = self.r32(buf)? as usize;
                        if let Some(f) = self.files.get_mut(&path) { f.resize(n, 0); }
                    }
                    10 | 65 | 72 => { // rename: FILE_RENAME_INFORMATION { flags(4), root(4), len(4), name[] } (flags at offset 0 for class 10)
                        let (root, len) = (self.r32(buf + 4)?, self.r32(buf + 8)?);
                        let mut v = Vec::new(); for i in 0..len / 2 { v.push(self.mem.r16(buf + 12 + i * 2).unwrap_or(0)); }
                        let mut dest = Self::nt_path(&String::from_utf16_lossy(&v));
                        if root != 0 { if let Some(Obj::File { path: rp, .. }) = self.handles.get(&root) { dest = format!("{rp}\\{dest}"); } }
                        if let Some(d) = self.files.remove(&path) { self.files.insert(dest.clone(), d); } else if self.is_dir(&path) {
                            let pre = format!("{path}\\");
                            let moved: Vec<String> = self.files.keys().filter(|k| k.starts_with(&pre)).cloned().collect();
                            for k in moved { if let Some(d) = self.files.remove(&k) { self.files.insert(format!("{dest}\\{}", &k[pre.len()..]), d); } }
                            self.dirs.remove(&path); self.dirs.insert(dest.clone());
                        }
                        if let Some(Obj::File { path: hp, .. }) = self.handles.get_mut(&h) { *hp = dest; }
                    }
                    _ => { self.note(format!("SetInformationFile class {class}")); return Ok(Some(0xC000_0003)); }
                }
                self.w32(iosb, 0)?; self.w32(iosb + 4, 0)?;
                0
            }
            "NtQueryInformationFile" if matches!(a[4], 4 | 9) => {
                let (h, iosb, buf, len, class) = (a[0], a[1], a[2], a[3], a[4]);
                let (path, is_dir) = match self.handles.get(&h) { Some(Obj::File { path, dir, .. }) => (path.clone(), *dir), _ => return Ok(Some(STATUS_INVALID_HANDLE)) };
                let size = self.files.get(&path).map_or(0, |f| f.len() as u32);
                if class == 4 { self.put_basic(buf, is_dir, size, false)?; self.w32(iosb, 0)?; self.w32(iosb + 4, 40)?; }
                else { // FileNameInformation: length + path without drive
                    let p = path.strip_prefix("c:").unwrap_or(&path).to_string();
                    let u: Vec<u16> = p.encode_utf16().collect();
                    if len < 4 + u.len() as u32 * 2 { return Ok(Some(0x8000_0005)); }
                    self.w32(buf, u.len() as u32 * 2)?;
                    for (i, c) in u.iter().enumerate() { self.w16(buf + 4 + i as u32 * 2, *c)?; }
                    self.w32(iosb, 0)?; self.w32(iosb + 4, 4 + u.len() as u32 * 2)?;
                }
                0
            }
            "NtQueryAttributesFile" | "NtQueryFullAttributesFile" => {
                let path = self.oa_path(a[0])?;
                let (is_dir, exists) = (self.is_dir(&path) && !self.files.contains_key(&path), self.files.contains_key(&path) || self.is_dir(&path));
                if !exists {
                    if let Some(k) = self.remote_key(&path) { self.need_file = Some(k); return Ok(None); }
                    return Ok(Some(STATUS_OBJECT_NAME_NOT_FOUND));
                }
                let size = self.files.get(&path).map_or(0, |f| f.len() as u32);
                self.put_basic(a[1], is_dir, size, name == "NtQueryFullAttributesFile")?;
                0
            }
            "NtDeleteFile" => {
                let path = self.oa_path(a[0])?;
                if self.files.remove(&path).is_some() { 0 } else if self.dirs.remove(&path) { 0 } else { STATUS_OBJECT_NAME_NOT_FOUND }
            }
            "NtFlushBuffersFile" | "NtLockFile" | "NtUnlockFile" => { if a.len() > 1 && name == "NtFlushBuffersFile" { self.w32(a[1], 0)?; self.w32(a[1] + 4, 0)?; } 0 }
            "NtClose" => {
                if self.del_on_close.remove(&a[0]) {
                    if let Some(Obj::File { path, .. }) = self.handles.get(&a[0]) { let p = path.clone(); self.files.remove(&p); self.dirs.remove(&p); }
                }
                return Ok(None); // not handled here: the generic NtClose releases the handle
            }
            _ => return Ok(None),
        }))
    }
}
